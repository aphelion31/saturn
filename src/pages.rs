//! Project pages, scoped to active project membership and page visibility.
use crate::common::*;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/pages/",
            get(list).post(create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/pages-summary/",
            get(summary),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/pages/{page_id}/",
            get(retrieve).patch(update).delete(destroy),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/pages/{page_id}/description/",
            get(description).patch(update_description),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/pages/{page_id}/archive/",
            axum::routing::post(archive).delete(unarchive),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/pages/{page_id}/lock/",
            axum::routing::post(lock).delete(unlock),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/pages/{page_id}/access/",
            axum::routing::post(access),
        )
}

const PAGE: &str = "jsonb_build_object('id',p.id,'name',p.name,'owned_by',p.owned_by_id,'access',p.access,'color',p.color,'parent',p.parent_id,'is_locked',p.is_locked,'archived_at',p.archived_at,'workspace',p.workspace_id,'created_at',p.created_at,'updated_at',p.updated_at,'created_by',p.created_by_id,'updated_by',p.updated_by_id,'view_props',p.view_props,'logo_props',p.logo_props,'description_html',p.description_html,'is_favorite',EXISTS(SELECT 1 FROM user_favorites f WHERE f.workspace_id=p.workspace_id AND f.project_id=$2 AND f.user_id=$3 AND f.entity_type='page' AND f.entity_identifier=p.id AND f.deleted_at IS NULL),'label_ids',COALESCE((SELECT jsonb_agg(pl.label_id) FROM page_labels pl WHERE pl.page_id=p.id AND pl.workspace_id=p.workspace_id AND pl.deleted_at IS NULL),'[]'::jsonb),'project_ids',COALESCE((SELECT jsonb_agg(pp.project_id) FROM project_pages pp WHERE pp.page_id=p.id AND pp.workspace_id=p.workspace_id AND pp.deleted_at IS NULL),'[]'::jsonb))";
const SCOPE: &str = "p.workspace_id=$1 AND p.deleted_at IS NULL AND (p.access=0 OR p.owned_by_id=$3) AND (NOT $4 OR p.owned_by_id=$3) AND EXISTS (SELECT 1 FROM project_pages pp WHERE pp.page_id=p.id AND pp.project_id=$2 AND pp.workspace_id=$1 AND pp.deleted_at IS NULL)";
type ProjectPath = (String, Uuid);
type PagePath = (String, Uuid, Uuid);

