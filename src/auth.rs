//! `/auth/*` (app and space sign in, sign up, sign out, email check) and `/api/instances/*`
//! (instance info, god-mode admin sign up and sign in).
//!
//! Plane's auth endpoints are HTML form posts that answer with a 302 redirect; errors travel back
//! as `?error_code=..&error_message=..` query params, which the React apps read. Saturn keeps that.
use axum::{
    Form, Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::{ApiError, ApiResult, AppState, CurrentUser, MaybeUser};
use crate::session;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/get-csrf-token/", get(csrf_token))
        .route("/auth/email-check/", post(email_check))
        .route("/auth/spaces/email-check/", post(email_check))
        .route("/auth/sign-in/", post(sign_in_app))
        .route("/auth/sign-up/", post(sign_up_app))
        .route("/auth/spaces/sign-in/", post(sign_in_space))
        .route("/auth/spaces/sign-up/", post(sign_up_space))
        .route("/auth/sign-out/", post(sign_out_app))
        .route("/auth/spaces/sign-out/", post(sign_out_space))
        .route("/api/instances/", get(instance_info))
        .route("/api/instances/admins/sign-up/", post(admin_sign_up))
        .route("/api/instances/admins/sign-in/", post(admin_sign_in))
        .route("/api/instances/admins/sign-out/", post(admin_sign_out))
        .route("/api/instances/admins/session/", get(admin_session))
        .route("/api/instances/admins/me/", get(admin_me))
        .route(
            "/api/instances/admins/sign-up-screen-visited/",
            post(sign_up_screen_visited),
        )
}

/// Plane's numeric auth error codes (plane/authentication/adapter/error.py), the subset Saturn emits.
fn code(name: &str) -> u32 {
    match name {
        "INSTANCE_NOT_CONFIGURED" => 5000,
        "INVALID_EMAIL" => 5005,
        "EMAIL_REQUIRED" => 5010,
        "SIGNUP_DISABLED" => 5015,
        "PASSWORD_TOO_WEAK" => 5020,
        "USER_ALREADY_EXIST" => 5030,
        "AUTHENTICATION_FAILED_SIGN_UP" => 5035,
        "REQUIRED_EMAIL_PASSWORD_SIGN_UP" => 5040,
        "INVALID_EMAIL_SIGN_UP" => 5045,
        "USER_DOES_NOT_EXIST" => 5060,
        "AUTHENTICATION_FAILED_SIGN_IN" => 5065,
        "REQUIRED_EMAIL_PASSWORD_SIGN_IN" => 5070,
        "INVALID_EMAIL_SIGN_IN" => 5075,
        "USER_ACCOUNT_DEACTIVATED" => 5019,
        "ADMIN_ALREADY_EXIST" => 5150,
        "REQUIRED_ADMIN_EMAIL_PASSWORD_FIRST_NAME" => 5155,
        "INVALID_ADMIN_EMAIL" => 5160,
        "INVALID_ADMIN_PASSWORD" => 5165,
        "REQUIRED_ADMIN_EMAIL_PASSWORD" => 5170,
        "ADMIN_AUTHENTICATION_FAILED" => 5175,
        "ADMIN_USER_ALREADY_EXIST" => 5180,
        "ADMIN_USER_DOES_NOT_EXIST" => 5185,
        _ => 5999,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum App {
    Web,
    Space,
    Admin,
}

fn base(st: &AppState, app: App) -> String {
    let w = st.cfg.web_url.trim_end_matches('/');
    match app {
        App::Web => w.to_string(),
        App::Space => format!("{w}/spaces/"),
        App::Admin => format!("{w}/god-mode/"),
    }
}

/// Same rules as Plane's `validate_next_path`: relative path only, no `//`, no `..`.
fn safe_next(p: Option<&str>) -> String {
    let Some(p) = p else { return String::new() };
    if p.len() > 500 {
        return String::new();
    }
    let p: String = p.chars().filter(|c| !matches!(c, '\\' | '\t' | '\r' | '\n')).collect();
    let p = match url::Url::parse(&p) {
        Ok(u) => u.path().to_string(),
        Err(_) => p,
    };
    if !p.starts_with('/') || p.starts_with("//") || p.contains("..") {
        return String::new();
    }
    p
}

/// `get_safe_redirect_url(base, next_path, params)`.
fn redirect_url(st: &AppState, app: App, next: &str, params: &[(&str, String)]) -> String {
    let b = base(st, app);
    let b = b.trim_end_matches('/');
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    if !next.is_empty() {
        q.append_pair("next_path", next);
    }
    for (k, v) in params {
        q.append_pair(k, v);
    }
    let q = q.finish();
    if q.is_empty() {
        b.to_string()
    } else {
        format!("{b}/?{q}")
    }
}

fn err_redirect(st: &AppState, app: App, next: &str, name: &str, email: Option<&str>) -> Response {
    let mut p = vec![
        ("error_code", code(name).to_string()),
        ("error_message", name.to_string()),
    ];
    if let Some(e) = email {
        p.push(("email", e.to_string()));
    }
    Redirect::to(&redirect_url(st, app, next, &p)).into_response()
}

fn redirect_with_cookie(to: &str, cookie: String) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, to.to_string()), (header::SET_COOKIE, cookie)],
    )
        .into_response()
}

fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !e.contains(' ')
}

/// ponytail: Plane uses zxcvbn score >= 3. Saturn checks length and character variety instead.
fn strong(p: &str) -> bool {
    let classes = [
        p.chars().any(|c| c.is_ascii_lowercase()),
        p.chars().any(|c| c.is_ascii_uppercase()),
        p.chars().any(|c| c.is_ascii_digit()),
        p.chars().any(|c| !c.is_ascii_alphanumeric()),
    ];
    p.chars().count() >= 8 && classes.iter().filter(|x| **x).count() >= 3
}

async fn config_value(st: &AppState, key: &str, default: &str) -> String {
    let v: Option<(Option<String>,)> =
        sqlx::query_as("SELECT value FROM instance_configurations WHERE key = $1 AND deleted_at IS NULL")
            .bind(key)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();
    v.and_then(|(v,)| v)
        .unwrap_or_else(|| std::env::var(key).unwrap_or_else(|_| default.to_string()))
}

async fn setup_done(st: &AppState) -> Result<bool, ApiError> {
    let v: Option<(bool,)> = sqlx::query_as("SELECT is_setup_done FROM instances ORDER BY created_at LIMIT 1")
        .fetch_optional(&st.db)
        .await?;
    Ok(v.map(|(b,)| b).unwrap_or(false))
}

fn device_info(headers: &HeaderMap, st: &AppState, app: App) -> Value {
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .unwrap_or("")
        .trim();
    json!({"user_agent": ua, "ip_address": ip, "domain": base(st, app)})
}

/// Where to go after sign in, as `get_redirection_path`.
async fn landing(st: &AppState, user_id: Uuid, email: &str) -> Result<String, ApiError> {
    let row: Option<(bool, Option<Uuid>)> =
        sqlx::query_as("SELECT is_onboarded, last_workspace_id FROM profiles WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&st.db)
            .await?;
    let (onboarded, last) = row.unwrap_or((false, None));
    if !onboarded {
        return Ok("onboarding".into());
    }
    let slug: Option<(String,)> = sqlx::query_as(
        "SELECT w.slug FROM workspaces w JOIN workspace_members m ON m.workspace_id = w.id
         WHERE m.member_id = $1 AND m.is_active AND m.deleted_at IS NULL AND w.deleted_at IS NULL
         ORDER BY (w.id = $2) DESC NULLS LAST, w.created_at LIMIT 1",
    )
    .bind(user_id)
    .bind(last)
    .fetch_optional(&st.db)
    .await?;
    if let Some((s,)) = slug {
        return Ok(s);
    }
    let invites: (i64,) =
        sqlx::query_as("SELECT count(*) FROM workspace_member_invites WHERE email = $1 AND deleted_at IS NULL")
            .bind(email)
            .fetch_one(&st.db)
            .await?;
    Ok(if invites.0 > 0 {
        "invitations".into()
    } else {
        "create-workspace".into()
    })
}

/// Join workspaces and projects whose invites this email already accepted (`process_workspace_project_invitations`).
async fn join_accepted_invites(st: &AppState, user_id: Uuid, email: &str) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, member_id, role)
         SELECT workspace_id, $1, role FROM workspace_member_invites WHERE email = $2 AND accepted AND deleted_at IS NULL
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(email)
    .execute(&st.db)
    .await?;
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, member_id, role, created_by_id)
         SELECT workspace_id, $1, CASE WHEN role IN (5, 15) THEN role ELSE 15 END, created_by_id
         FROM project_member_invites WHERE email = $2 AND accepted AND deleted_at IS NULL ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(email)
    .execute(&st.db)
    .await?;
    sqlx::query(
        "INSERT INTO project_members (workspace_id, project_id, member_id, role, created_by_id)
         SELECT workspace_id, project_id, $1, CASE WHEN role IN (5, 15) THEN role ELSE 15 END, created_by_id
         FROM project_member_invites WHERE email = $2 AND accepted AND deleted_at IS NULL ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(email)
    .execute(&st.db)
    .await?;
    Ok(())
}

