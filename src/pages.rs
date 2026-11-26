//! Project pages, scoped to active project membership and page visibility.
use crate::common::*;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
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
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/favorite-pages/{page_id}/",
            axum::routing::post(favorite).delete(unfavorite),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/pages/{page_id}/versions/",
            get(versions),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/pages/{page_id}/versions/{version_id}/",
            get(version),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/pages/{page_id}/duplicate/",
            axum::routing::post(duplicate),
        )
        .route("/api/workspaces/{slug}/pages/{page_id}/live-access/", get(live_access))
        .route("/live/collaboration", get(collaboration))
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
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER, GUEST])?;
    if !q.get("track_visit").is_some_and(|v| v.eq_ignore_ascii_case("false")) {
        crate::recent_visits::record(
            &st.db,
            crate::recent_visits::Visit {
                workspace_id: ctx.workspace_id(),
                user_id: user.id,
                entity_name: "page",
                entity_identifier: id,
                project_id: Some(project),
            },
        );
    }
    fetch(&st, &user, &slug, project, id).await
}

/// Page detail without visit tracking, used by create/update which return the page.
async fn fetch(st: &AppState, user: &CurrentUser, slug: &str, project: Uuid, id: Uuid) -> ApiResult {
    let ctx = require_project(&st.db, user, slug, project).await?;
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
fn optional_html(b: &Value) -> Result<Option<&str>, ApiError> {
    match b.get("description_html") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(ApiError::bad_body(json!({"description_html": "Not a valid string."}))),
    }
}

async fn validate_page_parent(
    db: &sqlx::PgPool,
    workspace_id: Uuid,
    project_id: Uuid,
    page_id: Option<Uuid>,
    b: &Value,
) -> Result<(bool, Option<Uuid>), ApiError> {
    let Some(raw) = b.get("parent") else {
        return Ok((false, None));
    };
    if raw.is_null() {
        return Ok((true, None));
    }
    let parent_id = uuid_of(b, "parent").ok_or_else(|| ApiError::bad("Invalid parent page"))?;
    if page_id == Some(parent_id) {
        return Err(ApiError::bad("Page cannot be its own parent"));
    }
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM pages p
            JOIN project_pages pp ON pp.page_id = p.id AND pp.workspace_id = p.workspace_id AND pp.deleted_at IS NULL
            WHERE p.id = $1 AND p.workspace_id = $2 AND pp.project_id = $3 AND p.deleted_at IS NULL
        )",
    )
    .bind(parent_id)
    .bind(workspace_id)
    .bind(project_id)
    .fetch_one(db)
    .await?;
    if !exists {
        return Err(ApiError::bad("Parent page not found"));
    }
    Ok((true, Some(parent_id)))
}

