//! God-mode instance administration: `/api/instances/configurations/`, `/api/instances/admins/`,
//! `/api/instances/workspaces/`, slug check. Auth (sign in/up, session) lives in auth.rs.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth::require_instance_admin;
use crate::common::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/instances/configurations/",
            get(configurations).patch(update_configurations),
        )
        .route(
            "/api/instances/configurations/disable-email-feature/",
            post(disable_email).delete(disable_email),
        )
        .route("/api/instances/email-credentials-check/", post(email_check))
        .route("/api/instances/admins/", get(admins).post(add_admin))
        .route("/api/instances/admins/{pk}/", delete(remove_admin))
        .route("/api/instances/workspaces/", get(workspaces).post(create_workspace))
        .route("/api/instances/workspace-slug-check/", get(slug_check))
        .route("/api/instances/changelog/", get(|| async { Json(json!([])) }))
}

/// Keys and defaults of Plane's `instance_config_variables` that Saturn uses or shows in god-mode.
/// Encrypted keys (OAuth secrets, SMTP password, LLM and Unsplash keys) belong to features Saturn
/// does not run, so they are listed but never stored or returned.
const CONFIG: &[(&str, &str, &str, bool)] = &[
    ("ENABLE_SIGNUP", "AUTHENTICATION", "1", false),
    ("ENABLE_EMAIL_PASSWORD", "AUTHENTICATION", "1", false),
    ("ENABLE_MAGIC_LINK_LOGIN", "AUTHENTICATION", "0", false),
    ("DISABLE_WORKSPACE_CREATION", "WORKSPACE_MANAGEMENT", "0", false),
    ("IS_GOOGLE_ENABLED", "GOOGLE", "0", false),
    ("GOOGLE_CLIENT_ID", "GOOGLE", "", false),
    ("GOOGLE_CLIENT_SECRET", "GOOGLE", "", true),
    ("IS_GITHUB_ENABLED", "GITHUB", "0", false),
    ("GITHUB_CLIENT_ID", "GITHUB", "", false),
    ("GITHUB_CLIENT_SECRET", "GITHUB", "", true),
    ("IS_GITLAB_ENABLED", "GITLAB", "0", false),
    ("GITLAB_HOST", "GITLAB", "", false),
    ("GITLAB_CLIENT_ID", "GITLAB", "", false),
    ("GITLAB_CLIENT_SECRET", "GITLAB", "", true),
    ("IS_GITEA_ENABLED", "GITEA", "0", false),
    ("GITEA_HOST", "GITEA", "", false),
    ("GITEA_CLIENT_ID", "GITEA", "", false),
    ("GITEA_CLIENT_SECRET", "GITEA", "", true),
    ("ENABLE_SMTP", "SMTP", "0", false),
    ("EMAIL_HOST", "SMTP", "", false),
    ("EMAIL_HOST_USER", "SMTP", "", false),
    ("EMAIL_HOST_PASSWORD", "SMTP", "", true),
    ("EMAIL_PORT", "SMTP", "587", false),
    ("EMAIL_FROM", "SMTP", "", false),
    ("EMAIL_USE_TLS", "SMTP", "1", false),
    ("EMAIL_USE_SSL", "SMTP", "0", false),
    ("LLM_API_KEY", "AI", "", true),
    ("LLM_PROVIDER", "AI", "openai", false),
    ("LLM_MODEL", "AI", "gpt-4o-mini", false),
    ("UNSPLASH_ACCESS_KEY", "UNSPLASH", "", true),
];

/// Insert missing configuration rows (Plane's `configure_instance` command), env vars first.
pub async fn ensure_configurations(db: &sqlx::PgPool) -> anyhow::Result<()> {
    for (key, category, default, encrypted) in CONFIG {
        let value = if *encrypted {
            String::new()
        } else {
            std::env::var(key).unwrap_or_else(|_| default.to_string())
        };
        sqlx::query(
            "INSERT INTO instance_configurations (key, value, category, is_encrypted)
             SELECT $1, $2, $3, $4 WHERE NOT EXISTS (SELECT 1 FROM instance_configurations WHERE key = $1)",
        )
        .bind(key)
        .bind(value)
        .bind(category)
        .bind(encrypted)
        .execute(db)
        .await?;
    }
    Ok(())
}

