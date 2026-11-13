//! `/api/users/me/*`: current user, profile, settings, onboarding. Also the `saturn seed` command.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use rand::Rng;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::{token_matches, valid_email};
use crate::common::*;
use crate::session;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/users/session/", get(session_info))
        .route("/api/users/me/accounts/", get(accounts_list))
        .route(
            "/api/users/me/accounts/{pk}/",
            get(account_retrieve).delete(account_delete),
        )
        .route("/api/users/me/email/generate-code/", post(generate_email_code))
        .route("/api/users/me/email/", patch(update_email))
        .route("/api/users/me/", get(me).patch(update_me).delete(deactivate))
        .route("/api/users/me/settings/", get(settings))
        .route("/api/users/me/profile/", get(profile).patch(update_profile))
        .route("/api/users/me/instance-admin/", get(instance_admin))
        .route("/api/users/me/onboard/", patch(onboard))
        .route("/api/users/me/tour-completed/", patch(tour_completed))
        .route("/api/users/api-tokens/", get(tokens_list).post(token_create))
        .route(
            "/api/users/api-tokens/{pk}/",
            get(token_retrieve).patch(token_update).delete(token_delete),
        )
}

async fn token_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(pk): Path<Uuid>,
    Json(b): Json<Value>,
) -> ApiResult {
    for key in ["label", "description"] {
        if b.get(key).is_some_and(|v| !v.is_string()) {
            return Err(ApiError::bad_body(json!({key: ["Not a valid string."]})));
        }
    }
    json_row(sqlx::query_scalar(&format!("UPDATE api_tokens AS t SET label=COALESCE($3,label), description=COALESCE($4,description), updated_at=now(), updated_by_id=$2 WHERE id=$1 AND user_id=$2 AND deleted_at IS NULL RETURNING {TOKEN_READ_JSON} || jsonb_build_object('token', t.token)"))
        .bind(pk).bind(user.id).bind(s(&b,"label")).bind(s(&b,"description")), &st.db).await
}

/// Plane `UserEndpoint.deactivate`: refuse for instance admins and for the sole admin of a project or
/// workspace that has other members; otherwise leave every membership, drop invites and sessions,
/// reset onboarding, scramble the password and deactivate. One transaction so a refusal changes nothing.
async fn deactivate(State(st): State<AppState>, user: CurrentUser) -> Result<Response, ApiError> {
    if crate::auth::is_instance_admin(&st, user.id).await? {
        return Err(ApiError::bad(
            "You cannot deactivate your account since you are an instance admin",
        ));
    }
    // A membership blocks deactivation when it has other members but no other active admin. Plane's
    // annotate counts only the user's own row (total_members is always 1), so its guard never fires and
    // a sole admin can orphan a workspace; Saturn enforces what the error message promises.
    let sole = |table: &str, scope: &str| {
        format!(
            "SELECT EXISTS (SELECT 1 FROM {table} m WHERE m.member_id = $1 AND m.is_active AND m.deleted_at IS NULL
               AND (SELECT count(*) FROM {table} o WHERE o.{scope} = m.{scope} AND o.deleted_at IS NULL) > 1
               AND NOT EXISTS (SELECT 1 FROM {table} o WHERE o.{scope} = m.{scope} AND o.role = 20 AND o.is_active
                 AND o.deleted_at IS NULL AND o.member_id <> $1))"
        )
    };
    let mut tx = st.db.begin().await?;
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    for (table, scope, what) in [
        ("project_members", "project_id", "projects"),
        ("workspace_members", "workspace_id", "workspaces"),
    ] {
        let blocked: bool = sqlx::query_scalar(&sole(table, scope))
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?;
        if blocked {
            return Err(ApiError::bad(format!(
                "You cannot deactivate account as you are the only admin in some {what}."
            )));
        }
    }
    sqlx::query("UPDATE project_members SET is_active = false WHERE member_id = $1 AND is_active")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE workspace_members SET is_active = false WHERE member_id = $1 AND is_active")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM workspace_member_invites WHERE email = $1")
        .bind(&user.email)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user.id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE profiles SET last_workspace_id = NULL, is_tour_completed = false, is_onboarded = false,
           onboarding_step = '{\"workspace_join\": false, \"profile_complete\": false, \"workspace_create\": false, \"workspace_invite\": false}'::jsonb
         WHERE user_id = $1",
    )
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE users SET is_password_autoset = true, password = $2, is_active = false, last_logout_time = now(),
           updated_at = now() WHERE id = $1",
    )
    .bind(user.id)
    .bind(session::hash_password(&Uuid::new_v4().simple().to_string()))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let text = format!(
        "Your account has been deactivated on Plane. {}/login",
        st.cfg.web_url.trim_end_matches('/')
    );
    crate::outbound::send_mail_bg(
        &st,
        user.email.clone(),
        format!("{} has been deactivated on Plane", user.email),
        ammonia::clean(&text),
        text,
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        session::set_cookie(&st, session::COOKIE, None, 0).parse().unwrap(),
    );
    Ok((StatusCode::NO_CONTENT, headers).into_response())
}