/// Insert a user with profile and notification preferences, as Plane's signup does.
pub async fn create_user(
    db: &sqlx::PgPool,
    email: &str,
    password: &str,
    first_name: &str,
    last_name: &str,
) -> Result<Uuid, ApiError> {
    let display = email.split('@').next().unwrap_or("user").to_string();
    let mut tx = db.begin().await?;
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO users (email, username, password, first_name, last_name, display_name, token, is_active,
                            last_login_medium, token_updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, true, 'email', now()) RETURNING id",
    )
    .bind(email)
    .bind(Uuid::new_v4().simple().to_string())
    .bind(session::hash_password(password))
    .bind(first_name)
    .bind(last_name)
    .bind(display)
    .bind(format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()))
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO profiles (user_id) VALUES ($1)")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO user_notification_preferences (user_id, property_change, state_change, comment, mention, issue_completed)
         VALUES ($1, true, true, true, true, true)",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

async fn csrf_token() -> Json<Value> {
    // Plane's SPAs post forms with this token, but Saturn (like Plane's DRF auth) protects
    // cookie-authenticated writes by checking Origin instead. Any opaque value is accepted.
    Json(json!({"csrf_token": Uuid::new_v4().simple().to_string()}))
}

#[derive(Deserialize)]
struct EmailCheck {
    email: Option<String>,
}

async fn email_check(State(st): State<AppState>, Json(b): Json<EmailCheck>) -> Response {
    let fail = |n: &str| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error_code": code(n), "error_message": n})),
        )
            .into_response()
    };
    match setup_done(&st).await {
        Ok(true) => {}
        _ => return fail("INSTANCE_NOT_CONFIGURED"),
    }
    let Some(email) = b.email.map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()) else {
        return fail("EMAIL_REQUIRED");
    };
    if !valid_email(&email) {
        return fail("INVALID_EMAIL");
    }
    // ponytail: no SMTP in Saturn, so magic codes are never offered.
    let existing: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();
    Json(json!({"existing": existing.is_some(), "status": "CREDENTIAL"})).into_response()
}

#[derive(Deserialize, Default)]
struct AuthForm {
    email: Option<String>,
    password: Option<String>,
    next_path: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
    company_name: Option<String>,
    is_telemetry_enabled: Option<String>,
}