async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project)): Path<ProjectPath>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    let html = sanitize_html(optional_html(&b)?.unwrap_or("<p></p>"))?;
    let stripped = strip_tags(&html);
    let access = b.get("access").and_then(Value::as_i64).unwrap_or(0);
    if !(0..=1).contains(&access) {
        return Err(ApiError::bad("Invalid access"));
    }
    let (_, parent_id) = validate_page_parent(&st.db, ctx.workspace_id(), project, None, &b).await?;
    let mut tx = st.db.begin().await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO pages (name,color,workspace_id,owned_by_id,created_by_id,updated_by_id,access,parent_id,description_html,description_stripped) VALUES ($1,$2,$3,$4,$4,$4,$5,$6,$7,$8) RETURNING id")
        .bind(s(&b,"name").unwrap_or("")).bind(s(&b,"color").unwrap_or("")).bind(ctx.workspace_id()).bind(user.id).bind(access as i16).bind(parent_id).bind(&html).bind(&stripped)
        .fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO project_pages (workspace_id,project_id,page_id,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$4)")
        .bind(ctx.workspace_id()).bind(project).bind(id).bind(user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    crate::side_effects::spawn_page_transaction(st.clone(), ctx.workspace_id(), id, Some(html), None);
    let Json(v) = fetch(&st, &user, &slug, project, id).await?;
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
    let row: Option<(bool, Uuid, i16, Option<String>)> = sqlx::query_as(&format!(
        "SELECT p.is_locked,p.owned_by_id,p.access,p.description_html FROM pages p WHERE {SCOPE} AND p.id=$5"
    ))
    .bind(ctx.workspace_id())
    .bind(project)
    .bind(user.id)
    .bind(ctx.restricted_guest())
    .bind(id)
    .fetch_optional(&st.db)
    .await?;
    let (locked, owner, old_access, old_html) = row.ok_or_else(ApiError::not_found)?;
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
    let (parent_set, parent_id) = validate_page_parent(&st.db, ctx.workspace_id(), project, Some(id), &b).await?;
    let html = optional_html(&b)?.map(sanitize_html).transpose()?;
    let stripped = html.as_deref().map(strip_tags);
    sqlx::query(&format!("WITH updated AS (UPDATE pages p SET name=COALESCE($6,p.name),color=COALESCE($7,p.color),access=$8,view_props=COALESCE($9,p.view_props),logo_props=COALESCE($10,p.logo_props),description_html=COALESCE($11,p.description_html),description_stripped=COALESCE($12,p.description_stripped),parent_id=CASE WHEN $13 THEN $14 ELSE p.parent_id END,updated_by_id=$3,updated_at=now() WHERE {SCOPE} AND p.id=$5 AND NOT p.is_locked RETURNING p.*) INSERT INTO page_versions (workspace_id,page_id,owned_by_id,created_by_id,updated_by_id,last_saved_at,description_binary,description_html,description_json,description_stripped) SELECT workspace_id,id,owned_by_id,$3,$3,now(),description_binary,description_html,description_json,description_stripped FROM updated WHERE $11::text IS NOT NULL"))
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id).bind(s(&b,"name")).bind(s(&b,"color")).bind(access as i16).bind(b.get("view_props")).bind(b.get("logo_props")).bind(&html).bind(stripped).bind(parent_set).bind(parent_id).execute(&st.db).await?;
    if html.is_some() {
        crate::side_effects::spawn_page_transaction(st.clone(), ctx.workspace_id(), id, html, old_html);
    }
    fetch(&st, &user, &slug, project, id).await
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
    crate::jobs::spawn_soft_delete_cascade(st.clone(), "pages", id);
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
    let html = optional_html(&b)?.map(sanitize_html).transpose()?;
    let stripped = html.as_deref().map(strip_tags);
    let binary = match b.get("description_binary") {
        None => None,
        Some(Value::String(v)) => Some(STANDARD.decode(v).map_err(|_| ApiError::bad("Invalid base64"))?),
        _ => return Err(ApiError::bad("Invalid description_binary")),
    };
    let old_html: Option<String> = if html.is_some() {
        sqlx::query_scalar(&format!(
            "SELECT p.description_html FROM pages p WHERE {SCOPE} AND p.id=$5"
        ))
        .bind(ctx.workspace_id())
        .bind(project)
        .bind(user.id)
        .bind(ctx.restricted_guest())
        .bind(id)
        .fetch_optional(&st.db)
        .await?
        .flatten()
    } else {
        None
    };
    let r = sqlx::query(&format!("WITH updated AS (UPDATE pages p SET description_binary=COALESCE($6,p.description_binary),description_html=COALESCE($7,p.description_html),description_json=COALESCE($8,p.description_json),description_stripped=COALESCE($9,p.description_stripped),updated_at=now(),updated_by_id=$3 WHERE {SCOPE} AND p.id=$5 AND NOT p.is_locked AND p.archived_at IS NULL RETURNING p.*) INSERT INTO page_versions (workspace_id,page_id,owned_by_id,created_by_id,updated_by_id,last_saved_at,description_binary,description_html,description_json,description_stripped) SELECT workspace_id,id,owned_by_id,$3,$3,now(),description_binary,description_html,description_json,description_stripped FROM updated"))
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id).bind(binary).bind(&html).bind(b.get("description_json")).bind(stripped).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    if html.is_some() {
        crate::side_effects::spawn_page_transaction(st.clone(), ctx.workspace_id(), id, html, old_html);
    }
    Ok(Json(json!({"message":"Updated successfully"})))
}

