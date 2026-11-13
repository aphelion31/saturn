//! File uploads (Plane's `assets/v2`), stored on local disk under `STORAGE_DIR` instead of S3/MinIO.
//!
//! Plane's browser flow is kept: POST creates a `file_assets` row and returns `upload_data`
//! (`url` + `fields`), the browser POSTs the file as multipart to `url` with those fields, then PATCHes
//! the asset to mark it uploaded. Here `url` is Saturn's own upload endpoint and the only field is an
//! HMAC token bound to the asset id, so nobody can write to an asset they did not create.
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, FromRequest, Multipart, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use uuid::Uuid;

use crate::common::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        // Legacy file-assets
        .route(
            "/api/users/file-assets/",
            get(legacy_list_user_assets).post(legacy_upload_user_asset),
        )
        .route(
            "/api/users/file-assets/{pk}/",
            get(legacy_get_user_asset).delete(legacy_delete_user_asset),
        )
        .route(
            "/api/workspaces/file-assets/{workspace_id}/{asset_key}/",
            get(legacy_get_workspace_asset).delete(legacy_delete_workspace_asset),
        )
        .route(
            "/api/workspaces/file-assets/{workspace_id}/{asset_key}",
            get(legacy_get_workspace_asset).delete(legacy_delete_workspace_asset),
        )
        .route(
            "/api/workspaces/file-assets/{workspace_id}/{asset_key}/restore/",
            post(legacy_restore_workspace_asset),
        )
        .route(
            "/api/workspaces/{slug}/file-assets/",
            get(legacy_list_workspace_assets).post(legacy_create_workspace_asset),
        )
        .route("/api/assets/v2/workspaces/{slug}/check/{asset_id}/", get(check_asset))
        .route(
            "/api/assets/v2/workspaces/{slug}/download/{asset_id}/",
            get(download_workspace_forced),
        )
        .route(
            "/api/assets/v2/workspaces/{slug}/projects/{project_id}/download/{asset_id}/",
            get(download_project_forced),
        )
        .route(
            "/api/assets/v2/workspaces/{slug}/restore/{asset_id}/",
            post(restore_asset),
        )
        .route(
            "/api/assets/v2/workspaces/{slug}/duplicate-assets/{asset_id}/",
            post(duplicate_asset),
        )
        .route(
            "/api/assets/v2/workspaces/{slug}/projects/{project_id}/{entity_id}/bulk/",
            post(bulk_assets),
        )
        .route("/api/assets/v2/workspaces/{slug}/", post(create_workspace_asset))
        .route(
            "/api/assets/v2/workspaces/{slug}/{asset_id}/",
            get(download_workspace_asset).patch(mark_uploaded).delete(delete_asset),
        )
        .route(
            "/api/assets/v2/workspaces/{slug}/projects/{project_id}/",
            post(create_project_asset),
        )
        .route(
            "/api/assets/v2/workspaces/{slug}/projects/{project_id}/{asset_id}/",
            get(download_project_asset)
                .patch(mark_project_uploaded)
                .delete(delete_project_asset),
        )
        .route("/api/assets/v2/user-assets/", post(create_user_asset))
        .route(
            "/api/assets/v2/user-assets/{asset_id}/",
            axum::routing::patch(mark_user_uploaded).delete(delete_user_asset),
        )
        .route("/api/assets/v2/static/{asset_id}/", get(static_asset))
        .route(
            "/api/assets/v2/upload/{asset_id}/",
            // The multipart body is capped again against FILE_SIZE_LIMIT inside the handler.
            post(upload).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route(
            "/api/assets/v2/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/attachments/",
            get(list_attachments).post(create_attachment),
        )
        .route(
            "/api/assets/v2/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/attachments/{asset_id}/",
            get(download_attachment)
                .patch(mark_attachment_uploaded)
                .delete(delete_attachment),
        )
}

const ENTITY_TYPES: &[&str] = &[
    "ISSUE_ATTACHMENT",
    "ISSUE_DESCRIPTION",
    "COMMENT_DESCRIPTION",
    "PAGE_DESCRIPTION",
    "USER_COVER",
    "USER_AVATAR",
    "WORKSPACE_LOGO",
    "PROJECT_COVER",
    "DRAFT_ISSUE_ATTACHMENT",
    "DRAFT_ISSUE_DESCRIPTION",
];
pub(crate) const IMAGE_TYPES: &[&str] = &["image/jpeg", "image/png", "image/webp", "image/jpg", "image/gif"];
/// Entity types readable through the public static endpoint (avatars, covers, logos), as in Plane.
const STATIC_TYPES: &[&str] = &["USER_AVATAR", "USER_COVER", "WORKSPACE_LOGO", "PROJECT_COVER"];

/// Plane's `sanitize_filename`: no directories, no control characters, no traversal.
pub(crate) fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').replace("..", "_");
    if cleaned.is_empty() {
        "unnamed".into()
    } else {
        cleaned.chars().take(200).collect()
    }
}

pub(crate) fn token(secret: &str, asset_id: Uuid) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    m.update(b"saturn-asset-upload:");
    m.update(asset_id.as_bytes());
    hex::encode(m.finalize().into_bytes())
}

pub(crate) fn upload_response(asset_id: Uuid, secret: &str) -> Value {
    json!({
        "upload_data": {
            "url": format!("/api/assets/v2/upload/{asset_id}/"),
            "fields": {"token": token(secret, asset_id)},
        },
        "asset_id": asset_id,
        "asset_url": format!("/api/assets/v2/static/{asset_id}/"),
    })
}

struct NewAsset<'a> {
    workspace_id: Uuid,
    user_id: Uuid,
    entity_type: &'a str,
    project_id: Option<Uuid>,
    issue_id: Option<Uuid>,
    target_user_id: Option<Uuid>,
    page_id: Option<Uuid>,
    comment_id: Option<Uuid>,
    images_only: bool,
}

/// Validate the body and insert the `file_assets` row. Returns Plane's presign response.
async fn insert_asset(st: &AppState, a: NewAsset<'_>, b: &Value) -> ApiResult {
    if !ENTITY_TYPES.contains(&a.entity_type) {
        return Err(ApiError::bad_body(
            json!({"error": "Invalid entity type.", "status": false}),
        ));
    }
    let name = sanitize_filename(s(b, "name").unwrap_or("unnamed"));
    let raw_typ = s(b, "type").unwrap_or("").trim();
    let typ = if raw_typ.is_empty() {
        if a.images_only {
            "image/jpeg"
        } else {
            "application/octet-stream"
        }
    } else {
        raw_typ
    };
    if a.images_only && !IMAGE_TYPES.contains(&typ) {
        return Err(ApiError::bad_body(json!({
            "error": "Invalid file type. Only JPEG, PNG, WebP, JPG and GIF files are allowed.", "status": false
        })));
    }
    let size = b
        .get("size")
        .and_then(Value::as_f64)
        .unwrap_or(st.cfg.file_size_limit as f64);
    let size = size.min(st.cfg.file_size_limit as f64);
    let id = Uuid::new_v4();
    let key = format!("{}/{}-{}", a.workspace_id, id.simple(), name);
    sqlx::query(
        "INSERT INTO file_assets (id, attributes, asset, size, workspace_id, created_by_id, updated_by_id, entity_type,
                                  entity_identifier, project_id, issue_id, user_id, page_id, comment_id, is_uploaded)
         VALUES ($1, $2, $3, $4, $5, $6, $6, $7, $8, $9, $10, $11, $12, $13, false)",
    )
    .bind(id)
    .bind(json!({"name": name, "type": typ, "size": size}))
    .bind(&key)
    .bind(size)
    .bind(a.workspace_id)
    .bind(a.user_id)
    .bind(a.entity_type)
    .bind(s(b, "entity_identifier"))
    .bind(a.project_id)
    .bind(a.issue_id)
    .bind(a.target_user_id)
    .bind(a.page_id)
    .bind(a.comment_id)
    .execute(&st.db)
    .await?;
    Ok(Json(upload_response(id, &st.cfg.secret_key)))
}