const CONFIG_JSON: &str = "jsonb_build_object('id', c.id, 'key', c.key,
    'value', CASE WHEN c.is_encrypted THEN '' ELSE c.value END, 'category', c.category, 'is_encrypted', c.is_encrypted,
    'created_at', c.created_at, 'updated_at', c.updated_at, 'created_by', c.created_by_id, 'updated_by', c.updated_by_id,
    'deleted_at', c.deleted_at)";

async fn configurations(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    require_instance_admin(&st, &user).await?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {CONFIG_JSON} FROM instance_configurations c WHERE c.deleted_at IS NULL ORDER BY c.key"
        )),
        &st.db,
    )
    .await
}

async fn update_configurations(State(st): State<AppState>, user: CurrentUser, Json(b): Json<Value>) -> ApiResult {
    require_instance_admin(&st, &user).await?;
    let Some(obj) = b.as_object() else {
        return Err(ApiError::bad("Expected an object of key: value"));
    };
    let mut keys = Vec::new();
    for (k, v) in obj {
        let value = match v {
            Value::Null => String::new(),
            Value::String(s) => s.trim().to_string(),
            other => other.to_string(),
        };
        // Plane stores secrets (OAuth client secrets, SMTP password, LLM key) Fernet-encrypted with SECRET_KEY.
        sqlx::query(
            "UPDATE instance_configurations
             SET value = CASE WHEN is_encrypted THEN $4 ELSE $2 END, updated_by_id = $3, updated_at = now()
             WHERE key = $1 AND deleted_at IS NULL",
        )
        .bind(k)
        .bind(&value)
        .bind(user.id)
        .bind(crate::outbound::encrypt(&st.cfg.secret_key, &value))
        .execute(&st.db)
        .await?;
        keys.push(k.clone());
    }
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {CONFIG_JSON} FROM instance_configurations c WHERE c.key = ANY($1) ORDER BY c.key"
        ))
        .bind(keys),
        &st.db,
    )
    .await
}

/// Plane EmailCredentialCheckEndpoint: send a test email with the stored SMTP settings.
async fn email_check(State(st): State<AppState>, user: CurrentUser, Json(b): Json<Value>) -> ApiResult {
    require_instance_admin(&st, &user).await?;
    let Some(to) = b
        .get("receiver_email")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    else {
        return Err(ApiError::bad("Receiver email is required"));
    };
    // Plane #9491 / #9487: an empty or invalid port is a 400, not a 500.
    let Some(smtp) = crate::outbound::smtp(&st).await else {
        return Err(ApiError::bad("Could not connect with the SMTP server."));
    };
    let text = "This is a sample email notification sent from Plane application.";
    match crate::outbound::send_mail(&smtp, to.trim(), "Email Notification from Plane", text, text).await {
        Ok(()) => Ok(Json(json!({"message": "Email successfully sent."}))),
        Err(e) => {
            let msg = if e.contains("535") || e.to_lowercase().contains("auth") {
                "Invalid credentials provided"
            } else if e.contains("from address") {
                "From address is invalid."
            } else if e.contains("to address") {
                "Invalid email header."
            } else {
                "Could not connect with the SMTP server."
            };
            tracing::warn!(error = %e, "test email failed");
            Err(ApiError::bad(msg))
        }
    }
}

async fn disable_email(State(st): State<AppState>, user: CurrentUser) -> ApiResult<StatusCode> {
    require_instance_admin(&st, &user).await?;
    sqlx::query(
        "UPDATE instance_configurations SET value = CASE WHEN key = 'ENABLE_SMTP' THEN '0' ELSE '' END, updated_at = now()
         WHERE key IN ('ENABLE_SMTP', 'EMAIL_HOST', 'EMAIL_HOST_USER', 'EMAIL_PORT', 'EMAIL_FROM')",
    )
    .execute(&st.db)
    .await?;
    no_content()
}