async fn set_locked(
    st: AppState,
    user: CurrentUser,
    (slug, project, id): PagePath,
    locked: bool,
) -> ApiResult<StatusCode> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    let r = sqlx::query(&format!(
        "UPDATE pages p SET is_locked=$6, updated_at=now(), updated_by_id=$3 WHERE {SCOPE} AND p.id=$5"
    ))
    .bind(ctx.workspace_id())
    .bind(project)
    .bind(user.id)
    .bind(ctx.restricted_guest())
    .bind(id)
    .bind(locked)
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}
async fn archive(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
) -> ApiResult {
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
    let owner = owner.ok_or_else(ApiError::not_found)?;
    if owner != user.id && !ctx.is_admin() && !ctx.ws.is_admin() {
        return Err(ApiError::bad("Only the owner or admin can archive the page"));
    }
    let now = chrono::Utc::now();
    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE user_favorites SET deleted_at=now()
         WHERE entity_type='page' AND entity_identifier=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(project)
    .bind(ctx.workspace_id())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "WITH RECURSIVE descendants AS (
            SELECT id FROM pages WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL
            UNION ALL
            SELECT p.id FROM pages p JOIN descendants d ON p.parent_id=d.id
            WHERE p.workspace_id=$2 AND p.deleted_at IS NULL
         )
         UPDATE pages SET archived_at=$3, updated_at=now(), updated_by_id=$4
         WHERE id IN (SELECT id FROM descendants)",
    )
    .bind(id)
    .bind(ctx.workspace_id())
    .bind(now)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"archived_at": now.to_rfc3339()})))
}
async fn unarchive(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
) -> ApiResult<StatusCode> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    let row: Option<(Uuid, Option<Uuid>)> = sqlx::query_as(&format!(
        "SELECT p.owned_by_id, p.parent_id FROM pages p WHERE {SCOPE} AND p.id=$5"
    ))
    .bind(ctx.workspace_id())
    .bind(project)
    .bind(user.id)
    .bind(ctx.restricted_guest())
    .bind(id)
    .fetch_optional(&st.db)
    .await?;
    let (owner, parent_id) = row.ok_or_else(ApiError::not_found)?;
    if owner != user.id && !ctx.is_admin() && !ctx.ws.is_admin() {
        return Err(ApiError::bad("Only the owner or admin can un archive the page"));
    }
    let mut tx = st.db.begin().await?;
    if let Some(pid) = parent_id {
        let parent_archived: Option<bool> = sqlx::query_scalar(
            "SELECT archived_at IS NOT NULL FROM pages WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL",
        )
        .bind(pid)
        .bind(ctx.workspace_id())
        .fetch_optional(&mut *tx)
        .await?;
        if parent_archived == Some(true) {
            sqlx::query("UPDATE pages SET parent_id=NULL, updated_at=now(), updated_by_id=$2 WHERE id=$1")
                .bind(id)
                .bind(user.id)
                .execute(&mut *tx)
                .await?;
        }
    }
    sqlx::query(
        "WITH RECURSIVE descendants AS (
            SELECT id FROM pages WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL
            UNION ALL
            SELECT p.id FROM pages p JOIN descendants d ON p.parent_id=d.id
            WHERE p.workspace_id=$2 AND p.deleted_at IS NULL
         )
         UPDATE pages SET archived_at=NULL, updated_at=now(), updated_by_id=$3
         WHERE id IN (SELECT id FROM descendants)",
    )
    .bind(id)
    .bind(ctx.workspace_id())
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    no_content()
}
async fn lock(State(st): State<AppState>, user: CurrentUser, Path(path): Path<PagePath>) -> ApiResult<StatusCode> {
    set_locked(st, user, path, true).await
}
async fn unlock(State(st): State<AppState>, user: CurrentUser, Path(path): Path<PagePath>) -> ApiResult<StatusCode> {
    set_locked(st, user, path, false).await
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

async fn favorite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
) -> ApiResult<StatusCode> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    let r = sqlx::query(&format!("INSERT INTO user_favorites (workspace_id,project_id,user_id,entity_type,entity_identifier,created_by_id,updated_by_id) SELECT $1,$2,$3,'page',p.id,$3,$3 FROM pages p WHERE {SCOPE} AND p.id=$5 AND NOT EXISTS (SELECT 1 FROM user_favorites f WHERE f.workspace_id=$1 AND f.project_id=$2 AND f.user_id=$3 AND f.entity_type='page' AND f.entity_identifier=p.id AND f.deleted_at IS NULL)"))
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}
async fn unfavorite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
) -> ApiResult<StatusCode> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    let r=sqlx::query("UPDATE user_favorites SET deleted_at=now() WHERE workspace_id=$1 AND project_id=$2 AND user_id=$3 AND entity_type='page' AND entity_identifier=$4 AND deleted_at IS NULL")
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}
const VERSION: &str = "jsonb_build_object('id',v.id,'workspace',v.workspace_id,'page',v.page_id,'last_saved_at',v.last_saved_at,'owned_by',v.owned_by_id,'created_at',v.created_at,'updated_at',v.updated_at,'created_by',v.created_by_id,'updated_by',v.updated_by_id)";
async fn versions(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
) -> ApiResult {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(sqlx::query_scalar(&format!("SELECT {VERSION} FROM page_versions v JOIN pages p ON p.id=v.page_id AND p.workspace_id=v.workspace_id WHERE {SCOPE} AND p.id=$5 AND v.deleted_at IS NULL ORDER BY v.created_at DESC"))
       .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id),&st.db).await
}
async fn version(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id, version)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(sqlx::query_scalar(&format!("SELECT {VERSION} || jsonb_build_object('description_html',v.description_html,'description_json',v.description_json,'description_binary',CASE WHEN v.description_binary IS NULL THEN NULL ELSE replace(encode(v.description_binary,'base64'), E'\\n', '') END) FROM page_versions v JOIN pages p ON p.id=v.page_id AND p.workspace_id=v.workspace_id WHERE {SCOPE} AND p.id=$5 AND v.id=$6 AND v.deleted_at IS NULL"))
       .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id).bind(version),&st.db).await
}
async fn duplicate(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<PagePath>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ctx = require_project(&st.db, &user, &slug, project).await?;
    ctx.require(&[ADMIN, MEMBER])?;
    let mut tx = st.db.begin().await?;
    let row: Option<(Uuid, Option<String>)> = sqlx::query_as(&format!("INSERT INTO pages (workspace_id,name,color,owned_by_id,created_by_id,updated_by_id,access,description_html,description_stripped,description_json,view_props,logo_props) SELECT p.workspace_id,p.name || ' (Copy)',p.color,$3,$3,$3,p.access,p.description_html,p.description_stripped,p.description_json,p.view_props,p.logo_props FROM pages p WHERE {SCOPE} AND p.id=$5 RETURNING id,description_html"))
        .bind(ctx.workspace_id()).bind(project).bind(user.id).bind(ctx.restricted_guest()).bind(id).fetch_optional(&mut *tx).await?;
    let (new_id, copied_html) = row.ok_or_else(ApiError::not_found)?;
    sqlx::query("INSERT INTO project_pages (workspace_id,project_id,page_id,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$4)").bind(ctx.workspace_id()).bind(project).bind(new_id).bind(user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    crate::side_effects::spawn_copy_page_assets(st.clone(), ctx.workspace_id(), project, new_id, user.id);
    crate::side_effects::spawn_page_transaction(st.clone(), ctx.workspace_id(), new_id, copied_html, None);
    let Json(v) = fetch(&st, &user, &slug, project, new_id).await?;
    Ok((StatusCode::CREATED, Json(v)))
}

/// Active collaborative page rooms, synchronized across multiple Saturn processes via PostgreSQL LISTEN/NOTIFY.
const ROOM_SYNC_CHANNEL: &str = "saturn_live_room";
const ROOM_NOTIFY_MAX_BYTES: usize = 7500;
type Rooms = std::collections::HashMap<Uuid, std::sync::Arc<tokio::sync::Mutex<Room>>>;
struct Room {
    state: Vec<u8>,
    events: tokio::sync::broadcast::Sender<Vec<u8>>,
    dirty_since: Option<tokio::time::Instant>,
    last_change: tokio::time::Instant,
    editor: Uuid,
    workspace_id: Uuid,
    project_id: Uuid,
    restricted: bool,
}
static ROOMS: std::sync::OnceLock<tokio::sync::Mutex<Rooms>> = std::sync::OnceLock::new();
static NODE_ID: std::sync::OnceLock<Uuid> = std::sync::OnceLock::new();

fn node_id() -> Uuid {
    *NODE_ID.get_or_init(Uuid::new_v4)
}

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct RoomSyncMsg {
    node: Uuid,
    page: Uuid,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<String>,
}

fn encode_room_sync(page: Uuid, kind: &str, raw: &[u8]) -> Option<String> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let payload = serde_json::to_string(&RoomSyncMsg {
        node: node_id(),
        page,
        kind: kind.to_string(),
        data: Some(STANDARD.encode(raw)),
    })
    .ok()?;
    (payload.len() <= ROOM_NOTIFY_MAX_BYTES).then_some(payload)
}