fn entity_ids(entity_type: &str, b: &Value) -> (Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>) {
    let id = uuid_of(b, "entity_identifier");
    match entity_type {
        "ISSUE_ATTACHMENT" | "ISSUE_DESCRIPTION" => (id, None, None, None),
        "USER_AVATAR" | "USER_COVER" => (None, id, None, None),
        "PAGE_DESCRIPTION" => (None, None, id, None),
        "COMMENT_DESCRIPTION" => (None, None, None, id),
        _ => (None, None, None, None),
    }
}

async fn create_workspace_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let entity_type = s(&b, "entity_type").unwrap_or("");
    if entity_type == "WORKSPACE_LOGO" && !ws.is_admin() {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            json!({"error": "Only workspace admins can upload a workspace logo."}),
        ));
    }
    let (issue_id, target_user_id, page_id, comment_id) = entity_ids(entity_type, &b);
    let project_id = if entity_type == "PROJECT_COVER" {
        uuid_of(&b, "entity_identifier")
    } else if let Some(pid) = uuid_of(&b, "project_id") {
        Some(pid)
    } else if let Some(iid) = issue_id {
        sqlx::query_scalar("SELECT project_id FROM issues WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL")
            .bind(iid)
            .bind(ws.workspace_id)
            .fetch_optional(&st.db)
            .await?
    } else if let Some(cid) = comment_id {
        sqlx::query_scalar(
            "SELECT project_id FROM issue_comments WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
        )
        .bind(cid)
        .bind(ws.workspace_id)
        .fetch_optional(&st.db)
        .await?
    } else if let Some(pgid) = page_id {
        sqlx::query_scalar("SELECT project_id FROM project_pages WHERE page_id = $1 AND workspace_id = $2 AND deleted_at IS NULL LIMIT 1")
            .bind(pgid)
            .bind(ws.workspace_id)
            .fetch_optional(&st.db)
            .await?
    } else {
        None
    };
    insert_asset(
        &st,
        NewAsset {
            workspace_id: ws.workspace_id,
            user_id: user.id,
            entity_type,
            project_id,
            issue_id,
            target_user_id,
            page_id,
            comment_id,
            images_only: true,
        },
        &b,
    )
    .await
}

async fn create_project_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let entity_type = s(&b, "entity_type").unwrap_or("");
    let (issue_id, _, page_id, comment_id) = entity_ids(entity_type, &b);
    insert_asset(
        &st,
        NewAsset {
            workspace_id: p.workspace_id(),
            user_id: user.id,
            entity_type,
            project_id: Some(project_id),
            issue_id,
            target_user_id: None,
            page_id,
            comment_id,
            images_only: false,
        },
        &b,
    )
    .await
}

pub(crate) async fn create_user_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Value>,
) -> ApiResult {
    let entity_type = s(&b, "entity_type").unwrap_or("");
    if !matches!(entity_type, "USER_AVATAR" | "USER_COVER") {
        return Err(ApiError::bad_body(
            json!({"error": "Invalid entity type.", "status": false}),
        ));
    }
    // User assets are not tied to a workspace in Plane; file them under the user id on disk.
    let name = sanitize_filename(s(&b, "name").unwrap_or("unnamed"));
    let typ = s(&b, "type").unwrap_or("image/jpeg");
    if !IMAGE_TYPES.contains(&typ) {
        return Err(ApiError::bad_body(json!({
            "error": "Invalid file type. Only JPEG, PNG, WebP, JPG and GIF files are allowed.", "status": false
        })));
    }
    let size = b
        .get("size")
        .and_then(Value::as_f64)
        .unwrap_or(st.cfg.file_size_limit as f64)
        .min(st.cfg.file_size_limit as f64);
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO file_assets (id, attributes, asset, size, user_id, created_by_id, updated_by_id, entity_type, is_uploaded)
         VALUES ($1, $2, $3, $4, $5, $5, $5, $6, false)",
    )
    .bind(id)
    .bind(json!({"name": name, "type": typ, "size": size}))
    .bind(format!("users/{}/{}-{}", user.id, id.simple(), name))
    .bind(size)
    .bind(user.id)
    .bind(entity_type)
    .execute(&st.db)
    .await?;
    Ok(Json(upload_response(id, &st.cfg.secret_key)))
}

/// Multipart upload target. Verifies the HMAC token, enforces `FILE_SIZE_LIMIT`, writes the file
/// to `STORAGE_DIR/<asset key from the database>`. The client never chooses the path.
async fn upload(State(st): State<AppState>, Path(asset_id): Path<Uuid>, mut form: Multipart) -> ApiResult<StatusCode> {
    let row: Option<(String, bool)> =
        sqlx::query_as("SELECT asset, is_uploaded FROM file_assets WHERE id = $1 AND deleted_at IS NULL")
            .bind(asset_id)
            .fetch_optional(&st.db)
            .await?;
    let (key, uploaded) = row.ok_or_else(ApiError::not_found)?;
    if uploaded {
        return Err(ApiError::bad("Asset already uploaded"));
    }
    let want = token(&st.cfg.secret_key, asset_id);
    let mut token_ok = false;
    let mut data: Option<Vec<u8>> = None;
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|_| ApiError::bad("Invalid multipart body"))?
    {
        match field.name() {
            Some("token") => {
                let got = field.text().await.map_err(|_| ApiError::bad("Invalid token field"))?;
                token_ok =
                    got.len() == want.len() && got.bytes().zip(want.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0;
            }
            Some("file") => {
                if !token_ok {
                    return Err(ApiError::forbidden());
                }
                let mut buf = Vec::new();
                let mut field = field;
                while let Some(chunk) = field.chunk().await.map_err(|_| ApiError::bad("Upload interrupted"))? {
                    if buf.len() + chunk.len() > st.cfg.file_size_limit as usize {
                        return Err(ApiError::bad("File size exceeds the allowed limit"));
                    }
                    buf.extend_from_slice(&chunk);
                }
                data = Some(buf);
            }
            _ => {}
        }
    }
    if !token_ok {
        return Err(ApiError::forbidden());
    }
    let data = data.ok_or_else(|| ApiError::bad("Missing file field"))?;
    let path = disk_path(&st, &key)?;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io_err)?;
    }
    tokio::fs::write(&path, &data).await.map_err(io_err)?;
    sqlx::query("UPDATE file_assets SET size = $2, updated_at = now() WHERE id = $1")
        .bind(asset_id)
        .bind(data.len() as f64)
        .execute(&st.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Plane `IssueAttachmentEndpoint.post` (legacy multipart upload): the `asset` file field is stored
/// directly, `attributes` may carry name/type/size. Same visibility as reading the issue (restricted
/// guests only on their own issues), size capped at FILE_SIZE_LIMIT, 201 with the row.
pub(crate) async fn legacy_attachment_upload(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue)): Path<(String, Uuid, Uuid)>,
    mut form: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let p = crate::issues::visible_issue(&st, &user, &slug, project, issue).await?;
    let mut attrs = json!({});
    let mut file: Option<(String, Vec<u8>)> = None;
    while let Some(mut field) = form
        .next_field()
        .await
        .map_err(|_| ApiError::bad("Invalid multipart body"))?
    {
        match field.name() {
            Some("attributes") => {
                let raw = field.text().await.map_err(|_| ApiError::bad("Invalid attributes"))?;
                attrs = serde_json::from_str(&raw)
                    .ok()
                    .filter(Value::is_object)
                    .ok_or_else(|| ApiError::bad_body(json!({"attributes": ["Value must be valid JSON."]})))?;
            }
            Some("asset") => {
                let name = sanitize_filename(field.file_name().unwrap_or(""));
                let mut buf = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(|_| ApiError::bad("Upload interrupted"))? {
                    if buf.len() + chunk.len() > st.cfg.file_size_limit as usize {
                        return Err(ApiError::bad_body(
                            json!({"asset": ["File too large. Size should not exceed 5 MB."]}),
                        ));
                    }
                    buf.extend_from_slice(&chunk);
                }
                file = Some((name, buf));
            }
            _ => {}
        }
    }
    let (name, data) = file.ok_or_else(|| ApiError::bad_body(json!({"asset": ["No file was submitted."]})))?;
    let name = if name.is_empty() {
        Uuid::new_v4().simple().to_string()
    } else {
        name
    };
    let key = format!("{}/{}-{}", p.workspace_id(), Uuid::new_v4().simple(), name);
    let path = disk_path(&st, &key)?;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io_err)?;
    }
    tokio::fs::write(&path, &data).await.map_err(io_err)?;
    let mut tx = st.db.begin().await?;
    let asset_id = Uuid::new_v4();
    let row: Value = sqlx::query_scalar(
        "INSERT INTO file_assets (id, attributes, asset, size, workspace_id, project_id, issue_id, created_by_id,
           updated_by_id, entity_type, is_uploaded, is_deleted, is_archived, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, 'ISSUE_ATTACHMENT', true, false, false, now(), now())
         RETURNING (to_jsonb(file_assets) - 'workspace_id' - 'project_id' - 'issue_id' - 'created_by_id' - 'updated_by_id')
           || jsonb_build_object('workspace', workspace_id, 'project', project_id, 'issue', issue_id,
                'created_by', created_by_id, 'updated_by', updated_by_id,
                'asset_url', '/api/assets/v2/workspaces/' || $9 || '/projects/' || project_id || '/issues/' || issue_id || '/attachments/' || id || '/')",
    )
    .bind(asset_id)
    .bind(&attrs)
    .bind(&key)
    .bind(data.len() as f64)
    .bind(p.workspace_id())
    .bind(project)
    .bind(issue)
    .bind(user.id)
    .bind(&slug)
    .fetch_one(&mut *tx)
    .await?;
    let mut event = crate::activity::ActivityEvent::new(
        issue,
        project,
        p.workspace_id(),
        user.id,
        "created",
        "created an attachment",
    );
    event.field = Some("attachment");
    event.new_value = Some(&key);
    event.new_identifier = Some(asset_id);
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