async fn sign_in(st: AppState, headers: HeaderMap, f: AuthForm, app: App) -> Result<Response, ApiError> {
    let next = safe_next(f.next_path.as_deref());
    if !setup_done(&st).await? {
        return Ok(err_redirect(&st, app, &next, "INSTANCE_NOT_CONFIGURED", None));
    }
    let email = f.email.unwrap_or_default().trim().to_lowercase();
    let password = f.password.unwrap_or_default();
    if email.is_empty() || password.is_empty() {
        return Ok(err_redirect(
            &st,
            app,
            &next,
            "REQUIRED_EMAIL_PASSWORD_SIGN_IN",
            Some(&email),
        ));
    }
    if !valid_email(&email) {
        return Ok(err_redirect(&st, app, &next, "INVALID_EMAIL_SIGN_IN", Some(&email)));
    }
    let row: Option<(Uuid, String, bool, bool, bool)> = sqlx::query_as(
        "SELECT id, password, is_active, is_bot, last_logout_time IS NOT NULL FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&st.db)
    .await?;
    let Some((id, hash, active, bot, logged_out_before)) = row else {
        return Ok(err_redirect(&st, app, &next, "USER_DOES_NOT_EXIST", Some(&email)));
    };
    if bot || !session::check_password(&password, &hash) {
        return Ok(err_redirect(
            &st,
            app,
            &next,
            "AUTHENTICATION_FAILED_SIGN_IN",
            Some(&email),
        ));
    }
    if !active && logged_out_before {
        return Ok(err_redirect(&st, app, &next, "USER_ACCOUNT_DEACTIVATED", Some(&email)));
    }
    sqlx::query("UPDATE users SET is_active = true, last_login_medium = 'email' WHERE id = $1")
        .bind(id)
        .execute(&st.db)
        .await?;
    join_accepted_invites(&st, id, &email).await?;
    let uagent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    sqlx::query("UPDATE users SET last_login_uagent = $2 WHERE id = $1")
        .bind(id)
        .bind(uagent)
        .execute(&st.db)
        .await?;
    let key = session::create(&st, id, false, device_info(&headers, &st, app)).await?;
    let path = if !next.is_empty() {
        next.clone()
    } else if app == App::Web {
        landing(&st, id, &email).await?
    } else {
        String::new()
    };
    let to = if app == App::Web {
        format!(
            "{}/{}",
            base(&st, app).trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    } else {
        redirect_url(&st, app, &path, &[])
    };
    Ok(redirect_with_cookie(
        &to,
        session::set_cookie(&st, session::COOKIE, Some(&key), session::AGE_SECS),
    ))
}

async fn sign_up(st: AppState, headers: HeaderMap, f: AuthForm, app: App) -> Result<Response, ApiError> {
    let next = safe_next(f.next_path.as_deref());
    if !setup_done(&st).await? {
        return Ok(err_redirect(&st, app, &next, "INSTANCE_NOT_CONFIGURED", None));
    }
    let email = f.email.clone().unwrap_or_default().trim().to_lowercase();
    let password = f.password.clone().unwrap_or_default();
    if email.is_empty() || password.is_empty() {
        return Ok(err_redirect(
            &st,
            app,
            &next,
            "REQUIRED_EMAIL_PASSWORD_SIGN_UP",
            Some(&email),
        ));
    }
    if !valid_email(&email) {
        return Ok(err_redirect(&st, app, &next, "INVALID_EMAIL_SIGN_UP", Some(&email)));
    }
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&st.db)
        .await?;
    if exists.is_some() {
        return Ok(err_redirect(&st, app, &next, "USER_ALREADY_EXIST", Some(&email)));
    }
    if config_value(&st, "ENABLE_SIGNUP", "1").await == "0" {
        let invited: (i64,) =
            sqlx::query_as("SELECT count(*) FROM workspace_member_invites WHERE email = $1 AND deleted_at IS NULL")
                .bind(&email)
                .fetch_one(&st.db)
                .await?;
        if invited.0 == 0 {
            return Ok(err_redirect(&st, app, &next, "SIGNUP_DISABLED", Some(&email)));
        }
    }
    if !strong(&password) {
        return Ok(err_redirect(&st, app, &next, "PASSWORD_TOO_WEAK", Some(&email)));
    }
    create_user(&st.db, &email, &password, "", "").await?;
    sign_in(
        st,
        headers,
        AuthForm {
            email: Some(email),
            password: Some(password),
            next_path: f.next_path,
            ..Default::default()
        },
        app,
    )
    .await
}

async fn sign_in_app(State(st): State<AppState>, h: HeaderMap, Form(f): Form<AuthForm>) -> Result<Response, ApiError> {
    sign_in(st, h, f, App::Web).await
}
async fn sign_up_app(State(st): State<AppState>, h: HeaderMap, Form(f): Form<AuthForm>) -> Result<Response, ApiError> {
    sign_up(st, h, f, App::Web).await
}
async fn sign_in_space(
    State(st): State<AppState>,
    h: HeaderMap,
    Form(f): Form<AuthForm>,
) -> Result<Response, ApiError> {
    sign_in(st, h, f, App::Space).await
}
async fn sign_up_space(
    State(st): State<AppState>,
    h: HeaderMap,
    Form(f): Form<AuthForm>,
) -> Result<Response, ApiError> {
    sign_up(st, h, f, App::Space).await
}

async fn sign_out(st: &AppState, user: MaybeUser, cookie_name: &str, to: String) -> Result<Response, ApiError> {
    if let Some(u) = user.0 {
        sqlx::query("UPDATE users SET last_logout_time = now() WHERE id = $1")
            .bind(u.id)
            .execute(&st.db)
            .await?;
        if let Some(k) = u.session_key {
            session::destroy(st, &k).await?;
        }
    }
    Ok(redirect_with_cookie(&to, session::set_cookie(st, cookie_name, None, 0)))
}

async fn sign_out_app(State(st): State<AppState>, u: MaybeUser) -> Result<Response, ApiError> {
    let to = base(&st, App::Web);
    sign_out(&st, u, session::COOKIE, to).await
}
async fn sign_out_space(
    State(st): State<AppState>,
    u: MaybeUser,
    Form(f): Form<AuthForm>,
) -> Result<Response, ApiError> {
    let to = redirect_url(&st, App::Space, &safe_next(f.next_path.as_deref()), &[]);
    sign_out(&st, u, session::COOKIE, to).await
}
async fn admin_sign_out(State(st): State<AppState>, u: MaybeUser) -> Result<Response, ApiError> {
    let to = base(&st, App::Admin);
    sign_out(&st, u, session::ADMIN_COOKIE, to).await
}

async fn is_instance_admin(st: &AppState, user_id: Uuid) -> Result<bool, ApiError> {
    let v: (bool,) =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM instance_admins WHERE user_id = $1 AND deleted_at IS NULL)")
            .bind(user_id)
            .fetch_one(&st.db)
            .await?;
    Ok(v.0)
}

