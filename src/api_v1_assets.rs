use crate::{assets, common::*, outbound};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
};
use serde_json::{Value, json};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/assets/user-assets/", post(user_create))
        .route("/api/v1/assets/user-assets/server/", post(user_create))
        .route(
            "/api/v1/assets/user-assets/{asset_id}/",
            axum::routing::patch(user_patch).delete(user_delete),
        )
        .route(
            "/api/v1/assets/user-assets/{asset_id}/server/",
            axum::routing::patch(user_patch).delete(user_delete),
        )
        .route("/api/v1/workspaces/{slug}/assets/", post(create))
        .route(
            "/api/v1/workspaces/{slug}/assets/{asset_id}/",
            get(download).patch(update),
        )
        .route("/api/v1/schema/", get(schema))
        .route("/api/v1/schema/swagger-ui/", get(swagger))
        .route("/api/v1/schema/redoc/", get(redoc))
}

fn api_key(headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.contains_key("x-api-key") {
        Ok(())
    } else {
        Err(ApiError::unauthorized())
    }
}

async fn user_create(
    State(st): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Json(b): Json<Value>,
) -> ApiResult {
    api_key(&headers)?;
    assets::create_user_asset(State(st), user, Json(b)).await
}
async fn user_patch(
    State(st): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    body: Option<Json<Value>>,
) -> ApiResult<StatusCode> {
    api_key(&headers)?;
    let attrs = body.and_then(|Json(b)| b.get("attributes").cloned());
    let result = sqlx::query("UPDATE file_assets SET is_uploaded=true, attributes=COALESCE($3,attributes) WHERE id=$1 AND user_id=$2 AND entity_type IN ('USER_AVATAR','USER_COVER') AND deleted_at IS NULL")
        .bind(id).bind(user.id).bind(attrs).execute(&st.db).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}
async fn user_delete(
    State(st): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    api_key(&headers)?;
    assets::delete_user_asset(State(st), user, Path(id)).await
}