fn io_err(e: std::io::Error) -> ApiError {
    tracing::error!("storage error: {e}");
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "Storage error"}))
}

/// Resolve a stored key under STORAGE_DIR, refusing anything that could escape it.
/// Keys are server-generated (`{workspace}/{uuid}-{name}`); the lexical check blocks `..` and absolute
/// paths, and an existing file must also canonicalize inside STORAGE_DIR, so a symlink can't leak out.
pub(crate) fn disk_path(st: &AppState, key: &str) -> Result<std::path::PathBuf, ApiError> {
    if key.contains("..") || key.starts_with('/') || key.contains('\\') {
        return Err(ApiError::bad("Invalid asset key"));
    }
    let path = st.cfg.storage_dir.join(key);
    if let (Ok(real), Ok(root)) = (path.canonicalize(), st.cfg.storage_dir.canonicalize()) {
        if !real.starts_with(&root) {
            return Err(ApiError::not_found());
        }
    }
    Ok(path)
}

/// Stream a stored file. Images from the allowlist render inline; anything else downloads, so
/// uploaded HTML or SVG can never execute on Saturn's origin.
pub(crate) async fn serve(st: &AppState, key: &str, attrs: &Value, force_download: bool) -> Result<Response, ApiError> {
    let bytes = tokio::fs::read(disk_path(st, key)?)
        .await
        .map_err(|_| ApiError::not_found())?;
    let typ = attrs.get("type").and_then(Value::as_str).unwrap_or("");
    let name = sanitize_filename(attrs.get("name").and_then(Value::as_str).unwrap_or("file"));
    let inline = !force_download && IMAGE_TYPES.contains(&typ);
    let mut res = Body::from(bytes).into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(if inline { typ } else { "application/octet-stream" })
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    let disp = format!(
        "{}; filename=\"{}\"",
        if inline { "inline" } else { "attachment" },
        name.replace('"', "")
    );
    if let Ok(v) = HeaderValue::from_str(&disp) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=3600"));
    Ok(res)
}

/// Avatars, covers and logos. Plane serves these to anyone who has the link; Saturn requires a
/// signed-in user, which every page that shows them already has.
async fn static_asset(
    State(st): State<AppState>,
    _user: CurrentUser,
    Path(asset_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let row: Option<(String, Value, String)> = sqlx::query_as(
        "SELECT asset, attributes, entity_type FROM file_assets WHERE id = $1 AND is_uploaded AND deleted_at IS NULL",
    )
    .bind(asset_id)
    .fetch_optional(&st.db)
    .await?;
    let (key, attrs, entity_type) = row.ok_or_else(ApiError::not_found)?;
    if !STATIC_TYPES.contains(&entity_type.as_str()) {
        return Err(ApiError::bad_body(
            json!({"error": "Invalid entity type.", "status": false}),
        ));
    }
    serve(&st, &key, &attrs, false).await
}

/// Load an asset in this workspace the caller may modify (creator, or workspace admin).
async fn owned_asset(
    st: &AppState,
    ws: &WorkspaceCtx,
    asset_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<(String, String, Uuid), ApiError> {
    let row: Option<(String, String, Option<Uuid>)> = sqlx::query_as(
        "SELECT asset, entity_type, created_by_id FROM file_assets
         WHERE id = $1 AND workspace_id = $2 AND ($3::uuid IS NULL OR project_id = $3) AND deleted_at IS NULL",
    )
    .bind(asset_id)
    .bind(ws.workspace_id)
    .bind(project_id)
    .fetch_optional(&st.db)
    .await?;
    let (key, entity_type, creator) = row.ok_or_else(ApiError::not_found)?;
    if creator != Some(ws.user_id) && !ws.is_admin() {
        return Err(ApiError::forbidden());
    }
    Ok((key, entity_type, asset_id))
}

async fn set_uploaded(st: &AppState, asset_id: Uuid, entity_type: &str, b: &Value) -> Result<(), ApiError> {
    sqlx::query("UPDATE file_assets SET is_uploaded = true, attributes = COALESCE($2, attributes), updated_at = now() WHERE id = $1")
        .bind(asset_id)
        .bind(b.get("attributes"))
        .execute(&st.db)
        .await?;
    crate::side_effects::spawn_asset_storage_metadata(st.clone(), asset_id);
    // Point the owning entity at the new asset, as Plane's entity_asset_save does.
    let sql = match entity_type {
        "WORKSPACE_LOGO" => {
            "UPDATE workspaces w SET logo_asset_id = f.id, logo = '' FROM file_assets f WHERE f.id = $1 AND w.id = f.workspace_id"
        }
        "PROJECT_COVER" => {
            "UPDATE projects p SET cover_image_asset_id = f.id, cover_image = '' FROM file_assets f WHERE f.id = $1 AND p.id = f.project_id"
        }
        "USER_AVATAR" => {
            "UPDATE users u SET avatar_asset_id = f.id, avatar = '' FROM file_assets f WHERE f.id = $1 AND u.id = f.user_id"
        }
        "USER_COVER" => {
            "UPDATE users u SET cover_image_asset_id = f.id, cover_image = NULL FROM file_assets f WHERE f.id = $1 AND u.id = f.user_id"
        }
        _ => return Ok(()),
    };
    sqlx::query(sql).bind(asset_id).execute(&st.db).await?;
    Ok(())
}

async fn soft_delete(st: &AppState, asset_id: Uuid, entity_type: &str) -> Result<(), ApiError> {
    sqlx::query("UPDATE file_assets SET is_deleted = true, deleted_at = now() WHERE id = $1")
        .bind(asset_id)
        .execute(&st.db)
        .await?;
    let sql = match entity_type {
        "WORKSPACE_LOGO" => "UPDATE workspaces SET logo_asset_id = NULL WHERE logo_asset_id = $1",
        "PROJECT_COVER" => "UPDATE projects SET cover_image_asset_id = NULL WHERE cover_image_asset_id = $1",
        "USER_AVATAR" => "UPDATE users SET avatar_asset_id = NULL WHERE avatar_asset_id = $1",
        "USER_COVER" => "UPDATE users SET cover_image_asset_id = NULL WHERE cover_image_asset_id = $1",
        _ => return Ok(()),
    };
    sqlx::query(sql).bind(asset_id).execute(&st.db).await?;
    Ok(())
}

async fn mark_uploaded(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, asset_id)): Path<(String, Uuid)>,
    body: Option<Json<Value>>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let (_, entity_type, id) = owned_asset(&st, &ws, asset_id, None).await?;
    set_uploaded(&st, id, &entity_type, &body.map(|b| b.0).unwrap_or_default()).await?;
    no_content()
}