async fn list(State(st): State<AppState>, user: CurrentUser, Path((slug, project)): Path<ProjectPath>) -> ApiResult {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {PAGE} FROM pages p WHERE {SCOPE} AND p.parent_id IS NULL ORDER BY p.created_at DESC, p.id"
        ))
        .bind(ctx.workspace_id())
        .bind(project)
        .bind(user.id)
        .bind(ctx.restricted_guest()),
        &st.db,
    )
    .await
}
async fn retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
) -> ApiResult {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(sqlx::query_scalar(&format!("SELECT {PAGE} || jsonb_build_object('issue_ids',COALESCE((SELECT jsonb_agg(l.entity_identifier) FROM page_logs l WHERE l.page_id=p.id AND l.workspace_id=p.workspace_id AND l.entity_name='issue' AND l.deleted_at IS NULL),'[]'::jsonb)) FROM pages p WHERE {SCOPE} AND p.id=$5"))
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id), &st.db).await
}
async fn summary(State(st): State<AppState>, user: CurrentUser, Path((slug, project)): Path<ProjectPath>) -> ApiResult {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(sqlx::query_scalar(&format!("SELECT jsonb_build_object('public_pages',count(*) FILTER (WHERE p.access=0 AND p.archived_at IS NULL),'private_pages',count(*) FILTER (WHERE p.access=1 AND p.archived_at IS NULL),'archived_pages',count(*) FILTER (WHERE p.archived_at IS NOT NULL)) FROM pages p WHERE {SCOPE} AND p.parent_id IS NULL"))
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()), &st.db).await
}
async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project)): Path<ProjectPath>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    if b.get("description_html").is_some() {
        return Err(ApiError::bad("HTML updates are not supported"));
    }
    let access = b.get("access").and_then(Value::as_i64).unwrap_or(0);
    if !(0..=1).contains(&access) {
        return Err(ApiError::bad("Invalid access"));
    }
    let mut tx = st.db.begin().await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO pages (name,color,workspace_id,owned_by_id,created_by_id,updated_by_id,access,description_html) VALUES ($1,$2,$3,$4,$4,$4,$5,$6) RETURNING id")
        .bind(s(&b,"name").unwrap_or("")).bind(s(&b,"color").unwrap_or("")).bind(ctx.workspace_id()).bind(user.id).bind(access as i16).bind(s(&b,"description_html").unwrap_or("<p></p>"))
        .fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO project_pages (workspace_id,project_id,page_id,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$4)")
        .bind(ctx.workspace_id()).bind(project).bind(id).bind(user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    let Json(v) = retrieve(State(st), user, Path((slug, project, id))).await?;
    Ok((StatusCode::CREATED, Json(v)))
}
async fn update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    let row: Option<(bool, Uuid, i16)> = sqlx::query_as(&format!(
        "SELECT p.is_locked,p.owned_by_id,p.access FROM pages p WHERE {SCOPE} AND p.id=$5"
    ))
    .bind(ctx.workspace_id())
    .bind(project)
    .bind(user.id)
    .bind(ctx.restricted_guest())
    .bind(id)
    .fetch_optional(&st.db)
    .await?;
    let (locked, owner, old_access) = row.ok_or_else(ApiError::not_found)?;
    if locked {
        return Err(ApiError::bad("Page is locked"));
    }
    let access = b.get("access").and_then(Value::as_i64).unwrap_or(old_access as i64);
    if !(0..=1).contains(&access) {
        return Err(ApiError::bad("Invalid access"));
    }
    if access != old_access as i64 && owner != user.id {
        return Err(ApiError::forbidden());
    }
    sqlx::query(&format!("UPDATE pages p SET name=COALESCE($6,p.name),color=COALESCE($7,p.color),access=$8,view_props=COALESCE($9,p.view_props),logo_props=COALESCE($10,p.logo_props),updated_by_id=$3,updated_at=now() WHERE {SCOPE} AND p.id=$5 AND NOT p.is_locked"))
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id).bind(s(&b,"name")).bind(s(&b,"color")).bind(access as i16).bind(b.get("view_props")).bind(b.get("logo_props")).execute(&st.db).await?;
    retrieve(State(st), user, Path((slug, project, id))).await
}
async fn destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
) -> ApiResult<StatusCode> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    let row: Option<(bool, Uuid)> = sqlx::query_as(&format!(
        "SELECT p.archived_at IS NOT NULL,p.owned_by_id FROM pages p WHERE {SCOPE} AND p.id=$5"
    ))
    .bind(ctx.workspace_id())
    .bind(project)
    .bind(user.id)
    .bind(ctx.restricted_guest())
    .bind(id)
    .fetch_optional(&st.db)
    .await?;
    let (archived, owner) = row.ok_or_else(ApiError::not_found)?;
    if !archived {
        return Err(ApiError::bad("The page should be archived before deleting"));
    }
    if owner != user.id {
        ctx.require(&[ADMIN])?;
    }
    sqlx::query(&format!(
        "UPDATE pages p SET deleted_at=now() WHERE {SCOPE} AND p.id=$5 AND p.archived_at IS NOT NULL"
    ))
    .bind(ctx.workspace_id())
    .bind(project)
    .bind(user.id)
    .bind(ctx.restricted_guest())
    .bind(id)
    .execute(&st.db)
    .await?;
    no_content()
}
async fn description(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
) -> Result<impl IntoResponse, ApiError> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER, GUEST])?;
    let bytes: Option<Option<Vec<u8>>> = sqlx::query_scalar(&format!(
        "SELECT p.description_binary FROM pages p WHERE {SCOPE} AND p.id=$5"
    ))
    .bind(ctx.workspace_id())
    .bind(project)
    .bind(user.id)
    .bind(ctx.restricted_guest())
    .bind(id)
    .fetch_optional(&st.db)
    .await?;
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        bytes.ok_or_else(ApiError::not_found)?.unwrap_or_default(),
    ))
}
async fn update_description(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    if b.get("description_html").is_some() {
        return Err(ApiError::bad("HTML updates are not supported"));
    }
    let binary = match b.get("description_binary") {
        None => None,
        Some(Value::String(v)) => Some(STANDARD.decode(v).map_err(|_| ApiError::bad("Invalid base64"))?),
        _ => return Err(ApiError::bad("Invalid description_binary")),
    };
    let r = sqlx::query(&format!("UPDATE pages p SET description_binary=COALESCE($6,p.description_binary),description_html=COALESCE($7,p.description_html),description_json=COALESCE($8,p.description_json),updated_at=now(),updated_by_id=$3 WHERE {SCOPE} AND p.id=$5 AND NOT p.is_locked AND p.archived_at IS NULL"))
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id).bind(binary).bind(s(&b,"description_html")).bind(b.get("description_json")).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    // ponytail: page transactions and version snapshots require Plane's background tasks.
    Ok(Json(json!({"message":"Updated successfully"})))
}