// --- API tokens (ApiTokenEndpoint). Token is returned once, on create. ---

/// APITokenReadSerializer fields (no `token`). `is_active` is computed from `expired_at`, as Plane does.
const TOKEN_READ_JSON: &str = "jsonb_build_object('id', t.id, 'label', t.label, 'description', t.description,
    'user', t.user_id, 'user_type', t.user_type, 'workspace', t.workspace_id, 'expired_at', t.expired_at,
    'last_used', t.last_used, 'created_at', t.created_at, 'updated_at', t.updated_at, 'deleted_at', t.deleted_at,
    'created_by', t.created_by_id, 'updated_by', t.updated_by_id,
    'is_active', (t.expired_at IS NULL OR t.expired_at > now()))";

async fn tokens_list(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {TOKEN_READ_JSON} FROM api_tokens t WHERE t.user_id = $1 AND t.deleted_at IS NULL ORDER BY t.created_at"
        ))
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn token_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let label = s(&b, "label")
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    let expired_at = match s(&b, "expired_at") {
        Some(raw) => Some(
            chrono::DateTime::parse_from_rfc3339(raw)
                .map_err(|_| ApiError::bad_body(json!({"expired_at": ["Datetime has wrong format."]})))?
                .with_timezone(&chrono::Utc),
        ),
        None => None,
    };
    // Plane: `"plane_api_" + uuid4().hex`. Token is stored in plaintext, as Plane does.
    let token = format!("plane_api_{}", Uuid::new_v4().simple());
    let mut v: Value = sqlx::query_scalar(&format!(
        "INSERT INTO api_tokens AS t (token, label, description, user_id, user_type, expired_at, is_active,
            created_by_id, updated_by_id)
         VALUES ($1, $2, COALESCE($3, ''), $4,
                 (SELECT CASE WHEN u.is_bot THEN 1 ELSE 0 END FROM users u WHERE u.id = $4), $5, true, $4, $4)
         RETURNING {TOKEN_READ_JSON}"
    ))
    .bind(&token)
    .bind(&label)
    .bind(s(&b, "description"))
    .bind(user.id)
    .bind(expired_at)
    .fetch_one(&st.db)
    .await?;
    v["token"] = json!(token);
    created(v)
}

async fn token_retrieve(State(st): State<AppState>, user: CurrentUser, Path(pk): Path<Uuid>) -> ApiResult {
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {TOKEN_READ_JSON} FROM api_tokens t WHERE t.id = $1 AND t.user_id = $2 AND t.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(user.id),
        &st.db,
    )
    .await
}