async fn delete_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, asset_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let (_, entity_type, id) = owned_asset(&st, &ws, asset_id, None).await?;
    soft_delete(&st, id, &entity_type).await?;
    no_content()
}

/// Members of the workspace may read its assets; project-scoped assets need project membership.
async fn readable_asset(
    st: &AppState,
    user: &CurrentUser,
    slug: &str,
    asset_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<(String, Value), ApiError> {
    let ws = require_workspace(&st.db, user, slug).await?;
    let row: Option<(String, Value, Option<Uuid>)> = sqlx::query_as(
        "SELECT asset, attributes, project_id FROM file_assets
         WHERE id = $1 AND workspace_id = $2 AND ($3::uuid IS NULL OR project_id = $3) AND is_uploaded AND deleted_at IS NULL",
    )
    .bind(asset_id)
    .bind(ws.workspace_id)
    .bind(project_id)
    .fetch_optional(&st.db)
    .await?;
    let (key, attrs, pid) = row.ok_or_else(ApiError::not_found)?;
    if let Some(pid) = pid {
        require_project(&st.db, user, slug, pid).await?;
    }
    Ok((key, attrs))
}

async fn download_workspace_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, asset_id)): Path<(String, Uuid)>,
) -> Result<Response, ApiError> {
    let (key, attrs) = readable_asset(&st, &user, &slug, asset_id, None).await?;
    serve(&st, &key, &attrs, false).await
}

async fn download_project_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, asset_id)): Path<(String, Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let (key, attrs) = readable_asset(&st, &user, &slug, asset_id, Some(project_id)).await?;
    serve(&st, &key, &attrs, false).await
}

async fn download_workspace_forced(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> Result<Response, ApiError> {
    let (key, attrs) = readable_asset(&st, &user, &slug, id, None).await?;
    serve(&st, &key, &attrs, true).await
}

async fn download_project_forced(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<(String, Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let (key, attrs) = readable_asset(&st, &user, &slug, id, Some(project)).await?;
    serve(&st, &key, &attrs, true).await
}

async fn check_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let row: Option<(Option<Uuid>,)> =
        sqlx::query_as("SELECT project_id FROM file_assets WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL")
            .bind(id)
            .bind(ws.workspace_id)
            .fetch_optional(&st.db)
            .await?;
    let exists = match row {
        None => false,
        Some((None,)) => true,
        Some((Some(project),)) => require_project(&st.db, &user, &slug, project).await.is_ok(),
    };
    Ok(Json(json!({"exists": exists})))
}

async fn restore_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let row: Option<(Option<Uuid>, Option<Uuid>)> =
        sqlx::query_as("SELECT created_by_id, project_id FROM file_assets WHERE id = $1 AND workspace_id = $2")
            .bind(id)
            .bind(ws.workspace_id)
            .fetch_optional(&st.db)
            .await?;
    let (creator, project) = row.ok_or_else(ApiError::not_found)?;
    if let Some(project) = project {
        require_project(&st.db, &user, &slug, project).await?;
    }
    if creator != Some(user.id) && !ws.is_admin() {
        return Err(ApiError::forbidden());
    }
    sqlx::query("UPDATE file_assets SET is_deleted = false, deleted_at = NULL WHERE id = $1 AND workspace_id = $2")
        .bind(id)
        .bind(ws.workspace_id)
        .execute(&st.db)
        .await?;
    no_content()
}

async fn duplicate_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let entity_type = s(&b, "entity_type").unwrap_or("");
    if !ENTITY_TYPES.contains(&entity_type) {
        return Err(ApiError::bad("Invalid entity type or entity id"));
    }
    let (key, attrs) = readable_asset(&st, &user, &slug, id, None).await?;
    let source_project: Option<Uuid> =
        sqlx::query_scalar("SELECT project_id FROM file_assets WHERE id = $1 AND workspace_id = $2")
            .bind(id)
            .bind(ws.workspace_id)
            .fetch_one(&st.db)
            .await?;
    let entity = uuid_of(&b, "entity_id");
    let project = if entity_type == "PROJECT_COVER" {
        entity
    } else {
        uuid_of(&b, "project_id").or(source_project)
    };
    if let Some(project) = project {
        require_project(&st.db, &user, &slug, project).await?;
    }
    if matches!(entity_type, "USER_AVATAR" | "USER_COVER") && entity != Some(user.id) {
        return Err(ApiError::forbidden());
    }
    if let Some(entity) = entity {
        let sql = match entity_type {
            "ISSUE_ATTACHMENT" | "ISSUE_DESCRIPTION" => Some(
                "SELECT EXISTS (SELECT 1 FROM issues WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL)",
            ),
            "COMMENT_DESCRIPTION" => Some(
                "SELECT EXISTS (SELECT 1 FROM issue_comments WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL)",
            ),
            "PAGE_DESCRIPTION" => Some(
                "SELECT EXISTS (SELECT 1 FROM project_pages WHERE page_id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL)",
            ),
            _ => None,
        };
        if let Some(sql) = sql {
            let valid: bool = sqlx::query_scalar(sql)
                .bind(entity)
                .bind(project)
                .bind(ws.workspace_id)
                .fetch_one(&st.db)
                .await?;
            if !valid {
                return Err(ApiError::not_found());
            }
        }
    }
    let new_id = Uuid::new_v4();
    let name = sanitize_filename(attrs.get("name").and_then(Value::as_str).unwrap_or("unnamed"));
    let new_key = format!("{}/{}-{}", ws.workspace_id, new_id.simple(), name);
    let dest = disk_path(&st, &new_key)?;
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io_err)?;
    }
    tokio::fs::copy(disk_path(&st, &key)?, &dest).await.map_err(io_err)?;
    let result = sqlx::query("INSERT INTO file_assets (id, attributes, asset, size, workspace_id, created_by_id, updated_by_id, entity_type, project_id, issue_id, page_id, comment_id, user_id, is_uploaded) SELECT $1, attributes, $2, size, workspace_id, $3, $3, $4, $5, $6, $7, $8, $9, true FROM file_assets WHERE id = $10 AND workspace_id = $11 AND deleted_at IS NULL")
        .bind(new_id).bind(&new_key).bind(user.id).bind(entity_type).bind(project)
        .bind(if matches!(entity_type, "ISSUE_ATTACHMENT" | "ISSUE_DESCRIPTION") { entity } else { None })
        .bind(if entity_type == "PAGE_DESCRIPTION" { entity } else { None })
        .bind(if entity_type == "COMMENT_DESCRIPTION" { entity } else { None })
        .bind(if matches!(entity_type, "USER_AVATAR" | "USER_COVER") { entity } else { None })
        .bind(id).bind(ws.workspace_id).execute(&st.db).await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(e.into());
    }
    Ok(Json(json!({"asset_id": new_id})))
}

