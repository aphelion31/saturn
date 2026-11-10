//! `/api/users/me/*`: current user, profile, settings, onboarding. Also the `saturn seed` command.
use axum::{
    Json, Router,
    extract::State,
    routing::{get, patch},
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/users/me/", get(me).patch(update_me))
        .route("/api/users/me/settings/", get(settings))
        .route("/api/users/me/profile/", get(profile).patch(update_profile))
        .route("/api/users/me/instance-admin/", get(instance_admin))
        .route("/api/users/me/onboard/", patch(onboard))
        .route("/api/users/me/tour-completed/", patch(tour_completed))
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

async fn update_me(State(st): State<AppState>, user: CurrentUser, Json(b): Json<Value>) -> ApiResult {
    if let Some(dn) = s(&b, "display_name")
        && dn.trim().is_empty()
    {
        return Err(ApiError::bad_body(
            json!({"display_name": ["This field may not be blank."]}),
        ));
    }
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE users u SET first_name = COALESCE($2, first_name), last_name = COALESCE($3, last_name),
                display_name = COALESCE($4, display_name), user_timezone = COALESCE($5, user_timezone),
                avatar = COALESCE($6, avatar), cover_image = COALESCE($7, cover_image), updated_at = now()
             WHERE u.id = $1 RETURNING {USER_ME_JSON}"
        ))
        .bind(user.id)
        .bind(s(&b, "first_name"))
        .bind(s(&b, "last_name"))
        .bind(s(&b, "display_name"))
        .bind(s(&b, "user_timezone"))
        .bind(s(&b, "avatar"))
        .bind(s(&b, "cover_image")),
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
    ensure_profile(&st, user.id).await?;
    let mut sets = patch_sets(&b, PROFILE_FIELDS, 2);
    sets.push("updated_at = now()".into());
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE profiles p SET {} WHERE p.user_id = $1 RETURNING {PROFILE_JSON}",
            sets.join(", ")
        ))
        .bind(user.id)
        .bind(&b),
        &st.db,
    )
    .await
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