/// Soft delete. X-Api-Key auth in common.rs filters `deleted_at IS NULL`, so the key stops working at once.
async fn token_delete(State(st): State<AppState>, user: CurrentUser, Path(pk): Path<Uuid>) -> ApiResult<StatusCode> {
    let r = sqlx::query(
        "UPDATE api_tokens SET deleted_at = now(), updated_at = now(), updated_by_id = $2
         WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(user.id)
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

/// UserMeSerializer fields.
pub const USER_ME_JSON: &str = "jsonb_build_object('id', u.id, 'avatar', u.avatar, 'cover_image', u.cover_image,
    'avatar_url', CASE WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id || '/'
                       ELSE NULLIF(u.avatar, '') END,
    'cover_image_url', CASE WHEN u.cover_image_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.cover_image_asset_id || '/'
                            ELSE u.cover_image END,
    'date_joined', u.date_joined, 'display_name', u.display_name, 'email', u.email, 'first_name', u.first_name,
    'last_name', u.last_name, 'is_active', u.is_active, 'is_bot', u.is_bot, 'is_email_verified', u.is_email_verified,
    'user_timezone', u.user_timezone, 'username', u.username, 'is_password_autoset', u.is_password_autoset,
    'last_login_medium', u.last_login_medium, 'last_login_time', u.last_login_time)";

/// UserLiteSerializer fields, used for assignees, members, created_by etc. Alias the users table as `u`.
pub const USER_LITE_JSON: &str = "jsonb_build_object('id', u.id, 'first_name', u.first_name, 'last_name', u.last_name,
    'avatar', u.avatar, 'avatar_url', CASE WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id || '/'
                                           ELSE NULLIF(u.avatar, '') END,
    'is_bot', u.is_bot, 'display_name', u.display_name)";

async fn me(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    json_row(
        sqlx::query_scalar(&format!("SELECT {USER_ME_JSON} FROM users u WHERE u.id = $1")).bind(user.id),
        &st.db,
    )
    .await
}

pub async fn validate_timezone(db: &sqlx::PgPool, body: &Value, field: &str) -> Result<(), ApiError> {
    if let Some(value) = body.get(field) {
        let valid = if let Some(name) = value.as_str() {
            sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = $1)")
                .bind(name)
                .fetch_one(db)
                .await?
        } else {
            false
        };
        if !valid {
            return Err(ApiError::bad_body(json!({field: ["Enter a valid timezone."]})));
        }
    }
    Ok(())
}

/// `cover_image` like Plane's `URLField(max_length=800)`: an http(s) URL, or a Saturn asset path
/// (`/api/assets/...`, Plane #9283). Rejects `javascript:` and other schemes the web app would render.
fn valid_cover_image(v: &str) -> bool {
    if v.len() > 800 || v.chars().any(char::is_control) {
        return false;
    }
    if v.starts_with("/api/assets/") && !v.contains("..") {
        return true;
    }
    url::Url::parse(v).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
}

async fn update_me(State(st): State<AppState>, user: CurrentUser, Json(b): Json<Value>) -> ApiResult {
    validate_timezone(&st.db, &b, "user_timezone").await?;
    if let Some(dn) = s(&b, "display_name")
        && dn.trim().is_empty()
    {
        return Err(ApiError::bad_body(
            json!({"display_name": ["This field may not be blank."]}),
        ));
    }
    // cover_image: absent = keep, null or "" = clear (Plane's remove-cover payload), string = validated URL.
    let cover = b.get("cover_image");
    let clear_cover = matches!(cover, Some(Value::Null)) || cover.and_then(Value::as_str) == Some("");
    let new_cover = match cover {
        Some(Value::String(v)) if !v.is_empty() => {
            if !valid_cover_image(v) {
                return Err(ApiError::bad_body(json!({"cover_image": ["Enter a valid URL."]})));
            }
            Some(v.as_str())
        }
        None | Some(Value::Null) | Some(Value::String(_)) => None,
        Some(_) => return Err(ApiError::bad_body(json!({"cover_image": ["Enter a valid URL."]}))),
    };
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE users u SET first_name = COALESCE($2, first_name), last_name = COALESCE($3, last_name),
                display_name = COALESCE($4, display_name), user_timezone = COALESCE($5, user_timezone),
                avatar = COALESCE($6, avatar),
                cover_image = CASE WHEN $8 THEN NULL ELSE COALESCE($7, cover_image) END,
                cover_image_asset_id = CASE WHEN $8 THEN NULL ELSE cover_image_asset_id END,
                updated_at = now()
             WHERE u.id = $1 RETURNING {USER_ME_JSON}"
        ))
        .bind(user.id)
        .bind(s(&b, "first_name"))
        .bind(s(&b, "last_name"))
        .bind(s(&b, "display_name"))
        .bind(s(&b, "user_timezone"))
        .bind(s(&b, "avatar"))
        .bind(new_cover)
        .bind(clear_cover),
        &st.db,
    )
    .await
}