async fn bulk_assets(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, entity)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project).await?;
    let ids: Vec<Uuid> = b
        .get("asset_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad("No asset ids provided."))?
        .iter()
        .map(|v| {
            v.as_str()
                .and_then(|s| {
                    let clean = s.split('?').next().unwrap_or(s).trim_end_matches('/');
                    let last = clean.split('/').last().unwrap_or(clean);
                    Uuid::parse_str(last).ok()
                })
                .ok_or_else(|| ApiError::bad("Invalid asset id"))
        })
        .collect::<Result<_, _>>()?;
    if ids.is_empty() {
        return Err(ApiError::bad("No asset ids provided."));
    }
    let typ: Option<String> = sqlx::query_scalar("SELECT entity_type FROM file_assets WHERE id = ANY($1) AND workspace_id = $2 AND created_by_id = $3 AND (project_id IS NULL OR project_id = $4) AND deleted_at IS NULL ORDER BY created_at LIMIT 1")
        .bind(&ids).bind(p.workspace_id()).bind(user.id).bind(project).fetch_optional(&st.db).await?;
    let typ = typ.ok_or_else(ApiError::not_found)?;
    if typ == "PROJECT_COVER" {
        p.require(&[ADMIN])?;
        let id: Uuid = sqlx::query_scalar("UPDATE file_assets SET project_id = $4 WHERE id = (SELECT id FROM file_assets WHERE id = ANY($1) AND workspace_id = $2 AND created_by_id = $3 AND (project_id IS NULL OR project_id = $4) AND deleted_at IS NULL ORDER BY created_at LIMIT 1) RETURNING id")
            .bind(&ids).bind(p.workspace_id()).bind(user.id).bind(project).fetch_one(&st.db).await?;
        sqlx::query(
            "UPDATE projects SET cover_image_asset_id = $1 WHERE id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(project)
        .bind(p.workspace_id())
        .execute(&st.db)
        .await?;
        return no_content();
    }
    let sql = match typ.as_str() {
        "ISSUE_DESCRIPTION" => {
            "UPDATE file_assets SET issue_id = $5, project_id = $4 WHERE id = ANY($1) AND workspace_id = $2 AND created_by_id = $3 AND (project_id IS NULL OR project_id = $4) AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM issues WHERE id = $5 AND project_id = $4 AND workspace_id = $2 AND deleted_at IS NULL)"
        }
        "COMMENT_DESCRIPTION" => {
            "UPDATE file_assets SET comment_id = $5, project_id = $4 WHERE id = ANY($1) AND workspace_id = $2 AND created_by_id = $3 AND (project_id IS NULL OR project_id = $4) AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM issue_comments WHERE id = $5 AND project_id = $4 AND workspace_id = $2 AND deleted_at IS NULL)"
        }
        "PAGE_DESCRIPTION" => {
            "UPDATE file_assets SET page_id = $5, project_id = $4 WHERE id = ANY($1) AND workspace_id = $2 AND created_by_id = $3 AND (project_id IS NULL OR project_id = $4) AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM project_pages WHERE page_id = $5 AND project_id = $4 AND workspace_id = $2 AND deleted_at IS NULL)"
        }
        "DRAFT_ISSUE_DESCRIPTION" => {
            "UPDATE file_assets SET draft_issue_id = $5, project_id = $4 WHERE id = ANY($1) AND workspace_id = $2 AND created_by_id = $3 AND (project_id IS NULL OR project_id = $4) AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM draft_issues WHERE id = $5 AND project_id = $4 AND workspace_id = $2 AND deleted_at IS NULL)"
        }
        _ => return no_content(),
    };
    sqlx::query(sql)
        .bind(&ids)
        .bind(p.workspace_id())
        .bind(user.id)
        .bind(project)
        .bind(entity)
        .execute(&st.db)
        .await?;
    no_content()
}

async fn mark_project_uploaded(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, asset_id)): Path<(String, Uuid, Uuid)>,
    body: Option<Json<Value>>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    let (_, entity_type, id) = owned_asset(&st, &p.ws, asset_id, Some(project_id)).await?;
    set_uploaded(&st, id, &entity_type, &body.map(|b| b.0).unwrap_or_default()).await?;
    no_content()
}

async fn delete_project_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, asset_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    let (_, entity_type, id) = owned_asset(&st, &p.ws, asset_id, Some(project_id)).await?;
    soft_delete(&st, id, &entity_type).await?;
    no_content()
}

async fn user_owned(st: &AppState, user: &CurrentUser, asset_id: Uuid) -> Result<String, ApiError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT entity_type FROM file_assets WHERE id = $1 AND user_id = $2 AND entity_type IN ('USER_AVATAR', 'USER_COVER')
         AND deleted_at IS NULL",
    )
    .bind(asset_id)
    .bind(user.id)
    .fetch_optional(&st.db)
    .await?;
    row.map(|(t,)| t).ok_or_else(ApiError::not_found)
}

async fn mark_user_uploaded(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(asset_id): Path<Uuid>,
    body: Option<Json<Value>>,
) -> ApiResult<StatusCode> {
    let entity_type = user_owned(&st, &user, asset_id).await?;
    set_uploaded(&st, asset_id, &entity_type, &body.map(|b| b.0).unwrap_or_default()).await?;
    no_content()
}

pub(crate) async fn delete_user_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(asset_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let entity_type = user_owned(&st, &user, asset_id).await?;
    soft_delete(&st, asset_id, &entity_type).await?;
    no_content()
}

// --- issue attachments ---

const ATTACHMENT_JSON: &str = "jsonb_build_object('id', f.id, 'attributes', f.attributes, 'asset', f.asset,
    'asset_url', '/api/assets/v2/workspaces/' || w.slug || '/projects/' || f.project_id || '/issues/' || f.issue_id || '/attachments/' || f.id || '/',
    'issue_id', f.issue_id, 'project_id', f.project_id, 'workspace_id', f.workspace_id, 'size', f.size,
    'created_at', f.created_at, 'updated_at', f.updated_at, 'created_by', f.created_by_id, 'updated_by', f.updated_by_id)";

/// The issue must belong to this project (and be visible to a restricted guest).
async fn issue_ctx(
    st: &AppState,
    user: &CurrentUser,
    slug: &str,
    project_id: Uuid,
    issue_id: Uuid,
) -> Result<ProjectCtx, ApiError> {
    let p = require_project(&st.db, user, slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let ok: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM issues WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL
                        AND (NOT $4 OR created_by_id = $5))",
    )
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(p.restricted_guest())
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    if !ok.0 {
        return Err(ApiError::not_found());
    }
    Ok(p)
}

