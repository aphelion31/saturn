//! Shared state, errors, session auth and permission checks. Every handler module uses these.
use axum::{
    Json,
    extract::{FromRef, FromRequestParts},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub cfg: std::sync::Arc<Config>,
}

/// Runtime configuration from the environment (see docs/architectures/02.STRUCTURE-PROJECT.md).
pub struct Config {
    pub secret_key: String,
    /// Public origin, e.g. `https://plane.example.com`. Used for redirects after sign in.
    pub web_url: String,
    pub web_dir: std::path::PathBuf,
    pub storage_dir: std::path::PathBuf,
    pub file_size_limit: i64,
    pub cookie_secure: bool,
}

impl FromRef<AppState> for PgPool {
    fn from_ref(s: &AppState) -> PgPool {
        s.db.clone()
    }
}

/// Uniform error. `?` on sqlx errors gives 500 (404 for RowNotFound).
/// Body shape matches DRF: `{"error": "..."}`, or `{"detail": "..."}` for 401/403 from auth.
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub Value);

impl ApiError {
    pub fn not_found() -> Self {
        Self(
            StatusCode::NOT_FOUND,
            json!({"error": "The requested resource does not exist."}),
        )
    }
    pub fn unauthorized() -> Self {
        Self(
            StatusCode::UNAUTHORIZED,
            json!({"detail": "Authentication credentials were not provided."}),
        )
    }
    pub fn forbidden() -> Self {
        Self(
            StatusCode::FORBIDDEN,
            json!({"detail": "You do not have permission to perform this action."}),
        )
    }
    pub fn bad(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, json!({"error": msg.into()}))
    }
    /// 400 with an arbitrary body, e.g. DRF field errors `{"name": ["This field is required."]}`.
    pub fn bad_body(v: Value) -> Self {
        Self(StatusCode::BAD_REQUEST, v)
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self(StatusCode::CONFLICT, json!({"error": msg.into()}))
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => Self::not_found(),
            sqlx::Error::Database(d) if d.is_unique_violation() => {
                Self::conflict("A resource with these values already exists.")
            }
            e => {
                tracing::error!("db error: {e}");
                Self(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"error": "Something went wrong please try again later"}),
                )
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

pub type ApiResult<T = Json<Value>> = Result<T, ApiError>;

/// 201 Created with a JSON body.
pub fn created(v: Value) -> ApiResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::CREATED, Json(v)))
}

/// 204 No Content.
pub fn no_content() -> ApiResult<StatusCode> {
    Ok(StatusCode::NO_CONTENT)
}

/// Signed-in user. Resolved from the `session-id` cookie (Django `sessions` table, compatible
/// with an existing Plane database), the `admin-session-id` cookie on `/api/instances/*`,
/// or an `X-Api-Key` header (`api_tokens` table) for the public `/api/v1` API.
#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub id: Uuid,
    pub email: String,
    pub session_key: Option<String>,
}

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser
where
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let st = AppState::from_ref(state);
        if let Some(u) = parts.extensions.get::<CurrentUser>() {
            return Ok(u.clone());
        }
        let u = resolve_user(&st, parts).await?.ok_or_else(ApiError::unauthorized)?;
        parts.extensions.insert(u.clone());
        Ok(u)
    }
}

/// Optional variant for endpoints that work signed in or out (public boards, instance info).
pub struct MaybeUser(pub Option<CurrentUser>);

impl<S: Send + Sync> FromRequestParts<S> for MaybeUser
where
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let st = AppState::from_ref(state);
        Ok(MaybeUser(resolve_user(&st, parts).await?))
    }
}

async fn resolve_user(st: &AppState, parts: &Parts) -> Result<Option<CurrentUser>, ApiError> {
    if let Some(key) = parts.headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        let row: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT u.id, COALESCE(u.email,'') FROM api_tokens t JOIN users u ON u.id = t.user_id
             WHERE t.token = $1 AND t.is_active AND t.deleted_at IS NULL AND u.is_active
               AND (t.expired_at IS NULL OR t.expired_at > now())",
        )
        .bind(key)
        .fetch_optional(&st.db)
        .await?;
        return Ok(row.map(|(id, email)| CurrentUser {
            id,
            email,
            session_key: None,
        }));
    }
    let admin = parts.uri.path().contains("instances");
    let name = if admin {
        crate::session::ADMIN_COOKIE
    } else {
        crate::session::COOKIE
    };
    let Some(key) = crate::session::cookie(&parts.headers, name) else {
        return Ok(None);
    };
    let Some(user_id) = crate::session::load(st, &key).await? else {
        return Ok(None);
    };
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, COALESCE(email,'') FROM users WHERE id = $1 AND is_active")
            .bind(user_id)
            .fetch_optional(&st.db)
            .await?;
    Ok(row.map(|(id, email)| CurrentUser {
        id,
        email,
        session_key: Some(key),
    }))
}