async fn settings(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    let last: Option<(Option<Uuid>,)> = sqlx::query_as("SELECT last_workspace_id FROM profiles WHERE user_id = $1")
        .bind(user.id)
        .fetch_optional(&st.db)
        .await?;
    let last = last.and_then(|(l,)| l);
    let invites: (i64,) =
        sqlx::query_as("SELECT count(*) FROM workspace_member_invites WHERE email = $1 AND deleted_at IS NULL")
            .bind(&user.email)
            .fetch_one(&st.db)
            .await?;
    let member_ws = "SELECT w.id, w.slug, w.name FROM workspaces w JOIN workspace_members m ON m.workspace_id = w.id
                     WHERE m.member_id = $1 AND m.is_active AND m.deleted_at IS NULL AND w.deleted_at IS NULL";
    let lastws: Option<(Uuid, String, String)> = match last {
        Some(id) => {
            sqlx::query_as(&format!("{member_ws} AND w.id = $2"))
                .bind(user.id)
                .bind(id)
                .fetch_optional(&st.db)
                .await?
        }
        None => None,
    };
    let workspace = if let Some((id, slug, name)) = lastws {
        json!({"last_workspace_id": id, "last_workspace_slug": slug, "last_workspace_name": name, "last_workspace_logo": "",
               "fallback_workspace_id": id, "fallback_workspace_slug": slug, "invites": invites.0})
    } else {
        let fb: Option<(Uuid, String, String)> = sqlx::query_as(&format!("{member_ws} ORDER BY w.created_at LIMIT 1"))
            .bind(user.id)
            .fetch_optional(&st.db)
            .await?;
        json!({"last_workspace_id": null, "last_workspace_slug": null,
               "fallback_workspace_id": fb.as_ref().map(|f| f.0), "fallback_workspace_slug": fb.as_ref().map(|f| f.1.clone()),
               "invites": invites.0})
    };
    Ok(Json(
        json!({"id": user.id, "email": user.email, "workspace": workspace}),
    ))
}

/// ProfileSerializer is `fields = "__all__"`: every column, with the FK as `user`.
const PROFILE_JSON: &str = "to_jsonb(p) - 'user_id' || jsonb_build_object('user', p.user_id)";

async fn ensure_profile(st: &AppState, user_id: Uuid) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO profiles (user_id) SELECT $1 WHERE NOT EXISTS (SELECT 1 FROM profiles WHERE user_id = $1)",
    )
    .bind(user_id)
    .execute(&st.db)
    .await?;
    Ok(())
}

async fn profile(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    ensure_profile(&st, user.id).await?;
    json_row(
        sqlx::query_scalar(&format!("SELECT {PROFILE_JSON} FROM profiles p WHERE p.user_id = $1")).bind(user.id),
        &st.db,
    )
    .await
}