fn publish_room_notify(db: sqlx::PgPool, payload: String) {
    if payload.is_empty() {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = sqlx::query("SELECT pg_notify($1, $2)")
            .bind(ROOM_SYNC_CHANNEL)
            .bind(payload)
            .execute(&db)
            .await
        {
            tracing::warn!(error = %e, "failed to publish live room notification");
        }
    });
}

fn merge_yjs_state(current: &[u8], incoming: &[u8]) -> Option<Vec<u8>> {
    use yrs::{Doc, ReadTxn, StateVector, Transact, Update, updates::decoder::Decode};
    let doc = Doc::new();
    if !current.is_empty() {
        let update = Update::decode_v1(current).ok()?;
        doc.transact_mut().apply_update(update).ok()?;
    }
    if !incoming.is_empty() {
        let update = Update::decode_v1(incoming).ok()?;
        doc.transact_mut().apply_update(update).ok()?;
    }
    Some(doc.transact().encode_state_as_update_v1(&StateVector::default()))
}

async fn apply_room_sync(st: &AppState, msg: RoomSyncMsg) {
    use base64::{Engine, engine::general_purpose::STANDARD};
    if msg.node == node_id() {
        return;
    }
    let Some(rooms) = ROOMS.get() else {
        return;
    };
    let room = {
        let map = rooms.lock().await;
        map.get(&msg.page).cloned()
    };
    let Some(room) = room else {
        return;
    };
    match msg.kind.as_str() {
        "awareness" => {
            let Some(bytes) = msg.data.and_then(|d| STANDARD.decode(d).ok()) else {
                return;
            };
            let _ = room.lock().await.events.send(bytes);
        }
        "sync" => {
            let Some(bytes) = msg.data.and_then(|d| STANDARD.decode(d).ok()) else {
                return;
            };
            let page_str = msg.page.to_string();
            let mut pos = 0;
            if get_buf(&bytes, &mut pos) != Some(page_str.as_bytes()) || get_var(&bytes, &mut pos) != Some(0) {
                return;
            }
            let Some(sub) = get_var(&bytes, &mut pos) else {
                return;
            };
            if sub != 1 && sub != 2 {
                return;
            }
            let Some(payload) = get_buf(&bytes, &mut pos) else {
                return;
            };
            let mut r = room.lock().await;
            if let Some(merged) = merge_yjs_state(&r.state, payload) {
                r.state = merged;
                let _ = r.events.send(bytes);
            }
        }
        "reload" => {
            let binary: Option<Vec<u8>> =
                match sqlx::query_scalar("SELECT description_binary FROM pages WHERE id = $1 AND deleted_at IS NULL")
                    .bind(msg.page)
                    .fetch_optional(&st.db)
                    .await
                {
                    Ok(Some(v)) => v,
                    _ => return,
                };
            let Some(bin) = binary.filter(|b| !b.is_empty()) else {
                return;
            };
            let mut r = room.lock().await;
            if let Some(merged) = merge_yjs_state(&r.state, &bin) {
                r.state = merged.clone();
                let sync_frame = frame(&msg.page.to_string(), 0, Some(2), &merged);
                let _ = r.events.send(sync_frame);
            }
        }
        _ => {}
    }
}