const ADMIN_JSON: &str = "jsonb_build_object('id', a.id, 'role', a.role, 'is_verified', a.is_verified,
    'instance', a.instance_id, 'user', a.user_id, 'created_at', a.created_at, 'updated_at', a.updated_at,
    'created_by', a.created_by_id, 'updated_by', a.updated_by_id, 'deleted_at', a.deleted_at,
    'user_detail', jsonb_build_object('id', u.id, 'first_name', u.first_name, 'last_name', u.last_name, 'avatar', u.avatar,
        'avatar_url', NULLIF(u.avatar, ''), 'is_bot', u.is_bot, 'display_name', u.display_name, 'email', u.email,
        'last_login_medium', u.last_login_medium))";

async fn admins(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    require_instance_admin(&st, &user).await?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {ADMIN_JSON} FROM instance_admins a JOIN users u ON u.id = a.user_id WHERE a.deleted_at IS NULL ORDER BY a.created_at"
        )),
        &st.db,
    )
    .await
}

async fn add_admin(
    State(st): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    require_instance_admin(&st, &user).await?;
    let Some(email) = s(&b, "email").map(|e| e.trim().to_lowercase()) else {
        return Err(ApiError::bad("Email is required"));
    };
    let role = b.get("role").and_then(Value::as_i64).unwrap_or(20) as i32;
    let target: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&st.db)
        .await?;
    let (target,) = target.ok_or_else(ApiError::not_found)?;
    let v: Value = sqlx::query_scalar(&format!(
        "WITH a AS (INSERT INTO instance_admins (user_id, instance_id, role, is_verified, created_by_id, updated_by_id)
                    SELECT $1, id, $2, false, $3, $3 FROM instances ORDER BY created_at LIMIT 1 RETURNING *)
         SELECT {ADMIN_JSON} FROM a JOIN users u ON u.id = a.user_id"
    ))
    .bind(target)
    .bind(role)
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    created(v)
}

async fn remove_admin(State(st): State<AppState>, user: CurrentUser, Path(pk): Path<Uuid>) -> ApiResult<StatusCode> {
    require_instance_admin(&st, &user).await?;
    let left: (i64,) = sqlx::query_as("SELECT count(*) FROM instance_admins WHERE deleted_at IS NULL AND id <> $1")
        .bind(pk)
        .fetch_one(&st.db)
        .await?;
    if left.0 == 0 {
        return Err(ApiError::bad("At least one instance admin is required"));
    }
    sqlx::query("DELETE FROM instance_admins WHERE id = $1")
        .bind(pk)
        .execute(&st.db)
        .await?;
    no_content()
}

const WS_JSON: &str = "to_jsonb(w) - 'owner_id' - 'created_by_id' - 'updated_by_id' - 'logo_asset_id'
    || jsonb_build_object('owner', w.owner_id, 'created_by', w.created_by_id, 'updated_by', w.updated_by_id,
       'logo_asset', w.logo_asset_id, 'logo_url', NULLIF(w.logo, ''),
       'total_projects', (SELECT count(*) FROM projects p WHERE p.workspace_id = w.id AND p.deleted_at IS NULL),
       'total_members', (SELECT count(*) FROM workspace_members m JOIN users u ON u.id = m.member_id
                         WHERE m.workspace_id = w.id AND m.is_active AND m.deleted_at IS NULL AND NOT u.is_bot))";