/// Writable profile columns and their JSON types. Anything else in the body is ignored.
const PROFILE_FIELDS: &[(&str, &str)] = &[
    ("theme", "jsonb"),
    ("is_tour_completed", "bool"),
    ("onboarding_step", "jsonb"),
    ("use_case", "text"),
    ("role", "text"),
    ("is_onboarded", "bool"),
    ("last_workspace_id", "uuid"),
    ("billing_address_country", "text"),
    ("billing_address", "jsonb"),
    ("has_billing_address", "bool"),
    ("company_name", "text"),
    ("is_mobile_onboarded", "bool"),
    ("mobile_onboarding_step", "jsonb"),
    ("mobile_timezone_auto_set", "bool"),
    ("language", "text"),
    ("is_smooth_cursor_enabled", "bool"),
    ("start_of_the_week", "smallint"),
    ("is_app_rail_docked", "bool"),
    ("background_color", "text"),
    ("goals", "jsonb"),
    ("has_marketing_email_consent", "bool"),
    ("is_navigation_tour_completed", "bool"),
    ("is_subscribed_to_changelog", "bool"),
    ("notification_view_mode", "text"),
    ("product_tour", "jsonb"),
];

/// Build `col = ($n::jsonb #>> '{}')::type` assignments for whitelisted keys present in `body`.
/// Column names come from the fixed whitelist, values are bound as one jsonb parameter.
pub fn patch_sets(body: &Value, fields: &[(&str, &str)], param: usize) -> Vec<String> {
    fields
        .iter()
        .filter(|(k, _)| body.get(*k).is_some())
        .map(|(k, t)| match *t {
            "jsonb" => format!("{k} = ${param}::jsonb -> '{k}'"),
            t => format!("{k} = (${param}::jsonb ->> '{k}')::{t}"),
        })
        .collect()
}

async fn update_profile(State(st): State<AppState>, user: CurrentUser, Json(b): Json<Value>) -> ApiResult {
    validate_timezone(&st.db, &b, "user_timezone").await?;
    ensure_profile(&st, user.id).await?;
    let mut tx = st.db.begin().await?;
    if let Some(timezone) = s(&b, "user_timezone") {
        sqlx::query("UPDATE users SET user_timezone = $2, updated_at = now() WHERE id = $1")
            .bind(user.id)
            .bind(timezone)
            .execute(&mut *tx)
            .await?;
    }
    let mut sets = patch_sets(&b, PROFILE_FIELDS, 2);
    sets.push("updated_at = now()".into());
    let result: Value = sqlx::query_scalar(&format!(
        "UPDATE profiles p SET {} WHERE p.user_id = $1 RETURNING {PROFILE_JSON}",
        sets.join(", ")
    ))
    .bind(user.id)
    .bind(&b)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(result))
}

async fn instance_admin(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    let v: (bool,) =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM instance_admins WHERE user_id = $1 AND deleted_at IS NULL)")
            .bind(user.id)
            .fetch_one(&st.db)
            .await?;
    Ok(Json(json!({"is_instance_admin": v.0})))
}

async fn onboard(State(st): State<AppState>, user: CurrentUser, Json(b): Json<Value>) -> ApiResult {
    ensure_profile(&st, user.id).await?;
    sqlx::query("UPDATE profiles SET is_onboarded = $2, updated_at = now() WHERE user_id = $1")
        .bind(user.id)
        .bind(b.get("is_onboarded").and_then(Value::as_bool).unwrap_or(false))
        .execute(&st.db)
        .await?;
    Ok(Json(json!({"message": "Updated successfully"})))
}

async fn tour_completed(State(st): State<AppState>, user: CurrentUser, Json(b): Json<Value>) -> ApiResult {
    ensure_profile(&st, user.id).await?;
    sqlx::query("UPDATE profiles SET is_tour_completed = $2, updated_at = now() WHERE user_id = $1")
        .bind(user.id)
        .bind(b.get("is_tour_completed").and_then(Value::as_bool).unwrap_or(false))
        .execute(&st.db)
        .await?;
    Ok(Json(json!({"message": "Updated successfully"})))
}

// --- session, login accounts, email change (UserSessionEndpoint, AccountEndpoint, UserEndpoint) ---