/// Listen for cross-process collaborative room updates over PostgreSQL `LISTEN/NOTIFY`.
pub fn spawn_room_sync(st: AppState) {
    tokio::spawn(async move {
        loop {
            match sqlx::postgres::PgListener::connect_with(&st.db).await {
                Ok(mut listener) => {
                    if let Err(e) = listener.listen(ROOM_SYNC_CHANNEL).await {
                        tracing::warn!(error = %e, "failed to subscribe to live room channel");
                    } else {
                        loop {
                            match listener.recv().await {
                                Ok(notif) => match serde_json::from_str::<RoomSyncMsg>(notif.payload()) {
                                    Ok(msg) => apply_room_sync(&st, msg).await,
                                    Err(e) => tracing::warn!(error = %e, "invalid live room sync notification"),
                                },
                                Err(e) => {
                                    tracing::warn!(error = %e, "live room listener disconnected; reconnecting");
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "failed to connect live room listener"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

/// Flush all dirty collaborative page rooms on graceful shutdown.
pub async fn flush_all_rooms(st: &AppState) {
    let Some(rooms) = ROOMS.get() else {
        return;
    };
    let entries: Vec<(Uuid, std::sync::Arc<tokio::sync::Mutex<Room>>)> = {
        let map = rooms.lock().await;
        map.iter().map(|(&id, r)| (id, r.clone())).collect()
    };
    for (id, room) in entries {
        let mut r = room.lock().await;
        if r.dirty_since.is_some() {
            let ws = r.workspace_id;
            let pid = r.project_id;
            let restricted = r.restricted;
            if let Err(e) = persist_live(st, pid, ws, restricted, id, &mut r).await {
                tracing::warn!(page = %id, error = %e, "failed to flush live page during shutdown");
            }
        }
    }
}

async fn resolve_live_access(
    db: &sqlx::PgPool,
    slug: &str,
    page_id: Uuid,
    user_id: Uuid,
    project_filter: Option<Uuid>,
    action: &str,
) -> Result<Option<(Uuid, Uuid, bool, bool)>, ApiError> {
    let rows: Vec<(Uuid, Uuid, i16, Uuid, i16, bool)> = sqlx::query_as(
        "SELECT pp.workspace_id, pp.project_id, pm.role, p.owned_by_id, p.access, pr.guest_view_all_features
         FROM project_pages pp
         JOIN pages p ON p.id = pp.page_id AND p.workspace_id = pp.workspace_id AND p.deleted_at IS NULL
         JOIN projects pr ON pr.id = pp.project_id AND pr.workspace_id = pp.workspace_id AND pr.deleted_at IS NULL
         JOIN workspaces w ON w.id = pp.workspace_id AND w.deleted_at IS NULL
         JOIN project_members pm ON pm.project_id = pr.id AND pm.workspace_id = w.id
              AND pm.member_id = $4 AND pm.is_active AND pm.deleted_at IS NULL
         WHERE pp.page_id = $1 AND w.slug = $2 AND ($3::uuid IS NULL OR pp.project_id = $3) AND pp.deleted_at IS NULL",
    )
    .bind(page_id)
    .bind(slug)
    .bind(project_filter)
    .bind(user_id)
    .fetch_all(db)
    .await?;

    let mut read_only_candidate = None;
    for (ws_id, pid, role, owner_id, access, guest_view_all) in rows {
        let is_owner = owner_id == user_id;
        if !is_owner && (access == 1 || (role == GUEST && !guest_view_all)) {
            continue;
        }
        let read_only = !is_owner && role == GUEST;
        let restricted = role == GUEST && !guest_view_all;
        if action == "edit" {
            if read_only {
                continue;
            }
            return Ok(Some((ws_id, pid, false, restricted)));
        }
        if !read_only {
            return Ok(Some((ws_id, pid, false, restricted)));
        }
        if read_only_candidate.is_none() {
            read_only_candidate = Some((ws_id, pid, true, restricted));
        }
    }
    Ok(read_only_candidate)
}

/// Plane `PageLiveAccessEndpoint`: `GET /api/workspaces/{slug}/pages/{page_id}/live-access/?action=view|edit`.
async fn live_access(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, page_id)): Path<(String, Uuid)>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let action = q.get("action").map(String::as_str).unwrap_or("");
    if !matches!(action, "view" | "edit") {
        return Err(ApiError::bad("action must be view or edit"));
    }
    let Some((_, project_id, read_only, _)) =
        resolve_live_access(&st.db, &slug, page_id, user.id, None, action).await?
    else {
        return Err(ApiError(StatusCode::NOT_FOUND, json!({"error": "Page not found"})));
    };
    if action == "edit" {
        Ok(Json(json!({"project_id": project_id})))
    } else {
        Ok(Json(json!({"project_id": project_id, "read_only": read_only})))
    }
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveQuery {
    workspace_slug: String,
    project_id: Option<Uuid>,
    document_type: Option<String>,
}

async fn collaboration(
    State(st): State<AppState>,
    user: CurrentUser,
    headers: axum::http::HeaderMap,
    axum::extract::Query(q): axum::extract::Query<LiveQuery>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(origin) = headers.get(header::ORIGIN) {
        let expected = url::Url::parse(&st.cfg.web_url).map_err(|_| ApiError::forbidden())?;
        let actual = origin
            .to_str()
            .ok()
            .and_then(|v| url::Url::parse(v).ok())
            .ok_or_else(ApiError::forbidden)?;
        if actual.origin() != expected.origin() {
            return Err(ApiError::forbidden());
        }
    }
    if let Some(pid) = q.project_id {
        let ctx = require_project(&st.db, &user, &q.workspace_slug, pid).await?;
        ctx.require(&[ADMIN, MEMBER, GUEST])?;
    } else {
        require_workspace(&st.db, &user, &q.workspace_slug).await?;
    }
    if q.document_type.as_deref().is_some_and(|t| t != "project_page") {
        // Plane Community apps/live only exposes TDocumentTypes = "project_page".
        return Err(ApiError::bad("Unsupported document type"));
    }
    Ok(ws
        .max_message_size(10 * 1024 * 1024)
        .on_upgrade(move |socket| live_socket(socket, st, user, q)))
}

fn put_var(mut n: usize, out: &mut Vec<u8>) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn get_var(data: &[u8], pos: &mut usize) -> Option<usize> {
    let mut n = 0usize;
    for i in 0..10 {
        let b = *data.get(*pos)?;
        *pos += 1;
        n |= ((b & 127) as usize).checked_shl(i * 7)?;
        if b & 128 == 0 {
            return Some(n);
        }
    }
    None
}
fn get_buf<'a>(data: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let len = get_var(data, pos)?;
    let end = pos.checked_add(len)?;
    let v = data.get(*pos..end)?;
    *pos = end;
    Some(v)
}
fn put_buf(data: &[u8], out: &mut Vec<u8>) {
    put_var(data.len(), out);
    out.extend_from_slice(data);
}
fn frame(name: &str, typ: usize, subtype: Option<usize>, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    put_buf(name.as_bytes(), &mut out);
    put_var(typ, &mut out);
    if let Some(t) = subtype {
        put_var(t, &mut out);
    }
    put_buf(payload, &mut out);
    out
}

async fn persist_live(
    st: &AppState,
    project_id: Uuid,
    workspace: Uuid,
    restricted: bool,
    id: Uuid,
    room: &mut Room,
) -> anyhow::Result<()> {
    let converted = match crate::live_doc::formats(&room.state) {
        Ok((html, json, title)) => match sanitize_html(&html) {
            Ok(html) => Some((html, json, title)),
            Err(_) => {
                tracing::warn!(page = %id, "invalid live document HTML; preserving binary");
                None
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, page = %id, "unsupported live document; preserving binary");
            None
        }
    };
    let mut tx = st.db.begin().await?;
    let previous: Option<String> = sqlx::query_scalar(&format!("SELECT p.description_html FROM pages p WHERE {SCOPE} AND p.id=$5 AND NOT p.is_locked AND p.archived_at IS NULL FOR UPDATE"))
        .bind(workspace).bind(project_id).bind(room.editor).bind(restricted).bind(id).fetch_optional(&mut *tx).await?;
    let previous = previous.ok_or_else(|| anyhow::anyhow!("page no longer writable"))?;
    let Some((html, json, title)) = converted else {
        sqlx::query(&format!("UPDATE pages p SET description_binary=$6,updated_by_id=$3,updated_at=now() WHERE {SCOPE} AND p.id=$5 AND NOT p.is_locked AND p.archived_at IS NULL"))
            .bind(workspace).bind(project_id).bind(room.editor).bind(restricted).bind(id).bind(&room.state).execute(&mut *tx).await?;
        tx.commit().await?;
        room.dirty_since = None;
        return Ok(());
    };
    let stripped = strip_tags(&html);
    let title = title.filter(|s| !s.trim().is_empty());
    sqlx::query(&format!("UPDATE pages p SET description_binary=$6,description_html=$7,description_json=$8,description_stripped=$9,name=COALESCE($10,p.name),updated_by_id=$3,updated_at=now() WHERE {SCOPE} AND p.id=$5 AND NOT p.is_locked AND p.archived_at IS NULL"))
        .bind(workspace).bind(project_id).bind(room.editor).bind(restricted).bind(id)
        .bind(&room.state).bind(&html).bind(&json).bind(&stripped).bind(&title).execute(&mut *tx).await?;
    if previous != html {
        let latest: Option<(Uuid, Uuid, bool)> = sqlx::query_as("SELECT id,owned_by_id,last_saved_at >= now() - interval '10 minutes' FROM page_versions WHERE workspace_id=$1 AND page_id=$2 AND deleted_at IS NULL ORDER BY last_saved_at DESC LIMIT 1")
            .bind(workspace).bind(id).fetch_optional(&mut *tx).await?;
        if let Some((version, owner, true)) = latest.filter(|(_, owner, _)| *owner == room.editor) {
            let _ = owner;
            sqlx::query("UPDATE page_versions SET description_binary=$3,description_html=$4,description_json=$5,description_stripped=$6,sub_pages_data='{}'::jsonb,updated_at=now() WHERE workspace_id=$1 AND page_id=$2 AND id=$7 AND deleted_at IS NULL")
                .bind(workspace).bind(id).bind(&room.state).bind(&html).bind(&json).bind(&stripped).bind(version).execute(&mut *tx).await?;
        } else {
            sqlx::query("INSERT INTO page_versions (workspace_id,page_id,owned_by_id,last_saved_at,description_binary,description_html,description_json,description_stripped) VALUES ($1,$2,$3,now(),$4,$5,$6,$7)")
                .bind(workspace).bind(id).bind(room.editor).bind(&room.state).bind(&html).bind(&json).bind(&stripped).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE page_versions SET deleted_at=now() WHERE workspace_id=$1 AND page_id=$2 AND deleted_at IS NULL AND id IN (SELECT id FROM page_versions WHERE workspace_id=$1 AND page_id=$2 AND deleted_at IS NULL ORDER BY last_saved_at DESC,id DESC OFFSET 20)")
            .bind(workspace).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if previous != html {
        // Plane: the live server PATCHes the page, which queues page_transaction (page_logs).
        crate::side_effects::spawn_page_transaction(st.clone(), workspace, id, Some(html), Some(previous));
    }
    room.dirty_since = None;
    Ok(())
}

async fn live_socket(mut socket: axum::extract::ws::WebSocket, st: AppState, user: CurrentUser, q: LiveQuery) {
    use axum::extract::ws::Message;
    use yrs::{Doc, ReadTxn, StateVector, Transact, Update, updates::decoder::Decode};
    let first = match socket.recv().await {
        Some(Ok(Message::Binary(b))) => b,
        _ => return,
    };
    let mut pos = 0;
    let Some(name) = get_buf(&first, &mut pos).and_then(|s| std::str::from_utf8(s).ok()) else {
        return;
    };
    let Ok(id) = Uuid::parse_str(name) else { return };
    if get_var(&first, &mut pos) != Some(2)
        || get_var(&first, &mut pos) != Some(0)
        || get_buf(&first, &mut pos).is_none()
    {
        return;
    }
    let Ok(Some((workspace, project_id, initial_read_only, restricted))) =
        resolve_live_access(&st.db, &q.workspace_slug, id, user.id, q.project_id, "view").await
    else {
        return;
    };
    let existing: Option<(Option<Vec<u8>>, String, String, bool)> = match sqlx::query_as(&format!(
        "SELECT p.description_binary, p.description_html, p.name, (p.is_locked OR p.archived_at IS NOT NULL)
         FROM pages p WHERE {SCOPE} AND p.id=$5"
    ))
    .bind(workspace)
    .bind(project_id)
    .bind(user.id)
    .bind(restricted)
    .bind(id)
    .fetch_optional(&st.db)
    .await
    {
        Ok(v) => v,
        Err(_) => return,
    };
    let Some((binary, html, title, locked_or_archived)) = existing else {
        return;
    };
    let read_only = initial_read_only || locked_or_archived;
    let rooms = ROOMS.get_or_init(|| tokio::sync::Mutex::new(Rooms::new()));
    let room = {
        let mut map = rooms.lock().await;
        if let Some(room) = map.get(&id) {
            room.clone()
        } else {
            let state = match binary.filter(|b| !b.is_empty()) {
                Some(binary) => binary,
                None => {
                    let Ok(state) = crate::live_doc::initialize(&html, &title) else {
                        return;
                    };
                    if !read_only {
                        let result = sqlx::query(&format!("UPDATE pages p SET description_binary=$6 WHERE {SCOPE} AND p.id=$5 AND NOT p.is_locked AND p.archived_at IS NULL AND (p.description_binary IS NULL OR octet_length(p.description_binary)=0)"))
                            .bind(workspace).bind(project_id).bind(user.id).bind(restricted).bind(id).bind(&state).execute(&st.db).await;
                        if !matches!(result, Ok(ref r) if r.rows_affected()==1) {
                            return;
                        }
                    }
                    state
                }
            };
            let (events, _) = tokio::sync::broadcast::channel(256);
            let room = std::sync::Arc::new(tokio::sync::Mutex::new(Room {
                state,
                events,
                dirty_since: None,
                last_change: tokio::time::Instant::now(),
                editor: user.id,
                workspace_id: workspace,
                project_id,
                restricted,
            }));
            map.insert(id, room.clone());
            room
        }
    };
    let mut rx = room.lock().await.events.subscribe();
    let mode_label: &[u8] = if read_only { b"read-only" } else { b"read-write" };
    if socket
        .send(Message::Binary(frame(name, 2, Some(2), mode_label).into()))
        .await
        .is_err()
    {
        return;
    }
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let mut r = room.lock().await;
                if r.dirty_since.is_some_and(|since| since.elapsed() >= std::time::Duration::from_secs(10) || r.last_change.elapsed() >= std::time::Duration::from_secs(2))
                    && let Err(e) = persist_live(&st, project_id, workspace, restricted, id, &mut r).await {
                        tracing::error!(error = %e, page = %id, "failed to persist live page");
                    }
            }
            incoming=socket.recv()=> {
                let Some(Ok(Message::Binary(data)))=incoming else { break };
                let Ok(Some((_, _, ro_now, _))) =
                    resolve_live_access(&st.db, &q.workspace_slug, id, user.id, Some(project_id), "view").await
                else {
                    break;
                };
                let mut p=0; if get_buf(&data,&mut p)!=Some(name.as_bytes()) { break; }
                let Some(typ)=get_var(&data,&mut p) else { break };
                if typ==1 {
                    if !read_only && !ro_now {
                        let writable: bool = sqlx::query_scalar(&format!("SELECT EXISTS(SELECT 1 FROM pages p WHERE {SCOPE} AND p.id=$5 AND NOT p.is_locked AND p.archived_at IS NULL)"))
                            .bind(workspace).bind(project_id).bind(user.id).bind(restricted).bind(id).fetch_one(&st.db).await.unwrap_or(false);
                        if writable {
                            let raw = data.to_vec();
                            let _=room.lock().await.events.send(raw.clone());
                            if let Some(payload) = encode_room_sync(id, "awareness", &raw) {
                                publish_room_notify(st.db.clone(), payload);
                            }
                        }
                    }
                    continue;
                }
                if typ!=0 { continue; }
                let Some(sub)=get_var(&data,&mut p) else { break }; let Some(payload)=get_buf(&data,&mut p) else { break };
                let mut room=room.lock().await;
                let doc=Doc::new();
                if !room.state.is_empty() { let Ok(update)=Update::decode_v1(&room.state) else { break }; if doc.transact_mut().apply_update(update).is_err() { break; } }
                if sub==0 {
                    let Ok(vector)=StateVector::decode_v1(payload) else { break };
                    let update=doc.transact().encode_state_as_update_v1(&vector);
                    let vector=yrs::updates::encoder::Encode::encode_v1(&doc.transact().state_vector());
                    if socket.send(Message::Binary(frame(name,0,Some(1),&update).into())).await.is_err() { break; }
                    if socket.send(Message::Binary(frame(name,0,Some(0),&vector).into())).await.is_err() { break; }
                } else if sub==1 || sub==2 {
                    if read_only || ro_now {
                        continue;
                    }
                    let writable: bool = sqlx::query_scalar(&format!("SELECT EXISTS(SELECT 1 FROM pages p WHERE {SCOPE} AND p.id=$5 AND NOT p.is_locked AND p.archived_at IS NULL)"))
                        .bind(workspace).bind(project_id).bind(user.id).bind(restricted).bind(id).fetch_one(&st.db).await.unwrap_or(false);
                    if !writable {
                        continue;
                    }
                    let Some(merged) = merge_yjs_state(&room.state, payload) else { break };
                    room.state = merged;
                    let now = tokio::time::Instant::now();
                    room.dirty_since.get_or_insert(now);
                    room.last_change=now;
                    room.editor=user.id;
                    let raw = data.to_vec();
                    let _=room.events.send(raw.clone());
                    if let Some(sync_payload) = encode_room_sync(id, "sync", &raw) {
                        publish_room_notify(st.db.clone(), sync_payload);
                    } else {
                        if let Err(e) = persist_live(&st, project_id, workspace, restricted, id, &mut room).await {
                            tracing::warn!(page = %id, error = %e, "failed to flush large live page update");
                        }
                        if let Ok(reload_payload) = serde_json::to_string(&RoomSyncMsg {
                            node: node_id(),
                            page: id,
                            kind: "reload".to_string(),
                            data: None,
                        }) {
                            publish_room_notify(st.db.clone(), reload_payload);
                        }
                    }
                }
            },
            event=rx.recv()=> {
                let Ok(Some(_)) =
                    resolve_live_access(&st.db, &q.workspace_slug, id, user.id, Some(project_id), "view").await
                else {
                    break;
                };
                match event { Ok(data)=>if socket.send(Message::Binary(data.into())).await.is_err(){break}, Err(_)=>break }
            }
        }
    }
    drop(rx);
    let mut map = rooms.lock().await;
    if std::sync::Arc::strong_count(&room) == 2 {
        let mut r = room.lock().await;
        if r.dirty_since.is_none()
            || persist_live(&st, project_id, workspace, restricted, id, &mut r)
                .await
                .is_ok()
        {
            map.remove(&id);
        } else {
            tracing::error!(page = %id, "failed to flush live page on disconnect; retaining room state");
            let retry_room = room.clone();
            let retry_state = st.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let mut map = rooms.lock().await;
                    let mut room = retry_room.lock().await;
                    if room.dirty_since.is_none()
                        || persist_live(&retry_state, project_id, workspace, restricted, id, &mut room)
                            .await
                            .is_ok()
                    {
                        if std::sync::Arc::strong_count(&retry_room) == 2 {
                            map.remove(&id);
                        }
                        break;
                    }
                    tracing::warn!(page = %id, "retrying live page flush");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_sync_roundtrip_and_size_limit() {
        let page = Uuid::new_v4();
        let small = encode_room_sync(page, "sync", b"hello-yjs").expect("small frame fits");
        let decoded: RoomSyncMsg = serde_json::from_str(&small).unwrap();
        assert_eq!(decoded.node, node_id());
        assert_eq!(decoded.page, page);
        assert_eq!(decoded.kind, "sync");
        assert!(decoded.data.is_some());

        let oversized = vec![b'x'; ROOM_NOTIFY_MAX_BYTES];
        assert!(encode_room_sync(page, "sync", &oversized).is_none());
    }

    #[test]
    fn merge_yjs_state_combines_updates() {
        let s1 = crate::live_doc::initialize("<p>Alpha</p>", "Doc").unwrap();
        let s2 = crate::live_doc::initialize("<p>Beta</p>", "Doc").unwrap();
        let merged = merge_yjs_state(&s1, &s2).expect("valid yjs states merge");
        assert!(!merged.is_empty());
        assert!(crate::live_doc::formats(&merged).is_ok());
    }
}