/// Plane roles (workspace and project): 20 admin, 15 member, 5 guest.
pub const ADMIN: i16 = 20;
pub const MEMBER: i16 = 15;
pub const GUEST: i16 = 5;

/// Membership of the current user in the workspace from the URL slug.
#[derive(Clone, Debug)]
pub struct WorkspaceCtx {
    pub workspace_id: Uuid,
    pub slug: String,
    pub user_id: Uuid,
    pub role: i16,
}

impl WorkspaceCtx {
    pub fn is_admin(&self) -> bool {
        self.role >= ADMIN
    }
    /// 403 unless the workspace role is in `allowed` (mirrors `allow_permission(level="WORKSPACE")`).
    pub fn require(&self, allowed: &[i16]) -> Result<(), ApiError> {
        if allowed.contains(&self.role) {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }
}

/// Verify `user` is an active member of workspace `slug`. Every `/api/workspaces/{slug}/...`
/// handler calls this first and filters every query by `ws.workspace_id`.
pub async fn require_workspace(db: &PgPool, user: &CurrentUser, slug: &str) -> Result<WorkspaceCtx, ApiError> {
    let row: Option<(Uuid, i16)> = sqlx::query_as(
        "SELECT w.id, m.role FROM workspaces w
         JOIN workspace_members m ON m.workspace_id = w.id AND m.member_id = $2 AND m.is_active AND m.deleted_at IS NULL
         WHERE w.slug = $1 AND w.deleted_at IS NULL",
    )
    .bind(slug)
    .bind(user.id)
    .fetch_optional(db)
    .await?;
    let (workspace_id, role) = row.ok_or_else(ApiError::forbidden)?;
    Ok(WorkspaceCtx {
        workspace_id,
        slug: slug.to_string(),
        user_id: user.id,
        role,
    })
}

/// Membership of the current user in a project.
#[derive(Clone, Debug)]
pub struct ProjectCtx {
    pub ws: WorkspaceCtx,
    pub project_id: Uuid,
    /// Project role (20/15/5).
    pub role: i16,
    /// `projects.guest_view_all_features`: when false, guests only see work items they created.
    pub guest_view_all: bool,
}

impl ProjectCtx {
    pub fn workspace_id(&self) -> Uuid {
        self.ws.workspace_id
    }
    pub fn user_id(&self) -> Uuid {
        self.ws.user_id
    }
    pub fn is_admin(&self) -> bool {
        self.role >= ADMIN
    }
    /// 403 unless the project role is in `allowed`, with Plane's rule that a workspace admin
    /// who is a project member passes any check (mirrors `allow_permission(level="PROJECT")`).
    pub fn require(&self, allowed: &[i16]) -> Result<(), ApiError> {
        if allowed.contains(&self.role) || self.ws.is_admin() {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }
    /// True when the caller is a restricted guest (may only see own work items).
    pub fn restricted_guest(&self) -> bool {
        self.role == GUEST && !self.guest_view_all
    }
}

/// Verify `user` is an active member of `project_id` in workspace `slug`.
pub async fn require_project(
    db: &PgPool,
    user: &CurrentUser,
    slug: &str,
    project_id: Uuid,
) -> Result<ProjectCtx, ApiError> {
    let ws = require_workspace(db, user, slug).await?;
    let row: Option<(i16, bool)> = sqlx::query_as(
        "SELECT pm.role, COALESCE(p.guest_view_all_features, false) FROM project_members pm
         JOIN projects p ON p.id = pm.project_id AND p.deleted_at IS NULL AND p.workspace_id = $3
         WHERE pm.project_id = $1 AND pm.member_id = $2 AND pm.is_active AND pm.deleted_at IS NULL",
    )
    .bind(project_id)
    .bind(user.id)
    .bind(ws.workspace_id)
    .fetch_optional(db)
    .await?;
    let (role, guest_view_all) = row.ok_or_else(ApiError::forbidden)?;
    Ok(ProjectCtx {
        ws,
        project_id,
        role,
        guest_view_all,
    })
}

/// Run a SELECT that returns one json column per row and collect it into a JSON array.
/// Write the row shape in SQL: `SELECT json_build_object(...) FROM ...` or `SELECT to_jsonb(t) FROM t`.
pub async fn json_rows<'q>(
    q: sqlx::query::QueryScalar<'q, sqlx::Postgres, Value, sqlx::postgres::PgArguments>,
    db: &PgPool,
) -> ApiResult {
    Ok(Json(Value::Array(q.fetch_all(db).await?)))
}

/// Run a SELECT returning one json column for one row; 404 when missing.
pub async fn json_row<'q>(
    q: sqlx::query::QueryScalar<'q, sqlx::Postgres, Value, sqlx::postgres::PgArguments>,
    db: &PgPool,
) -> ApiResult {
    Ok(Json(q.fetch_optional(db).await?.ok_or_else(ApiError::not_found)?))
}