/// `GET /api/users/session/`: `{"is_authenticated": false}` when signed out, Plane's shape.
async fn session_info(State(st): State<AppState>, MaybeUser(user): MaybeUser) -> ApiResult {
    let Some(user) = user else {
        return Ok(Json(json!({"is_authenticated": false})));
    };
    let Json(me) = json_row(
        sqlx::query_scalar(&format!("SELECT {USER_ME_JSON} FROM users u WHERE u.id = $1")).bind(user.id),
        &st.db,
    )
    .await?;
    Ok(Json(json!({"is_authenticated": true, "user": me})))
}

/// AccountSerializer without the secrets: no access_token, refresh_token, id_token or metadata.
const ACCOUNT_JSON: &str = "jsonb_build_object('id', a.id, 'provider', a.provider,
    'provider_account_id', a.provider_account_id, 'user', a.user_id, 'created_at', a.created_at,
    'updated_at', a.updated_at, 'last_connected_at', a.last_connected_at,
    'access_token_expired_at', a.access_token_expired_at, 'refresh_token_expired_at', a.refresh_token_expired_at)";

async fn accounts_list(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {ACCOUNT_JSON} FROM accounts a WHERE a.user_id = $1 ORDER BY a.created_at"
        ))
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn account_retrieve(State(st): State<AppState>, user: CurrentUser, Path(pk): Path<Uuid>) -> ApiResult {
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {ACCOUNT_JSON} FROM accounts a WHERE a.id = $1 AND a.user_id = $2"
        ))
        .bind(pk)
        .bind(user.id),
        &st.db,
    )
    .await
}

/// Hard delete, as Plane. Refused when it would leave a user with no way to sign in: the password is
/// an auto-set random one (`is_password_autoset`) and this is the last linked account.
/// The user row is locked so two concurrent deletes cannot both pass the check.
async fn account_delete(State(st): State<AppState>, user: CurrentUser, Path(pk): Path<Uuid>) -> ApiResult<StatusCode> {
    let mut tx = st.db.begin().await?;
    let (autoset,): (bool,) = sqlx::query_as("SELECT is_password_autoset FROM users WHERE id = $1 FOR UPDATE")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM accounts WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    let (owned,): (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM accounts WHERE id = $1 AND user_id = $2)")
        .bind(pk)
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    if !owned {
        return Err(ApiError::not_found());
    }
    if autoset && count <= 1 {
        return Err(ApiError::bad(
            "You cannot delete your only login method. Set a password first.",
        ));
    }
    sqlx::query("DELETE FROM accounts WHERE id = $1 AND user_id = $2")
        .bind(pk)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    no_content()
}

/// Increment a fixed-window counter kept in `saturn_magic_codes` (same pattern as the sign-up code issue count).
async fn bump_counter(db: &PgPool, key: &str) -> Result<i32, ApiError> {
    let (n,): (i32,) = sqlx::query_as(
        "INSERT INTO saturn_magic_codes (key, token, attempts, expires_at) VALUES ($1, '', 1, now() + interval '1 hour')
         ON CONFLICT (key) DO UPDATE SET
            attempts = CASE WHEN saturn_magic_codes.expires_at <= now() THEN 1 ELSE saturn_magic_codes.attempts + 1 END,
            expires_at = CASE WHEN saturn_magic_codes.expires_at <= now() THEN now() + interval '1 hour'
                              ELSE saturn_magic_codes.expires_at END
         RETURNING attempts",
    )
    .bind(key)
    .fetch_one(db)
    .await?;
    Ok(n)
}

/// Plane's throttle answers 429 (DRF `Throttled`).
fn throttled() -> ApiError {
    ApiError(
        StatusCode::TOO_MANY_REQUESTS,
        json!({"detail": "Request was throttled. Try again later."}),
    )
}

/// Plane's `_validate_new_email`, same messages.
async fn check_new_email(st: &AppState, user: &CurrentUser, email: &str) -> Result<(), ApiError> {
    if email.is_empty() {
        return Err(ApiError::bad("Email is required"));
    }
    if !valid_email(email) {
        return Err(ApiError::bad("Invalid email format"));
    }
    if email == user.email {
        return Err(ApiError::bad("New email must be different from current email"));
    }
    let (taken,): (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM users WHERE email = $1 AND id <> $2)")
        .bind(email)
        .bind(user.id)
        .fetch_one(&st.db)
        .await?;
    if taken {
        return Err(ApiError::bad("An account with this email already exists"));
    }
    Ok(())
}