/// Paginated workspace list (Plane's paginator envelope, 10 per page).
async fn workspaces(
    State(st): State<AppState>,
    user: CurrentUser,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    require_instance_admin(&st, &user).await?;
    let per_page: i64 = q
        .get("per_page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(10)
        .clamp(1, 10);
    let page: i64 = q
        .get("cursor")
        .and_then(|c| c.split(':').nth(1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(0)
        .max(0);
    let search = q.get("search").cloned().unwrap_or_default();
    let total: (i64,) =
        sqlx::query_as("SELECT count(*) FROM workspaces WHERE deleted_at IS NULL AND name ILIKE '%' || $1 || '%'")
            .bind(&search)
            .fetch_one(&st.db)
            .await?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {WS_JSON} FROM workspaces w WHERE w.deleted_at IS NULL AND w.name ILIKE '%' || $1 || '%'
         ORDER BY w.created_at DESC LIMIT $2 OFFSET $3"
    ))
    .bind(&search)
    .bind(per_page + 1)
    .bind(page * per_page)
    .fetch_all(&st.db)
    .await?;
    let has_next = rows.len() as i64 > per_page;
    let rows: Vec<Value> = rows.into_iter().take(per_page as usize).collect();
    Ok(Json(json!({
        "total_count": total.0, "next_cursor": format!("{per_page}:{}:0", page + 1),
        "prev_cursor": format!("{per_page}:{}:1", page - 1), "next_page_results": has_next,
        "prev_page_results": page > 0, "count": rows.len(), "total_pages": (total.0 + per_page - 1) / per_page,
        "total_results": total.0, "extra_stats": null, "results": rows,
    })))
}

/// Slugs Plane reserves (`RESTRICTED_WORKSPACE_SLUGS`) because they collide with app routes.
pub const RESTRICTED_SLUGS: &[&str] = &[
    "404",
    "accounts",
    "api",
    "create-workspace",
    "god-mode",
    "installations",
    "invitations",
    "onboarding",
    "profile",
    "spaces",
    "workspace-invitations",
    "password",
    "flags",
    "monitor",
    "monitoring",
    "ingest",
    "plane-pro",
    "plane-ultimate",
    "enterprise",
    "plane-enterprise",
    "disco",
    "silo",
    "chat",
    "calendar",
    "drive",
    "channels",
    "upgrade",
    "billing",
    "sign-in",
    "sign-up",
    "signin",
    "signup",
    "config",
    "live",
    "admin",
    "m",
    "import",
    "importers",
    "integrations",
    "integration",
    "configuration",
    "initiatives",
    "initiative",
    "config",
    "workflow",
    "workflows",
    "epics",
    "epic",
    "story",
    "mobile",
    "dashboard",
    "desktop",
    "onload",
    "real-time",
    "one",
    "pages",
    "mobile-login",
    "mobile-logout",
    "mobile-sign-in",
    "mobile-sign-up",
    "mobile-magic-sign-in",
    "mobile-magic-sign-up",
    "wiki",
    "docs",
];

pub fn slug_ok(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 48
        && !RESTRICTED_SLUGS.contains(&slug)
        && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

async fn slug_check(
    State(st): State<AppState>,
    user: CurrentUser,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    require_instance_admin(&st, &user).await?;
    let slug = q.get("slug").cloned().unwrap_or_default();
    if slug.is_empty() {
        return Err(ApiError::bad("Workspace Slug is required"));
    }
    let taken: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM workspaces WHERE slug = $1)")
        .bind(&slug)
        .fetch_one(&st.db)
        .await?;
    Ok(Json(json!({"status": !taken.0 && slug_ok(&slug)})))
}

async fn create_workspace(
    State(st): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    require_instance_admin(&st, &user).await?;
    let (Some(name), Some(slug)) = (s(&b, "name").map(str::trim), s(&b, "slug").map(str::trim)) else {
        return Err(ApiError::bad("Both name and slug are required"));
    };
    if name.is_empty() || slug.is_empty() {
        return Err(ApiError::bad("Both name and slug are required"));
    }
    if name.chars().count() > 80 || slug.len() > 48 {
        return Err(ApiError::bad("The maximum length for name is 80 and for slug is 48"));
    }
    if !slug_ok(slug) {
        return Err(ApiError::bad_body(json!(["Slug is not valid"])));
    }
    let mut tx = st.db.begin().await?;
    let id =
        crate::workspaces::create_workspace(&mut tx, user.id, name, slug, s(&b, "organization_size").unwrap_or(""))
            .await
            .map_err(|e| {
                if e.0 == StatusCode::CONFLICT {
                    ApiError(
                        StatusCode::CONFLICT,
                        json!({"slug": "The workspace with the slug already exists"}),
                    )
                } else {
                    e
                }
            })?;
    tx.commit().await?;
    let v: Value = sqlx::query_scalar(&format!("SELECT {WS_JSON} FROM workspaces w WHERE w.id = $1"))
        .bind(id)
        .fetch_one(&st.db)
        .await?;
    created(v)
}

#[cfg(test)]
mod tests {
    #[test]
    fn slugs() {
        assert!(super::slug_ok("acme-1"));
        assert!(!super::slug_ok("god-mode"));
        assert!(!super::slug_ok("a b"));
        assert!(!super::slug_ok(""));
    }
}