async fn list_attachments(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = issue_ctx(&st, &user, &slug, project_id, issue_id).await?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {ATTACHMENT_JSON} FROM file_assets f JOIN workspaces w ON w.id = f.workspace_id
             WHERE f.issue_id = $1 AND f.project_id = $2 AND f.workspace_id = $3 AND f.entity_type = 'ISSUE_ATTACHMENT'
               AND f.is_uploaded AND f.deleted_at IS NULL ORDER BY f.created_at DESC"
        ))
        .bind(issue_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn create_attachment(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = issue_ctx(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let mut v = insert_asset(
        &st,
        NewAsset {
            workspace_id: p.workspace_id(),
            user_id: user.id,
            entity_type: "ISSUE_ATTACHMENT",
            project_id: Some(project_id),
            issue_id: Some(issue_id),
            target_user_id: None,
            page_id: None,
            comment_id: None,
            images_only: false,
        },
        &b,
    )
    .await?
    .0;
    let id = v["asset_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(ApiError::not_found)?;
    let attachment: Value = sqlx::query_scalar(&format!(
        "SELECT {ATTACHMENT_JSON} FROM file_assets f JOIN workspaces w ON w.id = f.workspace_id WHERE f.id = $1"
    ))
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    v["attachment"] = attachment;
    Ok(Json(v))
}

async fn download_attachment(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, asset_id)): Path<(String, Uuid, Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let p = issue_ctx(&st, &user, &slug, project_id, issue_id).await?;
    let row: Option<(String, Value)> = sqlx::query_as(
        "SELECT asset, attributes FROM file_assets WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $4
         AND is_uploaded AND deleted_at IS NULL",
    )
    .bind(asset_id)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    let (key, attrs) = row.ok_or_else(ApiError::not_found)?;
    serve(&st, &key, &attrs, true).await
}

async fn mark_attachment_uploaded(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, asset_id)): Path<(String, Uuid, Uuid, Uuid)>,
    body: Option<Json<Value>>,
) -> ApiResult<StatusCode> {
    let p = issue_ctx(&st, &user, &slug, project_id, issue_id).await?;
    owned_asset(&st, &p.ws, asset_id, Some(project_id)).await?;
    let mut tx = st.db.begin().await?;
    let body = body.map(|b| b.0).unwrap_or_default();
    let row: Option<(String, bool)> = sqlx::query_as("UPDATE file_assets SET is_uploaded=true,attributes=COALESCE($5,attributes),updated_at=now() WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND entity_type='ISSUE_ATTACHMENT' AND deleted_at IS NULL RETURNING asset,is_uploaded")
        .bind(asset_id).bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(body.get("attributes")).fetch_optional(&mut *tx).await?;
    let (asset, _) = row.ok_or_else(ApiError::not_found)?;
    let mut event = crate::activity::ActivityEvent::new(
        issue_id,
        project_id,
        p.workspace_id(),
        user.id,
        "created",
        "created an attachment",
    );
    event.field = Some("attachment");
    event.new_value = Some(&asset);
    event.new_identifier = Some(asset_id);
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    crate::side_effects::spawn_asset_storage_metadata(st.clone(), asset_id);
    no_content()
}

async fn delete_attachment(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, asset_id)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = issue_ctx(&st, &user, &slug, project_id, issue_id).await?;
    owned_asset(&st, &p.ws, asset_id, Some(project_id)).await?;
    let mut tx = st.db.begin().await?;
    let result = sqlx::query("UPDATE file_assets SET is_deleted=true,deleted_at=now() WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND entity_type='ISSUE_ATTACHMENT' AND deleted_at IS NULL")
        .bind(asset_id).bind(issue_id).bind(project_id).bind(p.workspace_id()).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    let mut event = crate::activity::ActivityEvent::new(
        issue_id,
        project_id,
        p.workspace_id(),
        user.id,
        "deleted",
        "deleted the attachment",
    );
    event.field = Some("attachment");
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    no_content()
}

// --- legacy FileAsset endpoints ---

pub(crate) const FILE_ASSET_JSON: &str = "(to_jsonb(f) - 'workspace_id' - 'project_id' - 'issue_id' - 'created_by_id' - 'updated_by_id' - 'comment_id' - 'page_id' - 'user_id' - 'draft_issue_id')
|| jsonb_build_object(
    'workspace', f.workspace_id,
    'project', f.project_id,
    'issue', f.issue_id,
    'created_by', f.created_by_id,
    'updated_by', f.updated_by_id,
    'comment', f.comment_id,
    'page', f.page_id,
    'user', f.user_id,
    'draft_issue', f.draft_issue_id,
    'asset_url', CASE
        WHEN f.entity_type IN ('WORKSPACE_LOGO', 'USER_AVATAR', 'USER_COVER', 'PROJECT_COVER')
            THEN '/api/assets/v2/static/' || f.id || '/'
        WHEN f.workspace_id IS NOT NULL AND f.project_id IS NOT NULL AND f.entity_type = 'ISSUE_ATTACHMENT' AND w.slug IS NOT NULL
            THEN '/api/assets/v2/workspaces/' || w.slug || '/projects/' || f.project_id || '/issues/' || f.issue_id || '/attachments/' || f.id || '/'
        WHEN f.workspace_id IS NOT NULL AND f.project_id IS NOT NULL AND w.slug IS NOT NULL
            THEN '/api/assets/v2/workspaces/' || w.slug || '/projects/' || f.project_id || '/' || f.id || '/'
        WHEN f.workspace_id IS NOT NULL AND w.slug IS NOT NULL
            THEN '/api/assets/v2/workspaces/' || w.slug || '/' || f.id || '/'
        ELSE NULL
    END
)";

pub(crate) async fn require_workspace_by_id(
    db: &sqlx::PgPool,
    user: &CurrentUser,
    workspace_id: Uuid,
) -> Result<WorkspaceCtx, ApiError> {
    let row: Option<(String, i16)> = sqlx::query_as(
        "SELECT w.slug, m.role FROM workspaces w
         JOIN workspace_members m ON m.workspace_id = w.id AND m.member_id = $2 AND m.is_active AND m.deleted_at IS NULL
         WHERE w.id = $1 AND w.deleted_at IS NULL",
    )
    .bind(workspace_id)
    .bind(user.id)
    .fetch_optional(db)
    .await?;
    let (slug, role) = row.ok_or_else(ApiError::forbidden)?;
    Ok(WorkspaceCtx {
        workspace_id,
        slug,
        user_id: user.id,
        role,
    })
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct PermAsset {
    id: Uuid,
    workspace_id: Option<Uuid>,
    project_id: Option<Uuid>,
    entity_type: Option<String>,
    created_by_id: Option<Uuid>,
}

async fn can_modify_file_asset(
    db: &sqlx::PgPool,
    user: &CurrentUser,
    workspace_role: i16,
    asset: &PermAsset,
) -> Result<bool, ApiError> {
    if asset.entity_type.as_deref() == Some("WORKSPACE_LOGO") {
        return Ok(workspace_role >= ADMIN);
    }
    let mut role = Some(workspace_role);
    if let (Some(ws_id), Some(project_id)) = (asset.workspace_id, asset.project_id) {
        let pm_role: Option<(i16,)> = sqlx::query_as(
            "SELECT role FROM project_members
             WHERE workspace_id = $1 AND project_id = $2 AND member_id = $3 AND is_active AND deleted_at IS NULL",
        )
        .bind(ws_id)
        .bind(project_id)
        .bind(user.id)
        .fetch_optional(db)
        .await?;
        role = match pm_role {
            Some(_) if workspace_role >= ADMIN => Some(ADMIN),
            Some((r,)) => Some(r),
            None => None,
        };
        if asset.entity_type.as_deref() == Some("PROJECT_COVER") {
            return Ok(role == Some(ADMIN));
        }
    }
    if asset.created_by_id == Some(user.id) && role.is_some() {
        return Ok(true);
    }
    if asset.entity_type.as_deref() == Some("ISSUE_ATTACHMENT") {
        return Ok(role == Some(ADMIN));
    }
    Ok(role.map_or(false, |r| r >= MEMBER))
}

/// GET /api/users/file-assets/
async fn legacy_list_user_assets(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {FILE_ASSET_JSON} FROM file_assets f
             LEFT JOIN workspaces w ON w.id = f.workspace_id
             WHERE (f.user_id = $1 OR (f.user_id IS NULL AND f.created_by_id = $1))
               AND f.entity_type IN ('USER_AVATAR', 'USER_COVER')
               AND f.deleted_at IS NULL
             ORDER BY f.created_at DESC"
        ))
        .bind(user.id),
        &st.db,
    )
    .await
}