async fn set_flag(
    st: AppState,
    user: CurrentUser,
    (slug, project, id): PagePath,
    archived: Option<bool>,
    locked: Option<bool>,
) -> ApiResult<StatusCode> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    let owner: Option<Uuid> =
        sqlx::query_scalar(&format!("SELECT p.owned_by_id FROM pages p WHERE {SCOPE} AND p.id=$5"))
            .bind(ctx.workspace_id())
            .bind(project)
            .bind(user.id)
            .bind(ctx.restricted_guest())
            .bind(id)
            .fetch_optional(&st.db)
            .await?;
    if owner.ok_or_else(ApiError::not_found)? != user.id && archived.is_some() {
        ctx.require(&[ADMIN])?;
    }
    sqlx::query(&format!("UPDATE pages p SET archived_at=CASE WHEN $6::boolean IS NULL THEN p.archived_at WHEN $6 THEN CURRENT_DATE ELSE NULL END, is_locked=COALESCE($7,p.is_locked), updated_at=now(), updated_by_id=$3 WHERE {SCOPE} AND p.id=$5"))
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id).bind(archived).bind(locked).execute(&st.db).await?;
    // ponytail: descendant archive propagation and favorite cleanup are not implemented.
    no_content()
}
async fn archive(State(st): State<AppState>, user: CurrentUser, Path(path): Path<PagePath>) -> ApiResult<StatusCode> {
    set_flag(st, user, path, Some(true), None).await
}
async fn unarchive(State(st): State<AppState>, user: CurrentUser, Path(path): Path<PagePath>) -> ApiResult<StatusCode> {
    set_flag(st, user, path, Some(false), None).await
}
async fn lock(State(st): State<AppState>, user: CurrentUser, Path(path): Path<PagePath>) -> ApiResult<StatusCode> {
    set_flag(st, user, path, None, Some(true)).await
}
async fn unlock(State(st): State<AppState>, user: CurrentUser, Path(path): Path<PagePath>) -> ApiResult<StatusCode> {
    set_flag(st, user, path, None, Some(false)).await
}
async fn access(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
    Json(b): Json<Value>,
) -> ApiResult<StatusCode> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    let value = b.get("access").and_then(Value::as_i64).unwrap_or(0);
    if !(0..=1).contains(&value) {
        return Err(ApiError::bad("Invalid access"));
    }
    let r = sqlx::query(&format!("UPDATE pages p SET access=$6,updated_at=now(),updated_by_id=$3 WHERE {SCOPE} AND p.id=$5 AND (p.owned_by_id=$3 OR p.access=$6)"))
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id).bind(value as i16).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}