/// Read a string field from a JSON body.
pub fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// Read a uuid field from a JSON body (null or missing gives None).
pub fn uuid_of(v: &Value, k: &str) -> Option<Uuid> {
    s(v, k).and_then(|x| Uuid::parse_str(x).ok())
}

/// Strip HTML tags, like Django's `strip_tags`, for `description_stripped` columns.
pub fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Sanitize rich-text HTML before storing it, like Plane's `validate_html_content` (nh3 with the
/// editor's tags and attributes). Use for every `description_html` / `comment_html` write.
/// Returns 400 when the HTML is larger than 10 MB.
pub fn sanitize_html(html: &str) -> Result<String, ApiError> {
    use std::collections::{HashMap, HashSet};
    use std::sync::OnceLock;
    if html.len() > 10 * 1024 * 1024 {
        return Err(ApiError::bad("HTML content exceeds maximum size limit (10MB)"));
    }
    static CLEANER: OnceLock<ammonia::Builder<'static>> = OnceLock::new();
    let cleaner = CLEANER.get_or_init(|| {
        let mut b = ammonia::Builder::default();
        b.add_tags(["mention-component", "label", "input", "image-component"]);
        b.add_generic_attributes([
            "class",
            "id",
            "title",
            "role",
            "aria-label",
            "aria-hidden",
            "style",
            "start",
            "type",
            "xmlns",
            "data-tight",
            "data-node-type",
            "data-type",
            "data-checked",
            "data-background-color",
            "data-text-color",
            "data-name",
            "data-id",
            "data-icon-name",
            "data-icon-color",
            "data-background",
            "data-emoji-unicode",
            "data-emoji-url",
            "data-logo-in-use",
            "data-block-type",
        ]);
        let tag_attrs: HashMap<&'static str, HashSet<&'static str>> = [
            ("a", vec!["href", "target"]),
            (
                "image-component",
                vec!["id", "width", "height", "aspectratio", "src", "alignment", "status"],
            ),
            (
                "img",
                vec!["width", "height", "aspectratio", "alignment", "src", "alt", "title"],
            ),
            ("mention-component", vec!["id", "entity_identifier", "entity_name"]),
            ("th", vec!["colspan", "rowspan", "colwidth", "background", "style"]),
            (
                "td",
                vec!["colspan", "rowspan", "colwidth", "background", "textcolor", "style"],
            ),
            ("tr", vec!["background", "textcolor", "style"]),
            ("pre", vec!["language"]),
            ("code", vec!["language", "spellcheck"]),
            ("input", vec!["type", "checked"]),
        ]
        .into_iter()
        .map(|(t, a)| (t, a.into_iter().collect()))
        .collect();
        b.tag_attributes(tag_attrs);
        b.url_schemes(["http", "https", "mailto", "tel"].into_iter().collect());
        b.link_rel(None);
        b
    });
    Ok(cleaner.clean(html).to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn strip_tags_works() {
        assert_eq!(super::strip_tags("<p>hi <b>there</b></p>"), "hi there");
    }

    #[test]
    fn sanitize_keeps_editor_markup_and_drops_scripts() {
        let out = super::sanitize_html(
            r#"<p class="x" onclick="evil()">hi<script>alert(1)</script></p><a href="javascript:x">l</a><mention-component entity_identifier="u1" entity_name="user_mention"></mention-component><img src="https://a/b.png" onerror="x">"#,
        )
        .unwrap();
        assert!(out.contains(r#"<p class="x">hi</p>"#), "{out}");
        assert!(
            !out.contains("script")
                && !out.contains("onclick")
                && !out.contains("onerror")
                && !out.contains("javascript")
        );
        assert!(
            out.contains(r#"<mention-component entity_identifier="u1" entity_name="user_mention">"#),
            "{out}"
        );
    }
}