/// POST /api/users/file-assets/ (multipart upload)
async fn legacy_upload_user_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    mut form: Multipart,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let mut file_data: Option<(String, String, Vec<u8>)> = None;
    let mut entity_type = "USER_AVATAR".to_string();

    while let Some(field) = form
        .next_field()
        .await
        .map_err(|_| ApiError::bad("Invalid multipart body"))?
    {
        match field.name() {
            Some("entity_type") => {
                let val = field.text().await.map_err(|_| ApiError::bad("Invalid entity_type"))?;
                if !matches!(val.as_str(), "USER_AVATAR" | "USER_COVER") {
                    return Err(ApiError::bad_body(
                        json!({"error": "Invalid entity type.", "status": false}),
                    ));
                }
                entity_type = val;
            }
            Some("asset" | "file") => {
                let name = sanitize_filename(field.file_name().unwrap_or("unnamed"));
                let typ = field.content_type().unwrap_or("application/octet-stream").to_string();
                let mut buf = Vec::new();
                let mut field = field;
                while let Some(chunk) = field.chunk().await.map_err(|_| ApiError::bad("Upload interrupted"))? {
                    if buf.len() + chunk.len() > st.cfg.file_size_limit as usize {
                        return Err(ApiError::bad("File size exceeds the allowed limit"));
                    }
                    buf.extend_from_slice(&chunk);
                }
                file_data = Some((name, typ, buf));
            }
            _ => {}
        }
    }

    let (name, typ, data) = file_data.ok_or_else(|| ApiError::bad("Missing file field"))?;
    let id = Uuid::new_v4();
    let key = format!("users/{}/{}-{}", user.id, id.simple(), name);
    let path = disk_path(&st, &key)?;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io_err)?;
    }
    tokio::fs::write(&path, &data).await.map_err(io_err)?;

    let size = data.len() as f64;
    sqlx::query(
        "INSERT INTO file_assets (id, attributes, asset, size, user_id, created_by_id, updated_by_id, entity_type, is_uploaded)
         VALUES ($1, $2, $3, $4, $5, $5, $5, $6, true)",
    )
    .bind(id)
    .bind(json!({"name": name, "type": typ, "size": size}))
    .bind(&key)
    .bind(size)
    .bind(user.id)
    .bind(&entity_type)
    .execute(&st.db)
    .await?;

    let row: Value = sqlx::query_scalar(&format!(
        "SELECT {FILE_ASSET_JSON} FROM file_assets f
         LEFT JOIN workspaces w ON w.id = f.workspace_id
         WHERE f.id = $1"
    ))
    .bind(id)
    .fetch_one(&st.db)
    .await?;

    Ok((StatusCode::CREATED, Json(row)))
}

/// Helper to load a user's asset by pk (UUID or asset path). Returns 403 if owned by another user.
async fn check_user_asset_pk(st: &AppState, user: &CurrentUser, pk: &str) -> Result<(Uuid, Option<String>), ApiError> {
    let clean_pk = pk.split('?').next().unwrap_or(pk);
    if clean_pk.contains("..") || clean_pk.starts_with('/') || clean_pk.contains('\\') {
        return Err(ApiError::bad("Invalid asset key"));
    }
    let parsed_uuid = Uuid::parse_str(clean_pk).ok();
    let row: Option<(Uuid, Option<Uuid>, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT id, created_by_id, user_id, entity_type FROM file_assets
         WHERE ($1::uuid IS NOT NULL AND id = $1)
            OR asset = $2
            OR asset = $3
            OR asset = $4",
    )
    .bind(parsed_uuid)
    .bind(clean_pk)
    .bind(format!("users/{}/{}", user.id, clean_pk))
    .bind(format!("users/{}", clean_pk))
    .fetch_optional(&st.db)
    .await?;

    let (id, creator, uid, et) = row.ok_or_else(ApiError::not_found)?;
    if creator != Some(user.id) && uid != Some(user.id) {
        return Err(ApiError::forbidden());
    }
    Ok((id, et))
}

/// GET /api/users/file-assets/{asset_key}/
/// Plane `UserAssetsEndpoint.get`: assets with this key created by the caller, always HTTP 200.
/// Saturn also accepts the asset id or the key without its `users/{id}/` prefix, since Plane keys contain
/// slashes that `<str:asset_key>` can never match.
async fn legacy_get_user_asset(State(st): State<AppState>, user: CurrentUser, Path(pk): Path<String>) -> ApiResult {
    let files: Value = sqlx::query_scalar(&format!(
        "SELECT COALESCE(jsonb_agg({FILE_ASSET_JSON} ORDER BY f.created_at), '[]'::jsonb) FROM file_assets f
         LEFT JOIN workspaces w ON w.id = f.workspace_id
         WHERE (f.asset = $1 OR f.asset = $3 OR ($4::uuid IS NOT NULL AND f.id = $4))
           AND f.created_by_id = $2 AND f.deleted_at IS NULL"
    ))
    .bind(&pk)
    .bind(user.id)
    .bind(format!("users/{}/{}", user.id, pk))
    .bind(Uuid::parse_str(&pk).ok())
    .fetch_one(&st.db)
    .await?;
    if files.as_array().is_some_and(|a| !a.is_empty()) {
        Ok(Json(json!({"data": files, "status": true})))
    } else {
        Ok(Json(json!({"error": "Asset key does not exist", "status": false})))
    }
}

/// DELETE /api/users/file-assets/{pk}/
async fn legacy_delete_user_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(pk): Path<String>,
) -> ApiResult<StatusCode> {
    let (id, et) = check_user_asset_pk(&st, &user, &pk).await?;
    soft_delete(&st, id, et.as_deref().unwrap_or("")).await?;
    no_content()
}

/// GET /api/workspaces/file-assets/{workspace_id}/{asset_key}/
async fn legacy_get_workspace_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((workspace_id, asset_key)): Path<(Uuid, String)>,
) -> ApiResult {
    let _ws = require_workspace_by_id(&st.db, &user, workspace_id).await?;
    let clean_key = asset_key.split('?').next().unwrap_or(&asset_key);
    if clean_key.contains("..") || clean_key.starts_with('/') || clean_key.contains('\\') {
        return Err(ApiError::bad("Invalid asset key"));
    }
    let full_key = format!("{workspace_id}/{clean_key}");
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {FILE_ASSET_JSON} FROM file_assets f
         LEFT JOIN workspaces w ON w.id = f.workspace_id
         WHERE (f.asset = $1 OR f.asset = $2)
           AND f.workspace_id = $3
           AND f.deleted_at IS NULL
           AND (f.project_id IS NULL OR EXISTS (
               SELECT 1 FROM project_members pm
               WHERE pm.workspace_id = f.workspace_id AND pm.project_id = f.project_id
                 AND pm.member_id = $4 AND pm.is_active AND pm.deleted_at IS NULL
           ))
         ORDER BY f.created_at DESC"
    ))
    .bind(&full_key)
    .bind(clean_key)
    .bind(workspace_id)
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;

    if rows.is_empty() {
        Ok(Json(json!({"error": "Asset key does not exist", "status": false})))
    } else {
        Ok(Json(json!({"data": rows, "status": true})))
    }
}