/// `POST /api/users/me/email/generate-code/`. 3 requests per user per hour (checked before validation,
/// as DRF throttles run first), 5 codes per target address per hour, code valid 10 minutes.
/// The code is never logged.
async fn generate_email_code(State(st): State<AppState>, user: CurrentUser, Json(b): Json<Value>) -> ApiResult {
    let email = s(&b, "email").unwrap_or("").trim().to_lowercase();
    if bump_counter(&st.db, &format!("email_change_user:{}", user.id)).await? > 3 {
        return Err(throttled());
    }
    check_new_email(&st, &user, &email).await?;
    if bump_counter(&st.db, &format!("email_change_issued:{email}")).await? > 5 {
        return Err(throttled());
    }
    let token = format!("{:06}", rand::thread_rng().gen_range(100000..1000000));
    sqlx::query(
        "INSERT INTO saturn_magic_codes (key, token, attempts, expires_at)
         VALUES ($1, $2, 0, now() + interval '10 minutes')
         ON CONFLICT (key) DO UPDATE SET token = $2, attempts = 0, expires_at = now() + interval '10 minutes'",
    )
    .bind(format!("email_change:{}:{email}", user.id))
    .bind(&token)
    .execute(&st.db)
    .await?;
    crate::outbound::send_mail_bg(
        &st,
        email.clone(),
        "Verify your new email address".into(),
        format!("<p>Your Plane email verification code is <strong>{token}</strong>. It expires in 10 minutes.</p>"),
        format!("Your Plane email verification code is {token}. It expires in 10 minutes."),
    );
    Ok(Json(json!({"message": "Verification code sent to email"})))
}

/// `PATCH /api/users/me/email/` `{email, code}`. Wrong code: 5 tries then the code is deleted. A success
/// consumes the code, updates the email, unverifies it, ends the session (Plane logs the user out) and
/// notifies both addresses.
async fn update_email(
    State(st): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Value>,
) -> Result<Response, ApiError> {
    let email = s(&b, "email").unwrap_or("").trim().to_lowercase();
    let code = s(&b, "code").unwrap_or("").trim().to_string();
    check_new_email(&st, &user, &email).await?;
    if code.is_empty() {
        return Err(ApiError::bad("Verification code is required"));
    }
    let key = format!("email_change:{}:{email}", user.id);
    let mut tx = st.db.begin().await?;
    let record: Option<(String, i32)> = sqlx::query_as(
        "SELECT token, attempts FROM saturn_magic_codes WHERE key = $1 AND expires_at > now() FOR UPDATE",
    )
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((expected, attempts)) = record else {
        return Err(ApiError::bad("Verification code has expired or is invalid"));
    };
    if !token_matches(&expected, &code) {
        if attempts + 1 >= 5 {
            sqlx::query("DELETE FROM saturn_magic_codes WHERE key = $1")
                .bind(&key)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("UPDATE saturn_magic_codes SET attempts = attempts + 1 WHERE key = $1")
                .bind(&key)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        return Err(ApiError::bad("Invalid verification code"));
    }
    sqlx::query("DELETE FROM saturn_magic_codes WHERE key = $1")
        .bind(&key)
        .execute(&mut *tx)
        .await?;
    let updated: Value = match sqlx::query_scalar(&format!(
        "UPDATE users u SET email = $2, is_email_verified = false, updated_at = now()
         WHERE u.id = $1 RETURNING {USER_ME_JSON}"
    ))
    .bind(user.id)
    .bind(&email)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(v) => v,
        // Lost a race with another sign-up for the same address (unique index on users.email).
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") => {
            return Err(ApiError::bad("An account with this email already exists"));
        }
        Err(e) => return Err(e.into()),
    };
    tx.commit().await?;

    let mut headers = HeaderMap::new();
    if let Some(key) = &user.session_key {
        session::destroy(&st, key).await?;
        headers.insert(
            header::SET_COOKIE,
            session::set_cookie(&st, session::COOKIE, None, 0).parse().unwrap(),
        );
    }
    for to in [email.clone(), user.email.clone()] {
        crate::outbound::send_mail_bg(
            &st,
            to,
            "Plane email address successfully updated".into(),
            "<p>The email address on your Plane account was changed. If this was not you, contact your administrator.</p>"
                .into(),
            "The email address on your Plane account was changed. If this was not you, contact your administrator."
                .into(),
        );
    }
    Ok((headers, Json(updated)).into_response())
}