pub(crate) const MIME: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/svg+xml",
    "image/webp",
    "image/tiff",
    "image/bmp",
    "application/pdf",
    "application/msword",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "text/plain",
    "text/markdown",
    "application/rtf",
    "application/vnd.oasis.opendocument.spreadsheet",
    "application/vnd.oasis.opendocument.text",
    "application/vnd.oasis.opendocument.presentation",
    "application/vnd.oasis.opendocument.graphics",
    "application/vnd.visio",
    "image/x-portable-graymap",
    "image/x-portable-bitmap",
    "image/x-portable-pixmap",
    "application/vnd.oasis.opendocument.database",
    "audio/mpeg",
    "audio/wav",
    "audio/ogg",
    "audio/midi",
    "audio/x-midi",
    "audio/aac",
    "audio/flac",
    "audio/x-m4a",
    "video/mp4",
    "video/mpeg",
    "video/ogg",
    "video/webm",
    "video/quicktime",
    "video/x-msvideo",
    "video/x-ms-wmv",
    "application/zip",
    "application/x-rar",
    "application/x-rar-compressed",
    "application/x-tar",
    "application/gzip",
    "application/x-zip",
    "application/x-zip-compressed",
    "application/x-7z-compressed",
    "application/x-compressed",
    "application/x-compressed-tar",
    "application/x-compressed-tar-gz",
    "application/x-compressed-tar-bz2",
    "application/x-compressed-tar-zip",
    "application/x-compressed-tar-7z",
    "application/x-compressed-tar-rar",
    "model/gltf-binary",
    "model/gltf+json",
    "application/octet-stream",
    "font/ttf",
    "font/otf",
    "font/woff",
    "font/woff2",
    "text/css",
    "text/javascript",
    "application/json",
    "text/xml",
    "text/csv",
    "application/xml",
    "application/x-sql",
    "application/x-gzip",
];
async fn create(
    State(st): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult {
    api_key(&headers)?;
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let name = s(&b, "name")
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| ApiError::bad("Name and size are required fields."))?;
    let size = b
        .get("size")
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n > 0.0)
        .ok_or_else(|| ApiError::bad("Name and size are required fields."))?
        .min(st.cfg.file_size_limit as f64);
    let typ = s(&b, "type")
        .filter(|v| MIME.contains(v))
        .ok_or_else(|| ApiError::bad("Invalid file type."))?;
    let project = uuid_of(&b, "project_id").ok_or_else(|| ApiError::bad("Project id is required."))?;
    require_project(&st.db, &user, &slug, project).await?;
    let ext_id = s(&b, "external_id");
    let ext_source = s(&b, "external_source");
    if let (Some(e), Some(source)) = (ext_id.filter(|x| !x.is_empty()), ext_source.filter(|x| !x.is_empty())) {
        let existing: Option<Uuid> = sqlx::query_scalar("SELECT id FROM file_assets WHERE workspace_id=$1 AND project_id=$2 AND external_id=$3 AND external_source=$4 AND deleted_at IS NULL AND NOT is_deleted LIMIT 1").bind(ws.workspace_id).bind(project).bind(e).bind(source).fetch_optional(&st.db).await?;
        if let Some(id) = existing {
            return Err(ApiError(
                StatusCode::CONFLICT,
                json!({"message":"Asset with same external id and source already exists","asset_id":id,"asset_url":format!("/api/assets/v2/workspaces/{slug}/{id}/")}),
            ));
        }
    }
    let name = assets::sanitize_filename(name);
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO file_assets (id,attributes,asset,size,workspace_id,project_id,created_by_id,updated_by_id,external_id,external_source,entity_type,is_uploaded) VALUES ($1,$2,$3,$4,$5,$6,$7,$7,$8,$9,'ISSUE_ATTACHMENT',false)")
        .bind(id).bind(json!({"name":name,"type":typ,"size":size})).bind(format!("{}/{}-{}",ws.workspace_id,id.simple(),name)).bind(size).bind(ws.workspace_id).bind(project).bind(user.id).bind(ext_id).bind(ext_source).execute(&st.db).await?;
    Ok(Json(assets::upload_response(id, &st.cfg.secret_key)))
}
async fn lookup(st: &AppState, user: &CurrentUser, slug: &str, id: Uuid) -> Result<(Value, bool), ApiError> {
    let ws = require_workspace(&st.db, user, slug).await?;
    let row: Option<(Value,bool,Option<Uuid>)>=sqlx::query_as("SELECT attributes,is_uploaded,project_id FROM file_assets WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL AND NOT is_deleted").bind(id).bind(ws.workspace_id).fetch_optional(&st.db).await?;
    let (attrs, uploaded, project) = row.ok_or_else(ApiError::not_found)?;
    if let Some(project) = project {
        require_project(&st.db, user, slug, project).await?;
    }
    Ok((attrs, uploaded))
}
async fn download(
    State(st): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    api_key(&headers)?;
    let (attrs, uploaded) = lookup(&st, &user, &slug, id).await?;
    if !uploaded {
        return Err(ApiError::bad("Asset not yet uploaded"));
    }
    Ok(Json(
        json!({"asset_id":id,"asset_url":format!("/api/assets/v2/workspaces/{slug}/{id}/"),"asset_name":attrs.get("name").and_then(Value::as_str).unwrap_or(""),"asset_type":attrs.get("type").and_then(Value::as_str).unwrap_or("")}),
    ))
}
async fn update(
    State(st): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<StatusCode> {
    api_key(&headers)?;
    let (_, old) = lookup(&st, &user, &slug, id).await?;
    let uploaded = match b.get("is_uploaded") {
        None => old,
        Some(v) => v.as_bool().ok_or_else(|| ApiError::bad("Invalid is_uploaded"))?,
    };
    let ws = require_workspace(&st.db, &user, &slug).await?;
    sqlx::query("UPDATE file_assets SET is_uploaded=$3 WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL")
        .bind(id)
        .bind(ws.workspace_id)
        .bind(uploaded)
        .execute(&st.db)
        .await?;
    if uploaded {
        crate::side_effects::spawn_asset_storage_metadata(st.clone(), id);
    }
    no_content()
}

async fn enabled(st: &AppState) -> Result<(), ApiError> {
    if outbound::config(st, "ENABLE_DRF_SPECTACULAR", "0").await == "1" {
        Ok(())
    } else {
        Err(ApiError::not_found())
    }
}
async fn schema(State(st): State<AppState>) -> ApiResult {
    enabled(&st).await?;
    // ponytail: static route inventory only, add generated operation schemas when detailed documentation is needed.
    let mut paths = serde_json::Map::new();
    for source in [
        include_str!("api_v1.rs"),
        include_str!("api_v1_items.rs"),
        include_str!("api_v1_assets.rs"),
    ] {
        for path in source
            .split('"')
            .filter(|s| s.starts_with("/api/v1/") && s.ends_with('/'))
        {
            let params: Vec<Value> = path
                .split('{')
                .skip(1)
                .filter_map(|part| part.split('}').next())
                .map(|name| json!({"name":name,"in":"path","required":true,"schema":{"type":"string"}}))
                .collect();
            paths.insert(path.to_string(), json!({"parameters":params}));
        }
    }
    Ok(Json(
        json!({"openapi":"3.0.3","info":{"title":"Saturn API","version":"1.0.0"},"paths":paths}),
    ))
}
async fn swagger(State(st): State<AppState>) -> Result<Html<&'static str>, ApiError> {
    enabled(&st).await?;
    Ok(Html(
        "<!doctype html><html><head><title>Saturn API</title><link rel=\"stylesheet\" href=\"https://unpkg.com/swagger-ui-dist@5/swagger-ui.css\"></head><body><div id=\"swagger-ui\"></div><script src=\"https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js\"></script><script>SwaggerUIBundle({url:'/api/v1/schema/',dom_id:'#swagger-ui'})</script></body></html>",
    ))
}
async fn redoc(State(st): State<AppState>) -> Result<Html<&'static str>, ApiError> {
    enabled(&st).await?;
    Ok(Html(
        "<!doctype html><html><head><title>Saturn API</title></head><body><redoc spec-url=\"/api/v1/schema/\"></redoc><script src=\"https://cdn.jsdelivr.net/npm/redoc@2/bundles/redoc.standalone.js\"></script></body></html>",
    ))
}