/// 403 unless the caller is an instance admin. Use for every `/api/instances/*` write.
pub async fn require_instance_admin(st: &AppState, user: &CurrentUser) -> Result<(), ApiError> {
    if is_instance_admin(st, user.id).await? {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}

/// Make sure the single `instances` row exists (Plane creates it with `register_instance`).
pub async fn ensure_instance(db: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO instances (instance_name, instance_id, current_version, latest_version, last_checked_at, domain,
                                is_setup_done, is_signup_screen_visited, is_verified, is_telemetry_enabled,
                                is_support_required, edition, is_test, is_current_version_deprecated)
         SELECT 'Saturn', $1, $2, $2, now(), '', false, false, true, false, false, 'SATURN', false, false
         WHERE NOT EXISTS (SELECT 1 FROM instances)",
    )
    .bind(Uuid::new_v4().simple().to_string())
    .bind(env!("CARGO_PKG_VERSION"))
    .execute(db)
    .await?;
    Ok(())
}

async fn instance_info(State(st): State<AppState>) -> ApiResult {
    let instance: Option<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(i) || jsonb_build_object('workspaces_exist', EXISTS (SELECT 1 FROM workspaces WHERE deleted_at IS NULL))
         FROM instances i ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(&st.db)
    .await?;
    let Some(instance) = instance else {
        return Ok(Json(json!({"is_activated": false, "is_setup_done": false})));
    };
    let on = |v: String| v == "1";
    let cfg = json!({
        "enable_signup": on(config_value(&st, "ENABLE_SIGNUP", "0").await),
        "is_workspace_creation_disabled": on(config_value(&st, "DISABLE_WORKSPACE_CREATION", "0").await),
        "is_google_enabled": false,
        "is_github_enabled": false,
        "is_gitlab_enabled": false,
        "is_gitea_enabled": false,
        "is_magic_login_enabled": false,
        "is_email_password_enabled": true,
        "github_app_name": "",
        "slack_client_id": null,
        "has_unsplash_configured": !config_value(&st, "UNSPLASH_ACCESS_KEY", "").await.is_empty(),
        "has_llm_configured": false,
        "file_size_limit": st.cfg.file_size_limit as f64,
        "is_smtp_configured": false,
        "admin_base_url": "",
        "space_base_url": "",
        "app_base_url": "",
        "instance_changelog_url": "",
        "is_self_managed": true,
    });
    Ok(Json(json!({"config": cfg, "instance": instance})))
}