/// `saturn seed <email> <password> [workspace-name]`: instance admin, onboarded user, workspace, project.
/// Makes a fresh database usable without clicking through god-mode setup and onboarding.
pub async fn seed(st: &AppState, args: &[String]) -> anyhow::Result<()> {
    let (Some(email), Some(password)) = (args.first(), args.get(1)) else {
        anyhow::bail!("usage: saturn seed <email> <password> [workspace-name]");
    };
    let ws_name = args.get(2).cloned().unwrap_or_else(|| "Saturn".into());
    let email = email.trim().to_lowercase();
    let db = &st.db;
    let existing: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(db)
        .await?;
    let user_id = match existing {
        Some((id,)) => id,
        None => crate::auth::create_user(db, &email, password, "Admin", "")
            .await
            .map_err(|e| anyhow::anyhow!("{:?}", e.1))?,
    };
    let (instance_id,): (Uuid,) = sqlx::query_as("SELECT id FROM instances ORDER BY created_at LIMIT 1")
        .fetch_one(db)
        .await?;
    sqlx::query(
        "INSERT INTO instance_admins (user_id, instance_id, role, is_verified)
         SELECT $1, $2, 20, true WHERE NOT EXISTS (SELECT 1 FROM instance_admins WHERE user_id = $1)",
    )
    .bind(user_id)
    .bind(instance_id)
    .execute(db)
    .await?;
    sqlx::query("UPDATE instances SET is_setup_done = true, is_signup_screen_visited = true WHERE id = $1")
        .bind(instance_id)
        .execute(db)
        .await?;
    let slug = crate::states::slugify(&ws_name);
    let ws: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM workspaces WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(db)
        .await?;
    let workspace_id = match ws {
        Some((id,)) => id,
        None => {
            let mut tx = db.begin().await?;
            let id = crate::workspaces::create_workspace(&mut tx, user_id, &ws_name, &slug, "")
                .await
                .map_err(|e| anyhow::anyhow!("{:?}", e.1))?;
            crate::projects::create_project(&mut tx, id, user_id, "Getting started", "GS", &json!({}))
                .await
                .map_err(|e| anyhow::anyhow!("{:?}", e.1))?;
            tx.commit().await?;
            id
        }
    };
    sqlx::query(
        "UPDATE profiles SET is_onboarded = true, is_tour_completed = true, last_workspace_id = $2 WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(workspace_id)
    .execute(db)
    .await?;
    println!("seeded {email} (instance admin) in workspace /{slug}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::valid_cover_image;

    #[test]
    fn cover_image_urls() {
        for ok in [
            "https://images.unsplash.com/photo-1.jpg",
            "http://minio:9000/uploads/x.png",
            "/api/assets/v2/static/00000000-0000-0000-0000-000000000001/",
        ] {
            assert!(valid_cover_image(ok), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,x",
            "ftp://example.com/x.png",
            "not a url",
            "/api/assets/../../etc/passwd",
            "//evil.example/x.png",
            "https://x.example/\nset-cookie",
        ] {
            assert!(!valid_cover_image(bad), "{bad}");
        }
        assert!(!valid_cover_image(&format!("https://x.example/{}", "a".repeat(800))));
    }
}