/// DELETE /api/workspaces/file-assets/{workspace_id}/{asset_key}/
async fn legacy_delete_workspace_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((workspace_id, asset_key)): Path<(Uuid, String)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace_by_id(&st.db, &user, workspace_id).await?;
    let clean_key = asset_key.split('?').next().unwrap_or(&asset_key);
    if clean_key.contains("..") || clean_key.starts_with('/') || clean_key.contains('\\') {
        return Err(ApiError::bad("Invalid asset key"));
    }
    let full_key = format!("{workspace_id}/{clean_key}");
    let asset: Option<PermAsset> = sqlx::query_as(
        "SELECT id, workspace_id, project_id, entity_type, created_by_id FROM file_assets
         WHERE (asset = $1 OR asset = $2) AND workspace_id = $3 AND deleted_at IS NULL",
    )
    .bind(&full_key)
    .bind(clean_key)
    .bind(workspace_id)
    .fetch_optional(&st.db)
    .await?;

    let asset = asset.ok_or_else(ApiError::not_found)?;
    if !can_modify_file_asset(&st.db, &user, ws.role, &asset).await? {
        return Err(ApiError::forbidden());
    }
    soft_delete(&st, asset.id, asset.entity_type.as_deref().unwrap_or("")).await?;
    no_content()
}

/// POST /api/workspaces/file-assets/{workspace_id}/{asset_key}/restore/
async fn legacy_restore_workspace_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((workspace_id, asset_key)): Path<(Uuid, String)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace_by_id(&st.db, &user, workspace_id).await?;
    let clean_key = asset_key.split('?').next().unwrap_or(&asset_key);
    if clean_key.contains("..") || clean_key.starts_with('/') || clean_key.contains('\\') {
        return Err(ApiError::bad("Invalid asset key"));
    }
    let full_key = format!("{workspace_id}/{clean_key}");
    let asset: Option<PermAsset> = sqlx::query_as(
        "SELECT id, workspace_id, project_id, entity_type, created_by_id FROM file_assets
         WHERE (asset = $1 OR asset = $2) AND workspace_id = $3",
    )
    .bind(&full_key)
    .bind(clean_key)
    .bind(workspace_id)
    .fetch_optional(&st.db)
    .await?;

    let asset = asset.ok_or_else(ApiError::not_found)?;
    if !can_modify_file_asset(&st.db, &user, ws.role, &asset).await? {
        return Err(ApiError::forbidden());
    }
    sqlx::query("UPDATE file_assets SET is_deleted = false, deleted_at = NULL, updated_at = now() WHERE id = $1 AND workspace_id = $2")
        .bind(asset.id)
        .bind(workspace_id)
        .execute(&st.db)
        .await?;
    no_content()
}

/// GET /api/workspaces/{slug}/file-assets/
async fn legacy_list_workspace_assets(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {FILE_ASSET_JSON} FROM file_assets f
             JOIN workspaces w ON w.id = f.workspace_id
             WHERE f.workspace_id = $1 AND f.deleted_at IS NULL
               AND (f.project_id IS NULL OR EXISTS (
                   SELECT 1 FROM project_members pm
                   WHERE pm.workspace_id = f.workspace_id AND pm.project_id = f.project_id
                     AND pm.member_id = $2 AND pm.is_active AND pm.deleted_at IS NULL
               ))
             ORDER BY f.created_at DESC"
        ))
        .bind(ws.workspace_id)
        .bind(user.id),
        &st.db,
    )
    .await
}

/// POST /api/workspaces/{slug}/file-assets/
async fn legacy_create_workspace_asset(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    req: axum::extract::Request,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();

    let id = Uuid::new_v4();
    if content_type.starts_with("multipart/form-data") {
        let mut form = Multipart::from_request(req, &st)
            .await
            .map_err(|_| ApiError::bad("Invalid multipart body"))?;
        let mut file_data: Option<(String, String, Vec<u8>)> = None;
        let mut client_attrs: Option<Value> = None;

        while let Some(field) = form
            .next_field()
            .await
            .map_err(|_| ApiError::bad("Invalid multipart body"))?
        {
            match field.name() {
                Some("attributes") => {
                    if let Ok(t) = field.text().await {
                        client_attrs = serde_json::from_str(&t).ok();
                    }
                }
                Some("asset" | "file") => {
                    let name = sanitize_filename(field.file_name().unwrap_or("unnamed"));
                    let typ = field.content_type().unwrap_or("application/octet-stream").to_string();
                    let mut buf = Vec::new();
                    let mut field = field;
                    while let Some(chunk) = field.chunk().await.map_err(|_| ApiError::bad("Upload interrupted"))? {
                        if buf.len() + chunk.len() > st.cfg.file_size_limit as usize {
                            return Err(ApiError::bad("File size exceeds the allowed limit"));
                        }
                        buf.extend_from_slice(&chunk);
                    }
                    file_data = Some((name, typ, buf));
                }
                // Plane #10212: all other fields (project, issue, entity_type, is_uploaded, etc.) are read-only
                _ => {}
            }
        }

        let (name, typ, data) = file_data.ok_or_else(|| ApiError::bad("Missing file field"))?;
        let key = format!("{}/{}-{}", ws.workspace_id, id.simple(), name);
        let path = disk_path(&st, &key)?;
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(io_err)?;
        }
        tokio::fs::write(&path, &data).await.map_err(io_err)?;

        let size = data.len() as f64;
        let attrs = client_attrs.unwrap_or_else(|| json!({"name": name, "type": typ, "size": size}));

        sqlx::query(
            "INSERT INTO file_assets (id, attributes, asset, size, workspace_id, created_by_id, updated_by_id, is_uploaded)
             VALUES ($1, $2, $3, $4, $5, $6, $6, true)",
        )
        .bind(id)
        .bind(attrs)
        .bind(&key)
        .bind(size)
        .bind(ws.workspace_id)
        .bind(user.id)
        .execute(&st.db)
        .await?;
    } else {
        let b: Value = axum::Json::from_request(req, &st)
            .await
            .map_err(|_| ApiError::bad("Invalid JSON body"))?
            .0;
        let name = sanitize_filename(s(&b, "name").unwrap_or("unnamed"));
        let typ = s(&b, "type").unwrap_or("application/octet-stream");
        let size = b
            .get("size")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .min(st.cfg.file_size_limit as f64);
        let key = format!("{}/{}-{}", ws.workspace_id, id.simple(), name);
        let attrs = b
            .get("attributes")
            .cloned()
            .unwrap_or_else(|| json!({"name": name, "type": typ, "size": size}));

        sqlx::query(
            "INSERT INTO file_assets (id, attributes, asset, size, workspace_id, created_by_id, updated_by_id, is_uploaded)
             VALUES ($1, $2, $3, $4, $5, $6, $6, false)",
        )
        .bind(id)
        .bind(attrs)
        .bind(&key)
        .bind(size)
        .bind(ws.workspace_id)
        .bind(user.id)
        .execute(&st.db)
        .await?;
    }

    let row: Value = sqlx::query_scalar(&format!(
        "SELECT {FILE_ASSET_JSON} FROM file_assets f
         JOIN workspaces w ON w.id = f.workspace_id
         WHERE f.id = $1"
    ))
    .bind(id)
    .fetch_one(&st.db)
    .await?;

    Ok((StatusCode::CREATED, Json(row)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_cannot_escape() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a\\b\\..\\c.png"), "c.png");
        assert_eq!(sanitize_filename("..hidden"), "hidden");
        assert_eq!(sanitize_filename("my file<script>.png"), "my file_script_.png");
        assert_eq!(sanitize_filename(""), "unnamed");
    }

    #[test]
    fn tokens_are_per_asset() {
        let a = Uuid::new_v4();
        assert_eq!(token("k", a), token("k", a));
        assert_ne!(token("k", a), token("k", Uuid::new_v4()));
        assert_ne!(token("k", a), token("other", a));
    }
}