async fn admin_sign_up(
    State(st): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<AuthForm>,
) -> Result<Response, ApiError> {
    let back = |n: &str, email: &str| err_redirect(&st, App::Admin, "", n, Some(email));
    let email = f.email.clone().unwrap_or_default().trim().to_lowercase();
    let password = f.password.clone().unwrap_or_default();
    let first = f.first_name.clone().unwrap_or_default();
    let mut tx = st.db.begin().await?;
    let inst: Option<(Uuid, bool)> =
        sqlx::query_as("SELECT id, is_setup_done FROM instances ORDER BY created_at LIMIT 1 FOR UPDATE")
            .fetch_optional(&mut *tx)
            .await?;
    let Some((instance_id, done)) = inst else {
        return Ok(back("INSTANCE_NOT_CONFIGURED", &email));
    };
    let admins: (i64,) = sqlx::query_as("SELECT count(*) FROM instance_admins")
        .fetch_one(&mut *tx)
        .await?;
    if done || admins.0 > 0 {
        return Ok(back("ADMIN_ALREADY_EXIST", &email));
    }
    if email.is_empty() || password.is_empty() || first.is_empty() {
        return Ok(back("REQUIRED_ADMIN_EMAIL_PASSWORD_FIRST_NAME", &email));
    }
    if !valid_email(&email) {
        return Ok(back("INVALID_ADMIN_EMAIL", &email));
    }
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_some() {
        return Ok(back("ADMIN_USER_ALREADY_EXIST", &email));
    }
    if !strong(&password) {
        return Ok(back("INVALID_ADMIN_PASSWORD", &email));
    }
    drop(tx);
    let id = create_user(&st.db, &email, &password, &first, f.last_name.as_deref().unwrap_or("")).await?;
    let company = f.company_name.unwrap_or_default();
    sqlx::query("UPDATE profiles SET company_name = $2 WHERE user_id = $1")
        .bind(id)
        .bind(&company)
        .execute(&st.db)
        .await?;
    sqlx::query("UPDATE users SET is_superuser = false, is_staff = false WHERE id = $1")
        .bind(id)
        .execute(&st.db)
        .await?;
    sqlx::query("INSERT INTO instance_admins (user_id, instance_id, role, is_verified) VALUES ($1, $2, 20, false)")
        .bind(id)
        .bind(instance_id)
        .execute(&st.db)
        .await?;
    sqlx::query(
        "UPDATE instances SET is_setup_done = true, instance_name = $2, is_telemetry_enabled = $3, updated_at = now() WHERE id = $1",
    )
    .bind(instance_id)
    .bind(if company.is_empty() { "Saturn".to_string() } else { company })
    .bind(f.is_telemetry_enabled.as_deref() == Some("True"))
    .execute(&st.db)
    .await?;
    let key = session::create(&st, id, true, device_info(&headers, &st, App::Admin)).await?;
    let to = format!("{}general/", base(&st, App::Admin));
    Ok(redirect_with_cookie(
        &to,
        session::set_cookie(&st, session::ADMIN_COOKIE, Some(&key), session::ADMIN_AGE_SECS),
    ))
}

async fn admin_sign_in(
    State(st): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<AuthForm>,
) -> Result<Response, ApiError> {
    let email = f.email.unwrap_or_default().trim().to_lowercase();
    let password = f.password.unwrap_or_default();
    let back = |n: &str| err_redirect(&st, App::Admin, "", n, Some(&email));
    if !setup_done(&st).await? {
        return Ok(back("INSTANCE_NOT_CONFIGURED"));
    }
    if email.is_empty() || password.is_empty() {
        return Ok(back("REQUIRED_ADMIN_EMAIL_PASSWORD"));
    }
    if !valid_email(&email) {
        return Ok(back("INVALID_ADMIN_EMAIL"));
    }
    let row: Option<(Uuid, String, bool)> =
        sqlx::query_as("SELECT id, password, is_active FROM users WHERE email = $1")
            .bind(&email)
            .fetch_optional(&st.db)
            .await?;
    let Some((id, hash, active)) = row else {
        return Ok(back("ADMIN_USER_DOES_NOT_EXIST"));
    };
    if !active || !session::check_password(&password, &hash) || !is_instance_admin(&st, id).await? {
        return Ok(back("ADMIN_AUTHENTICATION_FAILED"));
    }
    let uagent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    sqlx::query("UPDATE users SET last_login_uagent = $2 WHERE id = $1")
        .bind(id)
        .bind(uagent)
        .execute(&st.db)
        .await?;
    let key = session::create(&st, id, true, device_info(&headers, &st, App::Admin)).await?;
    let to = format!("{}general/", base(&st, App::Admin));
    Ok(redirect_with_cookie(
        &to,
        session::set_cookie(&st, session::ADMIN_COOKIE, Some(&key), session::ADMIN_AGE_SECS),
    ))
}

const ADMIN_ME_COLS: &str =
    "jsonb_build_object('id', u.id, 'avatar', u.avatar, 'avatar_url', u.avatar, 'cover_image', u.cover_image,
    'date_joined', u.date_joined, 'display_name', u.display_name, 'email', u.email, 'first_name', u.first_name,
    'last_name', u.last_name, 'is_active', u.is_active, 'is_bot', u.is_bot, 'is_email_verified', u.is_email_verified,
    'user_timezone', u.user_timezone, 'username', u.username, 'is_password_autoset', u.is_password_autoset)";

async fn admin_session(State(st): State<AppState>, u: MaybeUser) -> ApiResult {
    if let Some(u) = u.0
        && is_instance_admin(&st, u.id).await?
    {
        let user: Value = sqlx::query_scalar(&format!("SELECT {ADMIN_ME_COLS} FROM users u WHERE id = $1"))
            .bind(u.id)
            .fetch_one(&st.db)
            .await?;
        return Ok(Json(json!({"is_authenticated": true, "user": user})));
    }
    Ok(Json(json!({"is_authenticated": false})))
}

async fn admin_me(State(st): State<AppState>, u: CurrentUser) -> ApiResult {
    require_instance_admin(&st, &u).await?;
    let user: Value = sqlx::query_scalar(&format!("SELECT {ADMIN_ME_COLS} FROM users u WHERE id = $1"))
        .bind(u.id)
        .fetch_one(&st.db)
        .await?;
    Ok(Json(user))
}

async fn sign_up_screen_visited(State(st): State<AppState>) -> ApiResult {
    sqlx::query("UPDATE instances SET is_signup_screen_visited = true")
        .execute(&st.db)
        .await?;
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_path_rules() {
        assert_eq!(safe_next(Some("/acme/projects")), "/acme/projects");
        assert_eq!(safe_next(Some("//evil.com")), "");
        assert_eq!(safe_next(Some("/\t/evil.com")), "");
        assert_eq!(safe_next(Some("https://evil.com/x")), "/x");
        assert_eq!(safe_next(Some("/a/../b")), "");
        assert_eq!(safe_next(Some("relative")), "");
    }

    #[test]
    fn password_rule() {
        assert!(strong("Saturn-rocks-2026"));
        assert!(!strong("password"));
        assert!(!strong("Ab1!"));
    }
}
