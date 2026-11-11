use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};

use crate::common::*;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

const USER_JSON: &str = "jsonb_build_object('id', u.id, 'first_name', u.first_name, 'last_name', u.last_name,
    'email', u.email, 'avatar', u.avatar, 'avatar_url',
    CASE WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id || '/' ELSE NULLIF(u.avatar, '') END,
    'display_name', u.display_name)";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/users/me/", get(me))
        .route("/api/v1/workspaces/{slug}/members/", get(workspace_members))
        .route(
            "/api/v1/workspaces/{slug}/projects/",
            get(projects).post(create_project),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/",
            get(project).patch(update_project).delete(delete_project),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/archive/",
            axum::routing::post(archive_project).delete(unarchive_project),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/states/",
            get(states).post(create_state),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/states/{state_id}/",
            get(state).patch(update_state).delete(delete_state),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/labels/",
            get(labels).post(create_label),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/labels/{label_id}/",
            get(label).patch(update_label).delete(delete_label),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/cycles-lite/",
            get(cycle_list),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/cycles/",
            get(v1_cycles).post(v1_create_cycle),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/cycles/{cycle_id}/",
            get(v1_cycle).patch(v1_update_cycle).delete(v1_delete_cycle),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/cycles/{cycle_id}/archive/",
            axum::routing::post(v1_archive_cycle),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/archived-cycles/",
            get(v1_archived_cycles),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/archived-cycles/{cycle_id}/unarchive/",
            axum::routing::delete(v1_unarchive_cycle),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/cycles/{cycle_id}/cycle-issues/",
            get(v1_cycle_issues).post(v1_add_cycle_issues),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/cycles/{cycle_id}/cycle-issues/{issue_id}/",
            get(v1_cycle_issue).delete(v1_delete_cycle_issue),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/cycles/{cycle_id}/transfer-issues/",
            axum::routing::post(v1_transfer_cycle_issues),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/modules/",
            get(v1_modules).post(v1_create_module),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/modules-lite/",
            get(v1_modules_lite),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/modules/{pk}/",
            get(v1_module).patch(v1_update_module).delete(v1_delete_module),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/modules/{pk}/archive/",
            axum::routing::post(v1_archive_module),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/archived-modules/",
            get(v1_archived_modules),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/archived-modules/{pk}/unarchive/",
            axum::routing::delete(v1_unarchive_module),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/modules/{module_id}/module-issues/",
            get(v1_module_issues).post(v1_add_module_issues),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/modules/{module_id}/module-issues/{issue_id}/",
            axum::routing::delete(v1_delete_module_issue),
        )
}

const CYCLE_JSON: &str = "(to_jsonb(c)-'workspace_id'-'project_id'-'owned_by_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('workspace',c.workspace_id,'project',c.project_id,'owned_by',c.owned_by_id,'created_by',c.created_by_id,'updated_by',c.updated_by_id)";

async fn cycle_list_impl(
    st: AppState,
    user: CurrentUser,
    slug: String,
    pid: Uuid,
    q: HashMap<String, String>,
    archived: bool,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let rows:Vec<Value>=sqlx::query_scalar(&format!("SELECT {CYCLE_JSON} FROM cycles c JOIN projects p ON p.id=c.project_id AND p.workspace_id=c.workspace_id WHERE c.project_id=$1 AND c.workspace_id=$2 AND c.deleted_at IS NULL AND (c.archived_at IS NOT NULL)=$3 AND p.deleted_at IS NULL AND p.archived_at IS NULL ORDER BY c.created_at DESC,c.id"))
        .bind(pid).bind(p.workspace_id()).bind(archived).fetch_all(&st.db).await?;
    paginate(rows, &q)
}
async fn cycle_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    cycle_list_impl(st, user, slug, pid, q, false).await
}

async fn set_archived(st: AppState, user: CurrentUser, slug: String, id: Uuid, archive: bool) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let mut tx = st.db.begin().await?;
    let result = sqlx::query("UPDATE projects SET archived_at=CASE WHEN $3 THEN now() ELSE NULL END, updated_at=now(), updated_by_id=$4 WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL")
        .bind(id).bind(p.workspace_id()).bind(archive).bind(user.id).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    if archive {
        sqlx::query(
            "UPDATE user_favorites SET deleted_at=now() WHERE project_id=$1 AND workspace_id=$2 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(p.workspace_id())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    no_content()
}

async fn create_project(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let name = s(&b, "name")
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 255)
        .ok_or_else(|| ApiError::bad("Invalid name"))?;
    let identifier = s(&b, "identifier")
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 12 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .ok_or_else(|| ApiError::bad("Invalid identifier"))?;
    for field in ["project_lead", "default_assignee"] {
        if let Some(v) = b.get(field).filter(|v| !v.is_null()) {
            let id = v
                .as_str()
                .and_then(|v| Uuid::parse_str(v).ok())
                .ok_or_else(|| ApiError::bad("Invalid workspace member"))?;
            let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM workspace_members WHERE workspace_id=$1 AND member_id=$2 AND is_active AND deleted_at IS NULL)").bind(ws.workspace_id).bind(id).fetch_one(&st.db).await?;
            if !exists {
                return Err(ApiError::bad("Invalid workspace member"));
            }
        }
    }
    let mut tx = st.db.begin().await?;
    let id = crate::projects::create_project(&mut tx, ws.workspace_id, user.id, name, identifier, &b).await?;
    tx.commit().await?;
    let Json(value) = project(State(st), user, Path((slug, id))).await?;
    created(value)
}

async fn update_project(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN])?;
    if b.get("name")
        .is_some_and(|v| v.as_str().is_none_or(|x| x.trim().is_empty() || x.len() > 255))
    {
        return Err(ApiError::bad("Invalid name"));
    }
    let row = sqlx::query("UPDATE projects SET name=COALESCE($3,name),description=COALESCE($4,description),updated_by_id=$5,updated_at=now() WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL AND archived_at IS NULL")
        .bind(id).bind(p.workspace_id()).bind(s(&b,"name")).bind(s(&b,"description")).bind(user.id).execute(&st.db).await?;
    if row.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    project(State(st), user, Path((slug, id))).await
}

async fn delete_project(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN])?;
    let row=sqlx::query("UPDATE projects SET deleted_at=now(),updated_at=now(),updated_by_id=$3 WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL").bind(id).bind(p.workspace_id()).bind(user.id).execute(&st.db).await?;
    if row.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn archive_project(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    set_archived(st, user, slug, id, true).await
}

async fn unarchive_project(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    set_archived(st, user, slug, id, false).await
}

const STATE_JSON: &str = "(to_jsonb(s) - 'workspace_id' - 'project_id' - 'created_by_id' - 'updated_by_id') || jsonb_build_object('workspace',s.workspace_id,'project',s.project_id,'created_by',s.created_by_id,'updated_by',s.updated_by_id)";
const LABEL_JSON: &str = "(to_jsonb(l) - 'workspace_id' - 'project_id' - 'created_by_id' - 'updated_by_id' - 'parent_id') || jsonb_build_object('workspace',l.workspace_id,'project',l.project_id,'created_by',l.created_by_id,'updated_by',l.updated_by_id,'parent',l.parent_id)";
const PROJECT_JSON: &str = "(to_jsonb(p) - 'workspace_id' - 'created_by_id' - 'updated_by_id' - 'default_assignee_id' - 'project_lead_id' - 'estimate_id' - 'default_state_id' - 'cover_image_asset_id') || jsonb_build_object('workspace',p.workspace_id,'created_by',p.created_by_id,'updated_by',p.updated_by_id,'default_assignee',p.default_assignee_id,'project_lead',p.project_lead_id,'estimate',p.estimate_id,'default_state',p.default_state_id,'cover_image_asset',p.cover_image_asset_id,'cover_image_url',p.cover_image,'is_member',pm.id IS NOT NULL,'member_role',pm.role,'sort_order',pup.sort_order,'total_members',(SELECT count(*) FROM project_members m WHERE m.workspace_id=p.workspace_id AND m.project_id=p.id AND m.is_active AND m.deleted_at IS NULL),'total_cycles',(SELECT count(*) FROM cycles c WHERE c.workspace_id=p.workspace_id AND c.project_id=p.id AND c.deleted_at IS NULL),'total_modules',(SELECT count(*) FROM modules m WHERE m.workspace_id=p.workspace_id AND m.project_id=p.id AND m.deleted_at IS NULL),'is_deployed',EXISTS(SELECT 1 FROM deploy_boards d WHERE d.workspace_id=p.workspace_id AND d.project_id=p.id AND d.deleted_at IS NULL))";

fn check_state(b: &Value) -> ApiResult<()> {
    if let Some(group) = b.get("group") {
        if !["backlog", "unstarted", "started", "completed", "cancelled"].contains(&group.as_str().unwrap_or("")) {
            return Err(ApiError::bad("Invalid state group"));
        }
    }
    for key in ["name", "color"] {
        if b.get(key)
            .is_some_and(|v| v.as_str().is_none_or(|s| s.trim().is_empty() || s.len() > 255))
        {
            return Err(ApiError::bad(format!("Invalid {key}")));
        }
    }
    Ok(())
}

async fn create_state(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN])?;
    check_state(&b)?;
    let name = s(&b, "name").ok_or_else(|| ApiError::bad("Name required"))?;
    let color = s(&b, "color").ok_or_else(|| ApiError::bad("Color required"))?;
    let mut tx = st.db.begin().await?;
    let id: Uuid=sqlx::query_scalar("INSERT INTO states (name,color,slug,\"group\",description,project_id,workspace_id,created_by_id,updated_by_id,sequence,\"default\",external_id,external_source) VALUES ($1,$2,$3,$4,COALESCE($5,''),$6,$7,$8,$8,COALESCE($9,(SELECT COALESCE(max(sequence),50535)+15000 FROM states WHERE project_id=$6 AND workspace_id=$7 AND deleted_at IS NULL)),$10,$11,$12) RETURNING id")
        .bind(name).bind(color).bind(crate::states::slugify(name)).bind(s(&b,"group").unwrap_or("backlog")).bind(s(&b,"description")).bind(pid).bind(p.workspace_id()).bind(user.id).bind(b.get("sequence").and_then(Value::as_f64)).bind(b.get("default").and_then(Value::as_bool).unwrap_or(false)).bind(s(&b,"external_id")).bind(s(&b,"external_source")).fetch_one(&mut *tx).await?;
    if b.get("default").and_then(Value::as_bool) == Some(true) {
        sqlx::query("UPDATE states SET \"default\"=false WHERE project_id=$1 AND workspace_id=$2 AND id<>$3 AND deleted_at IS NULL").bind(pid).bind(p.workspace_id()).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    state(State(st), user, Path((slug, pid, id))).await
}

async fn update_state(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN])?;
    check_state(&b)?;
    let mut tx = st.db.begin().await?;
    let result=sqlx::query("UPDATE states SET name=COALESCE($4,name),color=COALESCE($5,color),\"group\"=COALESCE($6,\"group\"),description=COALESCE($7,description),sequence=COALESCE($8,sequence),\"default\"=COALESCE($9,\"default\"),updated_by_id=$10,updated_at=now() WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND NOT is_triage")
        .bind(id).bind(pid).bind(p.workspace_id()).bind(s(&b,"name")).bind(s(&b,"color")).bind(s(&b,"group")).bind(s(&b,"description")).bind(b.get("sequence").and_then(Value::as_f64)).bind(b.get("default").and_then(Value::as_bool)).bind(user.id).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    if b.get("default").and_then(Value::as_bool) == Some(true) {
        sqlx::query("UPDATE states SET \"default\"=false WHERE project_id=$1 AND workspace_id=$2 AND id<>$3 AND deleted_at IS NULL").bind(pid).bind(p.workspace_id()).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    state(State(st), user, Path((slug, pid, id))).await
}

async fn delete_state(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN])?;
    let default:bool=sqlx::query_scalar("SELECT \"default\" FROM states WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND NOT is_triage").bind(id).bind(pid).bind(p.workspace_id()).fetch_one(&st.db).await?;
    if default {
        return Err(ApiError::bad("Default state cannot be deleted"));
    }
    let used:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM issues WHERE state_id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)").bind(id).bind(pid).bind(p.workspace_id()).fetch_one(&st.db).await?;
    if used {
        return Err(ApiError::bad("The state is not empty"));
    }
    sqlx::query("UPDATE states SET deleted_at=now(),updated_by_id=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL").bind(id).bind(pid).bind(p.workspace_id()).bind(user.id).execute(&st.db).await?;
    no_content()
}

async fn validate_label_parent(
    st: &AppState,
    b: &Value,
    pid: Uuid,
    wid: Uuid,
    id: Option<Uuid>,
) -> ApiResult<Option<Uuid>> {
    let Some(value) = b.get("parent").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let parent = value
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| ApiError::bad("Invalid parent"))?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM labels WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
    )
    .bind(parent)
    .bind(pid)
    .bind(wid)
    .fetch_one(&st.db)
    .await?;
    if !exists || Some(parent) == id {
        return Err(ApiError::bad("Invalid parent"));
    }
    Ok(Some(parent))
}

async fn create_label(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN])?;
    let name = s(&b, "name")
        .filter(|v| !v.trim().is_empty() && v.len() <= 255)
        .ok_or_else(|| ApiError::bad("Invalid name"))?;
    let color = s(&b, "color")
        .filter(|v| !v.is_empty() && v.len() <= 255)
        .ok_or_else(|| ApiError::bad("Invalid color"))?;
    let parent = validate_label_parent(&st, &b, pid, p.workspace_id(), None).await?;
    let id:Uuid=sqlx::query_scalar("INSERT INTO labels (name,color,description,parent_id,project_id,workspace_id,created_by_id,updated_by_id,external_id,external_source,sort_order) VALUES ($1,$2,COALESCE($3,''),$4,$5,$6,$7,$7,$8,$9,COALESCE($10,65535)) RETURNING id")
        .bind(name).bind(color).bind(s(&b,"description")).bind(parent).bind(pid).bind(p.workspace_id()).bind(user.id).bind(s(&b,"external_id")).bind(s(&b,"external_source")).bind(b.get("sort_order").and_then(Value::as_f64)).fetch_one(&st.db).await?;
    let Json(value) = label(State(st), user, Path((slug, pid, id))).await?;
    created(value)
}

async fn update_label(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN])?;
    for key in ["name", "color"] {
        if b.get(key)
            .is_some_and(|v| v.as_str().is_none_or(|s| s.trim().is_empty() || s.len() > 255))
        {
            return Err(ApiError::bad(format!("Invalid {key}")));
        }
    }
    let parent = validate_label_parent(&st, &b, pid, p.workspace_id(), Some(id)).await?;
    let row=sqlx::query("UPDATE labels SET name=COALESCE($4,name),color=COALESCE($5,color),description=COALESCE($6,description),parent_id=CASE WHEN $7 THEN $8 ELSE parent_id END,sort_order=COALESCE($9,sort_order),external_id=CASE WHEN $10 THEN $11 ELSE external_id END,external_source=CASE WHEN $12 THEN $13 ELSE external_source END,updated_by_id=$14,updated_at=now() WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
        .bind(id).bind(pid).bind(p.workspace_id()).bind(s(&b,"name")).bind(s(&b,"color")).bind(s(&b,"description")).bind(b.get("parent").is_some()).bind(parent).bind(b.get("sort_order").and_then(Value::as_f64)).bind(b.get("external_id").is_some()).bind(s(&b,"external_id")).bind(b.get("external_source").is_some()).bind(s(&b,"external_source")).bind(user.id).execute(&st.db).await?;
    if row.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    label(State(st), user, Path((slug, pid, id))).await
}

async fn delete_label(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN])?;
    let row=sqlx::query("UPDATE labels SET deleted_at=now(),updated_by_id=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL").bind(id).bind(pid).bind(p.workspace_id()).bind(user.id).execute(&st.db).await?;
    if row.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn projects(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!("SELECT {PROJECT_JSON} FROM projects p LEFT JOIN project_members pm ON pm.project_id=p.id AND pm.workspace_id=p.workspace_id AND pm.member_id=$2 AND pm.is_active AND pm.deleted_at IS NULL LEFT JOIN project_user_properties pup ON pup.project_id=p.id AND pup.workspace_id=p.workspace_id AND pup.user_id=$2 AND pup.deleted_at IS NULL WHERE p.workspace_id=$1 AND p.deleted_at IS NULL AND p.archived_at IS NULL AND (pm.id IS NOT NULL OR p.network=2) ORDER BY p.created_at DESC,p.id"))
        .bind(ws.workspace_id).bind(user.id).fetch_all(&st.db).await?;
    paginate(rows, &q)
}

async fn project(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(sqlx::query_scalar(&format!("SELECT {PROJECT_JSON} FROM projects p LEFT JOIN project_members pm ON pm.project_id=p.id AND pm.workspace_id=p.workspace_id AND pm.member_id=$3 AND pm.is_active AND pm.deleted_at IS NULL LEFT JOIN project_user_properties pup ON pup.project_id=p.id AND pup.workspace_id=p.workspace_id AND pup.user_id=$3 AND pup.deleted_at IS NULL WHERE p.id=$1 AND p.workspace_id=$2 AND p.deleted_at IS NULL AND p.archived_at IS NULL"))
        .bind(project_id).bind(p.workspace_id()).bind(user.id), &st.db).await
}

async fn labels(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!("SELECT {LABEL_JSON} FROM labels l JOIN projects p ON p.id=l.project_id AND p.workspace_id=l.workspace_id WHERE l.project_id=$1 AND l.workspace_id=$2 AND l.deleted_at IS NULL AND p.deleted_at IS NULL AND p.archived_at IS NULL ORDER BY l.created_at DESC,l.id"))
        .bind(project_id).bind(p.workspace_id()).fetch_all(&st.db).await?;
    paginate(rows, &q)
}

async fn label(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, label_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(sqlx::query_scalar(&format!("SELECT {LABEL_JSON} FROM labels l JOIN projects p ON p.id=l.project_id AND p.workspace_id=l.workspace_id WHERE l.id=$1 AND l.project_id=$2 AND l.workspace_id=$3 AND l.deleted_at IS NULL AND p.deleted_at IS NULL AND p.archived_at IS NULL"))
        .bind(label_id).bind(project_id).bind(p.workspace_id()), &st.db).await
}

fn paginate(rows: Vec<Value>, q: &HashMap<String, String>) -> ApiResult {
    let limit = match q.get("per_page") {
        Some(v) => v
            .parse::<usize>()
            .ok()
            .filter(|v| (1..=1000).contains(v))
            .ok_or_else(|| ApiError::bad("Invalid per_page parameter."))?,
        None => 1000,
    };
    let page = match q.get("cursor") {
        Some(cursor) => {
            let parts: Vec<_> = cursor.split(':').collect();
            if parts.len() != 3
                || parts[0]
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 1000.0)
                    .is_none()
                || parts[2].parse::<i64>().is_err()
            {
                return Err(ApiError::bad("Invalid cursor parameter."));
            }
            parts[1]
                .parse::<usize>()
                .map_err(|_| ApiError::bad("Invalid cursor parameter."))?
        }
        None => 0,
    };
    let start = page
        .checked_mul(limit)
        .ok_or_else(|| ApiError::bad("Invalid cursor parameter."))?;
    let total = rows.len();
    let results: Vec<_> = rows.into_iter().skip(start).take(limit).collect();
    Ok(axum::Json(
        json!({"next_cursor": format!("{limit}:{}:0",page.saturating_add(1)), "prev_cursor": format!("{limit}:{}:1",page as i128 - 1), "next_page_results": start.saturating_add(limit) < total, "prev_page_results": page > 0, "count": results.len(), "total_pages": total.div_ceil(limit), "total_results": total, "extra_stats": null, "results": results}),
    ))
}

async fn states(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!("SELECT {STATE_JSON} FROM states s JOIN projects p ON p.id=s.project_id AND p.workspace_id=s.workspace_id WHERE s.project_id=$1 AND s.workspace_id=$2 AND s.deleted_at IS NULL AND NOT s.is_triage AND p.deleted_at IS NULL AND p.archived_at IS NULL ORDER BY s.sequence,s.created_at DESC,s.id"))
        .bind(project_id).bind(p.workspace_id()).fetch_all(&st.db).await?;
    paginate(rows, &q)
}

async fn state(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, state_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(sqlx::query_scalar(&format!("SELECT {STATE_JSON} FROM states s JOIN projects p ON p.id=s.project_id AND p.workspace_id=s.workspace_id WHERE s.id=$1 AND s.project_id=$2 AND s.workspace_id=$3 AND s.deleted_at IS NULL AND NOT s.is_triage AND p.deleted_at IS NULL AND p.archived_at IS NULL"))
        .bind(state_id).bind(project_id).bind(p.workspace_id()), &st.db).await
}

async fn me(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    json_row(
        sqlx::query_scalar(&format!("SELECT {USER_JSON} FROM users u WHERE u.id = $1")).bind(user.id),
        &st.db,
    )
    .await
}

async fn workspace_members(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {USER_JSON} || jsonb_build_object('role', m.role)
        FROM workspace_members m JOIN users u ON u.id = m.member_id
        WHERE m.workspace_id = $1 AND m.deleted_at IS NULL ORDER BY m.created_at, m.id"
        ))
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

const TZ: &str = "(SELECT p.timezone FROM projects p WHERE p.id = $1)";

fn start_sql(p: &str) -> String {
    format!(
        "(CASE WHEN {p}::text IS NULL THEN NULL
          WHEN {p}::date = (now() AT TIME ZONE {TZ})::date THEN now()
          ELSE ((({p}::date)::timestamp + interval '1 second') AT TIME ZONE {TZ}) END)"
    )
}

fn end_sql(p: &str) -> String {
    format!(
        "(CASE WHEN {p}::text IS NULL THEN NULL
          ELSE ((({p}::date)::timestamp + interval '23 hours 59 minutes') AT TIME ZONE {TZ}) END)"
    )
}

fn parse_day(b: &Value, k: &str) -> Result<Option<String>, ApiError> {
    let v = match b.get(k) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(v)) if v.is_empty() => return Ok(None),
        Some(Value::String(v)) => v,
        Some(_) => {
            return Err(ApiError::bad(
                "Date has wrong format. Use one of these formats instead: YYYY-MM-DD.",
            ));
        }
    };
    let day = v.get(..10).unwrap_or(v);
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map_err(|_| ApiError::bad("Date has wrong format. Use one of these formats instead: YYYY-MM-DD."))?;
    Ok(Some(day.to_string()))
}

const CYCLE_DETAIL_JSON: &str = "(to_jsonb(c)-'workspace_id'-'project_id'-'owned_by_id'-'created_by_id'-'updated_by_id'-'deleted_at') || jsonb_build_object(
    'workspace',c.workspace_id,
    'project',c.project_id,
    'owned_by',c.owned_by_id,
    'created_by',c.created_by_id,
    'updated_by',c.updated_by_id,
    'total_issues',COALESCE(k.total, 0),
    'completed_issues',COALESCE(k.completed, 0),
    'cancelled_issues',COALESCE(k.cancelled, 0),
    'started_issues',COALESCE(k.started, 0),
    'unstarted_issues',COALESCE(k.unstarted, 0),
    'backlog_issues',COALESCE(k.backlog, 0),
    'total_estimates',k.total_est,
    'completed_estimates',k.comp_est,
    'started_estimates',k.start_est
)";

const CYCLE_COUNTS_FROM: &str = "LEFT JOIN LATERAL (
    SELECT count(*) AS total,
        count(*) FILTER (WHERE s.group = 'backlog') AS backlog,
        count(*) FILTER (WHERE s.group = 'unstarted') AS unstarted,
        count(*) FILTER (WHERE s.group = 'started') AS started,
        count(*) FILTER (WHERE s.group = 'completed') AS completed,
        count(*) FILTER (WHERE s.group = 'cancelled') AS cancelled,
        sum(ep.key)::float8 AS total_est,
        sum(ep.key) FILTER (WHERE s.group = 'completed')::float8 AS comp_est,
        sum(ep.key) FILTER (WHERE s.group = 'started')::float8 AS start_est
    FROM cycle_issues ci
    JOIN issues i ON i.id = ci.issue_id AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft
    LEFT JOIN states s ON s.id = i.state_id
    LEFT JOIN estimate_points ep ON ep.id = i.estimate_point_id AND ep.deleted_at IS NULL
    WHERE ci.cycle_id = c.id AND ci.deleted_at IS NULL
) k ON true";

const CYCLE_ISSUE_JSON: &str = "(to_jsonb(ci)-'workspace_id'-'project_id'-'cycle_id'-'issue_id'-'created_by_id'-'updated_by_id'-'deleted_at') || jsonb_build_object('workspace',ci.workspace_id,'project',ci.project_id,'cycle',ci.cycle_id,'issue',ci.issue_id,'created_by',ci.created_by_id,'updated_by',ci.updated_by_id,'sub_issues_count',(SELECT count(*) FROM issues sub WHERE sub.parent_id=ci.issue_id AND sub.deleted_at IS NULL AND sub.archived_at IS NULL AND NOT sub.is_draft))";

const ISSUE_JSON: &str = "(to_jsonb(i)-'workspace_id'-'project_id'-'created_by_id'-'updated_by_id'-'state_id'-'parent_id'-'estimate_point_id'-'deleted_at') || jsonb_build_object('workspace',i.workspace_id,'project',i.project_id,'created_by',i.created_by_id,'updated_by',i.updated_by_id,'state',i.state_id,'parent',i.parent_id,'estimate_point',i.estimate_point_id,'sub_issues_count',(SELECT count(*) FROM issues sub WHERE sub.parent_id=i.id AND sub.deleted_at IS NULL AND sub.archived_at IS NULL AND NOT sub.is_draft),'bridge_id',ci.id,'assignees',COALESCE((SELECT jsonb_agg(ia.assignee_id) FROM issue_assignees ia WHERE ia.issue_id=i.id AND ia.deleted_at IS NULL),'[]'::jsonb),'labels',COALESCE((SELECT jsonb_agg(il.label_id) FROM issue_labels il WHERE il.issue_id=i.id AND il.deleted_at IS NULL),'[]'::jsonb))";

async fn v1_cycles(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let view = q.get("cycle_view").map(String::as_str).unwrap_or("all");
    let cond = match view {
        "current" => "AND c.start_date <= now() AND c.end_date >= now()",
        "upcoming" => "AND c.start_date > now()",
        "completed" => "AND c.end_date < now()",
        "draft" => "AND c.start_date IS NULL AND c.end_date IS NULL",
        "incomplete" => "AND (c.end_date >= now() OR c.end_date IS NULL)",
        _ => "",
    };
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {CYCLE_DETAIL_JSON} FROM cycles c {CYCLE_COUNTS_FROM}
         JOIN projects p ON p.id = c.project_id AND p.workspace_id = c.workspace_id
         WHERE c.project_id = $1 AND c.workspace_id = $2 AND c.deleted_at IS NULL AND c.archived_at IS NULL
           AND p.deleted_at IS NULL AND p.archived_at IS NULL {cond}
         ORDER BY c.created_at DESC, c.id"
    ))
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;

    if view == "current" {
        Ok(Json(Value::Array(rows)))
    } else {
        paginate(rows, &q)
    }
}

async fn v1_cycle(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, cid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {CYCLE_DETAIL_JSON} FROM cycles c {CYCLE_COUNTS_FROM}
             WHERE c.id = $1 AND c.project_id = $2 AND c.workspace_id = $3 AND c.deleted_at IS NULL AND c.archived_at IS NULL"
        ))
        .bind(cid)
        .bind(pid)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn v1_create_cycle(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let name = s(&b, "name")
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 255)
        .ok_or_else(|| ApiError::bad("Invalid name"))?;

    let start = parse_day(&b, "start_date")?;
    let end = parse_day(&b, "end_date")?;
    if start.is_some() != end.is_some() {
        return Err(ApiError::bad(
            "Both start date and end date are either required or are to be null",
        ));
    }
    if let (Some(s), Some(e)) = (&start, &end) {
        if s > e {
            return Err(ApiError::bad("Start date cannot exceed end date"));
        }
    }

    if let (Some(ext_id), Some(ext_src)) = (s(&b, "external_id"), s(&b, "external_source")) {
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM cycles WHERE project_id=$1 AND workspace_id=$2 AND external_id=$3 AND external_source=$4 AND deleted_at IS NULL"
        )
        .bind(pid).bind(p.workspace_id()).bind(ext_id).bind(ext_src).fetch_optional(&st.db).await?;
        if let Some(existing_id) = exists {
            return Err(ApiError(
                StatusCode::CONFLICT,
                json!({
                    "error": "Cycle with the same external id and external source already exists",
                    "id": existing_id.to_string()
                }),
            ));
        }
    }

    let owned_by = match b.get("owned_by").and_then(Value::as_str) {
        Some(s) => Uuid::parse_str(s).map_err(|_| ApiError::bad("Invalid owned_by"))?,
        None => user.id,
    };

    let sql = format!(
        "INSERT INTO cycles (id, name, description, start_date, end_date, owned_by_id, created_by_id, updated_by_id,
            project_id, workspace_id, timezone, view_props, logo_props, external_source, external_id, sort_order)
         VALUES (gen_random_uuid(), $4, COALESCE($5, ''), {start}, {end}, $11, $3, $3, $1, $2,
            COALESCE((SELECT p.timezone FROM projects p WHERE p.id = $1), 'UTC'),
            COALESCE($6::jsonb, '{{}}'::jsonb), COALESCE($7::jsonb, '{{}}'::jsonb), $8, $9,
            COALESCE($10::float8, (SELECT COALESCE(max(sort_order), 0) + 10000 FROM cycles WHERE project_id = $1), 65535))
         RETURNING id",
        start = start_sql("$12"),
        end = end_sql("$13"),
    );

    let id: Uuid = sqlx::query_scalar(&sql)
        .bind(pid)
        .bind(p.workspace_id())
        .bind(user.id)
        .bind(name)
        .bind(s(&b, "description"))
        .bind(b.get("view_props").filter(|v| !v.is_null()))
        .bind(b.get("logo_props").filter(|v| !v.is_null()))
        .bind(s(&b, "external_source"))
        .bind(s(&b, "external_id"))
        .bind(b.get("sort_order").and_then(Value::as_f64))
        .bind(owned_by)
        .bind(start)
        .bind(end)
        .fetch_one(&st.db)
        .await?;

    let Json(val) = v1_cycle(State(st), user, Path((slug, pid, id))).await?;
    created(val)
}

async fn v1_update_cycle(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, cid)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let cur: (
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT c.archived_at, c.end_date, c.external_id, c.external_source FROM cycles c
         WHERE c.id = $1 AND c.project_id = $2 AND c.workspace_id = $3 AND c.deleted_at IS NULL",
    )
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;

    if cur.0.is_some() {
        return Err(ApiError::bad("Archived cycle cannot be edited"));
    }

    let completed = cur.1.is_some_and(|e| e < Utc::now());
    if completed && b.get("sort_order").is_none() {
        return Err(ApiError::bad(
            "The Cycle has already been completed so it cannot be edited",
        ));
    }

    if let Some(n) = b.get("name") {
        if n.as_str().is_none_or(|s| s.trim().is_empty() || s.len() > 255) {
            return Err(ApiError::bad("Invalid name"));
        }
    }

    let has_start = b.get("start_date").is_some();
    let has_end = b.get("end_date").is_some();
    let start = parse_day(&b, "start_date")?;
    let end = parse_day(&b, "end_date")?;
    if has_start && has_end {
        if let (Some(s), Some(e)) = (&start, &end) {
            if s > e {
                return Err(ApiError::bad("Start date cannot exceed end date"));
            }
        }
    }

    if let Some(ext_id) = s(&b, "external_id") {
        let ext_src = s(&b, "external_source").or(cur.3.as_deref());
        if cur.2.as_deref() != Some(ext_id) {
            let conflict: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM cycles WHERE project_id=$1 AND workspace_id=$2 AND external_id=$3 AND external_source=$4 AND deleted_at IS NULL AND id <> $5"
            )
            .bind(pid).bind(p.workspace_id()).bind(ext_id).bind(ext_src).bind(cid).fetch_optional(&st.db).await?;
            if let Some(conflict_id) = conflict {
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    json!({
                        "error": "Cycle with the same external id and external source already exists",
                        "id": conflict_id.to_string()
                    }),
                ));
            }
        }
    }

    let has_owned_by = b.get("owned_by").is_some();
    let owned_by = b
        .get("owned_by")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok());

    let sql = format!(
        "UPDATE cycles c SET
            name = COALESCE($5, c.name),
            description = COALESCE($6, c.description),
            start_date = CASE WHEN $7 THEN {start} ELSE c.start_date END,
            end_date = CASE WHEN $9 THEN {end} ELSE c.end_date END,
            view_props = COALESCE($11::jsonb, c.view_props),
            logo_props = COALESCE($12::jsonb, c.logo_props),
            external_source = COALESCE($13, c.external_source),
            external_id = COALESCE($14, c.external_id),
            sort_order = COALESCE($15::float8, c.sort_order),
            owned_by_id = CASE WHEN $16 THEN $17 ELSE c.owned_by_id END,
            updated_by_id = $3, updated_at = now()
         WHERE c.id = $4 AND c.project_id = $1 AND c.workspace_id = $2
           AND c.deleted_at IS NULL AND c.archived_at IS NULL",
        start = start_sql("$8"),
        end = end_sql("$10"),
    );

    let res = sqlx::query(&sql)
        .bind(pid)
        .bind(p.workspace_id())
        .bind(user.id)
        .bind(cid)
        .bind(s(&b, "name"))
        .bind(s(&b, "description"))
        .bind(has_start)
        .bind(start)
        .bind(has_end)
        .bind(end)
        .bind(b.get("view_props").filter(|v| !v.is_null()))
        .bind(b.get("logo_props").filter(|v| !v.is_null()))
        .bind(s(&b, "external_source"))
        .bind(s(&b, "external_id"))
        .bind(b.get("sort_order").and_then(Value::as_f64))
        .bind(has_owned_by)
        .bind(owned_by)
        .execute(&st.db)
        .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }

    v1_cycle(State(st), user, Path((slug, pid, cid))).await
}

async fn v1_delete_cycle(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, cid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    let (created_by_id,): (Option<Uuid>,) = sqlx::query_as(
        "SELECT created_by_id FROM cycles WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;

    if !p.is_admin() && created_by_id != Some(user.id) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            json!({"error": "Only admin or creator can delete the cycle"}),
        ));
    }

    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE cycles SET deleted_at=now(), updated_at=now(), updated_by_id=$2 WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(cid)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE cycle_issues SET deleted_at=now(), updated_at=now(), updated_by_id=$2 WHERE cycle_id=$1 AND deleted_at IS NULL")
        .bind(cid).bind(user.id).execute(&mut *tx).await?;
    sqlx::query("UPDATE user_favorites SET deleted_at=now() WHERE entity_type='cycle' AND entity_identifier=$1 AND project_id=$2 AND deleted_at IS NULL")
        .bind(cid).bind(pid).execute(&mut *tx).await?;
    tx.commit().await?;
    no_content()
}

async fn v1_archive_cycle(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, cid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let (end_date,): (Option<DateTime<Utc>>,) = sqlx::query_as(
        "SELECT end_date FROM cycles WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;

    if end_date.is_none_or(|e| e >= Utc::now()) {
        return Err(ApiError::bad("Only completed cycles can be archived"));
    }

    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE cycles SET archived_at=now(), updated_at=now(), updated_by_id=$2 WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(cid)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE user_favorites SET deleted_at=now() WHERE entity_type='cycle' AND entity_identifier=$1 AND project_id=$2 AND deleted_at IS NULL")
        .bind(cid).bind(pid).execute(&mut *tx).await?;
    tx.commit().await?;
    no_content()
}

async fn v1_archived_cycles(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {CYCLE_DETAIL_JSON} FROM cycles c {CYCLE_COUNTS_FROM}
         WHERE c.project_id=$1 AND c.workspace_id=$2 AND c.deleted_at IS NULL AND c.archived_at IS NOT NULL
         ORDER BY c.created_at DESC, c.id"
    ))
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    paginate(rows, &q)
}

async fn v1_unarchive_cycle(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, cid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let res = sqlx::query(
        "UPDATE cycles SET archived_at=NULL, updated_at=now(), updated_by_id=$4
         WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND archived_at IS NOT NULL",
    )
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(user.id)
    .execute(&st.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn v1_cycle_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, cid)): Path<(String, Uuid, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM cycles WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
    )
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;
    if !exists {
        return Err(ApiError::not_found());
    }

    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {ISSUE_JSON} FROM issues i
         JOIN cycle_issues ci ON ci.issue_id = i.id AND ci.cycle_id = $1 AND ci.deleted_at IS NULL
         WHERE i.project_id = $2 AND i.workspace_id = $3 AND i.deleted_at IS NULL
           AND (NOT $4 OR i.created_by_id = $5)
         ORDER BY i.created_at DESC, i.id"
    ))
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(p.restricted_guest())
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;
    paginate(rows, &q)
}

async fn v1_add_cycle_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, cid)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let issues: Vec<Uuid> = b
        .get("issues")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default();
    if issues.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            json!({"error": "Work items are required", "code": "MISSING_WORK_ITEMS"}),
        ));
    }

    let (end_date,): (Option<DateTime<Utc>>,) = sqlx::query_as(
        "SELECT end_date FROM cycles WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;

    if end_date.is_some_and(|e| e < Utc::now()) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            json!({"code": "CYCLE_COMPLETED", "message": "The Cycle has already been completed so no new issues can be added"}),
        ));
    }

    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE cycle_issues SET cycle_id=$1, updated_by_id=$2, updated_at=now()
         WHERE issue_id = ANY($3) AND project_id=$4 AND workspace_id=$5 AND deleted_at IS NULL AND cycle_id <> $1",
    )
    .bind(cid)
    .bind(user.id)
    .bind(&issues)
    .bind(pid)
    .bind(p.workspace_id())
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO cycle_issues (id, cycle_id, issue_id, project_id, workspace_id, created_by_id, updated_by_id)
         SELECT gen_random_uuid(), $1, i.id, i.project_id, i.workspace_id, $2, $2
         FROM issues i
         WHERE i.id = ANY($3) AND i.project_id = $4 AND i.workspace_id = $5 AND i.deleted_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM cycle_issues ci WHERE ci.issue_id = i.id AND ci.cycle_id = $1 AND ci.deleted_at IS NULL)
         ON CONFLICT DO NOTHING"
    )
    .bind(cid).bind(user.id).bind(&issues).bind(pid).bind(p.workspace_id()).execute(&mut *tx).await?;
    tx.commit().await?;

    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {CYCLE_ISSUE_JSON} FROM cycle_issues ci
         WHERE ci.cycle_id = $1 AND ci.project_id = $2 AND ci.workspace_id = $3 AND ci.deleted_at IS NULL
         ORDER BY ci.created_at DESC, ci.id"
    ))
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    Ok(Json(Value::Array(rows)))
}

async fn v1_cycle_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, cid, iid)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(sqlx::query_scalar(&format!(
        "SELECT {CYCLE_ISSUE_JSON} FROM cycle_issues ci
         WHERE ci.cycle_id = $1 AND ci.issue_id = $2 AND ci.project_id = $3 AND ci.workspace_id = $4 AND ci.deleted_at IS NULL"
    ))
    .bind(cid).bind(iid).bind(pid).bind(p.workspace_id()), &st.db).await
}

async fn v1_delete_cycle_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, cid, iid)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let res = sqlx::query(
        "UPDATE cycle_issues SET deleted_at=now(), updated_at=now(), updated_by_id=$5
         WHERE issue_id=$1 AND cycle_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL",
    )
    .bind(iid)
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(user.id)
    .execute(&st.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn v1_transfer_cycle_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, cid)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let Some(new_cid) = uuid_of(&b, "new_cycle_id") else {
        return Err(ApiError::bad("New Cycle Id is required"));
    };

    let old_cycle: (Option<DateTime<Utc>>,) = sqlx::query_as(
        "SELECT end_date FROM cycles WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;

    if old_cycle.0.is_some_and(|e| e > Utc::now()) {
        return Err(ApiError::bad("The old cycle is not completed yet"));
    }

    let new_cycle: (Option<DateTime<Utc>>,) = sqlx::query_as(
        "SELECT end_date FROM cycles WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(new_cid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(|| ApiError::bad("The cycle where the issues are transferred is already completed"))?;

    if new_cycle.0.is_some_and(|e| e < Utc::now()) {
        return Err(ApiError::bad(
            "The cycle where the issues are transferred is already completed",
        ));
    }

    let mut tx = st.db.begin().await?;
    let c: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT count(*),
            count(*) FILTER (WHERE s.group = 'completed'),
            count(*) FILTER (WHERE s.group = 'cancelled'),
            count(*) FILTER (WHERE s.group = 'started'),
            count(*) FILTER (WHERE s.group = 'unstarted'),
            count(*) FILTER (WHERE s.group = 'backlog')
         FROM cycle_issues ci
         JOIN issues i ON i.id = ci.issue_id AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft
         LEFT JOIN states s ON s.id = i.state_id
         WHERE ci.cycle_id = $1 AND ci.project_id = $2 AND ci.workspace_id = $3 AND ci.deleted_at IS NULL",
    )
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE cycles SET progress_snapshot = jsonb_build_object(
            'total_issues', $2::bigint,
            'completed_issues', $3::bigint,
            'cancelled_issues', $4::bigint,
            'started_issues', $5::bigint,
            'unstarted_issues', $6::bigint,
            'backlog_issues', $7::bigint
        ) WHERE id = $1",
    )
    .bind(cid)
    .bind(c.0)
    .bind(c.1)
    .bind(c.2)
    .bind(c.3)
    .bind(c.4)
    .bind(c.5)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE cycle_issues ci SET cycle_id = $1, updated_by_id = $2, updated_at = now()
         WHERE ci.cycle_id = $3 AND ci.project_id = $4 AND ci.workspace_id = $5 AND ci.deleted_at IS NULL
           AND ci.issue_id IN (
               SELECT i.id FROM issues i
               JOIN states s ON s.id = i.state_id
               WHERE s.group = ANY($6) AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft
           )",
    )
    .bind(new_cid)
    .bind(user.id)
    .bind(cid)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(vec!["backlog", "unstarted", "started"])
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Json(json!({"message": "Success"})))
}

const MODULE_LITE_JSON: &str = "(to_jsonb(m)-'workspace_id'-'project_id'-'lead_id'-'created_by_id'-'updated_by_id'-'deleted_at') || jsonb_build_object('workspace',m.workspace_id,'project',m.project_id,'lead',m.lead_id,'created_by',m.created_by_id,'updated_by',m.updated_by_id,'members',COALESCE((SELECT jsonb_agg(mm.member_id) FROM module_members mm WHERE mm.module_id=m.id AND mm.deleted_at IS NULL),'[]'::jsonb))";

const MODULE_DETAIL_JSON: &str = "(to_jsonb(m)-'workspace_id'-'project_id'-'lead_id'-'created_by_id'-'updated_by_id'-'deleted_at') || jsonb_build_object('workspace',m.workspace_id,'project',m.project_id,'lead',m.lead_id,'created_by',m.created_by_id,'updated_by',m.updated_by_id,'members',COALESCE((SELECT jsonb_agg(mm.member_id) FROM module_members mm WHERE mm.module_id=m.id AND mm.deleted_at IS NULL),'[]'::jsonb),'total_issues',COALESCE(k.total,0),'completed_issues',COALESCE(k.completed,0),'cancelled_issues',COALESCE(k.cancelled,0),'started_issues',COALESCE(k.started,0),'unstarted_issues',COALESCE(k.unstarted,0),'backlog_issues',COALESCE(k.backlog,0))";

const MODULE_COUNTS_FROM: &str = "LEFT JOIN LATERAL (
    SELECT count(DISTINCT mi.issue_id) AS total,
        count(DISTINCT mi.issue_id) FILTER (WHERE s.group = 'backlog') AS backlog,
        count(DISTINCT mi.issue_id) FILTER (WHERE s.group = 'unstarted') AS unstarted,
        count(DISTINCT mi.issue_id) FILTER (WHERE s.group = 'started') AS started,
        count(DISTINCT mi.issue_id) FILTER (WHERE s.group = 'completed') AS completed,
        count(DISTINCT mi.issue_id) FILTER (WHERE s.group = 'cancelled') AS cancelled
    FROM module_issues mi
    JOIN issues i ON i.id = mi.issue_id AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft
    LEFT JOIN states s ON s.id = i.state_id
    WHERE mi.module_id = m.id AND mi.deleted_at IS NULL
) k ON true";

const MODULE_ISSUE_JSON: &str = "(to_jsonb(mi)-'workspace_id'-'project_id'-'module_id'-'issue_id'-'created_by_id'-'updated_by_id'-'deleted_at') || jsonb_build_object('workspace',mi.workspace_id,'project',mi.project_id,'module',mi.module_id,'issue',mi.issue_id,'created_by',mi.created_by_id,'updated_by',mi.updated_by_id,'sub_issues_count',(SELECT count(*) FROM issues sub WHERE sub.parent_id=mi.issue_id AND sub.deleted_at IS NULL AND sub.archived_at IS NULL AND NOT sub.is_draft))";

const MODULE_ISSUE_ITEM_JSON: &str = "(to_jsonb(i)-'workspace_id'-'project_id'-'created_by_id'-'updated_by_id'-'state_id'-'parent_id'-'estimate_point_id'-'deleted_at') || jsonb_build_object('workspace',i.workspace_id,'project',i.project_id,'created_by',i.created_by_id,'updated_by',i.updated_by_id,'state',i.state_id,'parent',i.parent_id,'estimate_point',i.estimate_point_id,'sub_issues_count',(SELECT count(*) FROM issues sub WHERE sub.parent_id=i.id AND sub.deleted_at IS NULL AND sub.archived_at IS NULL AND NOT sub.is_draft),'bridge_id',mi.id,'assignees',COALESCE((SELECT jsonb_agg(ia.assignee_id) FROM issue_assignees ia WHERE ia.issue_id=i.id AND ia.deleted_at IS NULL),'[]'::jsonb),'labels',COALESCE((SELECT jsonb_agg(il.label_id) FROM issue_labels il WHERE il.issue_id=i.id AND il.deleted_at IS NULL),'[]'::jsonb))";

async fn v1_modules(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {MODULE_DETAIL_JSON} FROM modules m {MODULE_COUNTS_FROM}
         JOIN projects p ON p.id = m.project_id AND p.workspace_id = m.workspace_id
         WHERE m.project_id = $1 AND m.workspace_id = $2 AND m.deleted_at IS NULL AND m.archived_at IS NULL
           AND p.deleted_at IS NULL AND p.archived_at IS NULL
         ORDER BY m.created_at DESC, m.id"
    ))
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    paginate(rows, &q)
}

async fn v1_modules_lite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {MODULE_LITE_JSON} FROM modules m
         JOIN projects p ON p.id = m.project_id AND p.workspace_id = m.workspace_id
         WHERE m.project_id = $1 AND m.workspace_id = $2 AND m.deleted_at IS NULL AND m.archived_at IS NULL
           AND p.deleted_at IS NULL AND p.archived_at IS NULL
         ORDER BY m.created_at DESC, m.id"
    ))
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    paginate(rows, &q)
}

async fn v1_module(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, mid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {MODULE_DETAIL_JSON} FROM modules m {MODULE_COUNTS_FROM}
             WHERE m.id = $1 AND m.project_id = $2 AND m.workspace_id = $3 AND m.deleted_at IS NULL AND m.archived_at IS NULL"
        ))
        .bind(mid)
        .bind(pid)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn v1_create_module(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let name = s(&b, "name")
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 255)
        .ok_or_else(|| ApiError::bad("Invalid name"))?;

    let dup: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM modules WHERE name = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
    )
    .bind(name)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    if let Some(dup_id) = dup {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            json!({
                "id": dup_id.to_string(),
                "code": "MODULE_NAME_ALREADY_EXISTS",
                "error": "Module with this name already exists",
                "message": "Module with this name already exists"
            }),
        ));
    }

    if let (Some(ext_id), Some(ext_src)) = (s(&b, "external_id"), s(&b, "external_source")) {
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM modules WHERE project_id=$1 AND workspace_id=$2 AND external_id=$3 AND external_source=$4 AND deleted_at IS NULL"
        )
        .bind(pid).bind(p.workspace_id()).bind(ext_id).bind(ext_src).fetch_optional(&st.db).await?;
        if let Some(existing_id) = exists {
            return Err(ApiError(
                StatusCode::CONFLICT,
                json!({
                    "error": "Module with the same external id and external source already exists",
                    "id": existing_id.to_string()
                }),
            ));
        }
    }

    let start = parse_day(&b, "start_date")?;
    let target = parse_day(&b, "target_date")?;
    if let (Some(s), Some(e)) = (&start, &target) {
        if s > e {
            return Err(ApiError::bad("Start date cannot exceed target date"));
        }
    }

    let lead_id = b
        .get("lead")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok());
    let status = s(&b, "status").unwrap_or("planned");

    let mut tx = st.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO modules (id, name, description, start_date, target_date, status, lead_id, project_id, workspace_id,
            created_by_id, updated_by_id, view_props, logo_props, external_source, external_id, sort_order)
         VALUES (gen_random_uuid(), $1, COALESCE($2, ''), $3::date, $4::date, $5, $6, $7, $8, $9, $9,
            COALESCE($10::jsonb, '{}'::jsonb), COALESCE($11::jsonb, '{}'::jsonb), $12, $13,
            COALESCE($14::float8, (SELECT COALESCE(max(sort_order), 0) + 10000 FROM modules WHERE project_id = $7), 65535))
         RETURNING id",
    )
    .bind(name)
    .bind(s(&b, "description"))
    .bind(start)
    .bind(target)
    .bind(status)
    .bind(lead_id)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(user.id)
    .bind(b.get("view_props").filter(|v| !v.is_null()))
    .bind(b.get("logo_props").filter(|v| !v.is_null()))
    .bind(s(&b, "external_source"))
    .bind(s(&b, "external_id"))
    .bind(b.get("sort_order").and_then(Value::as_f64))
    .fetch_one(&mut *tx)
    .await?;

    if let Some(members) = b.get("members").and_then(Value::as_array) {
        let member_ids: Vec<Uuid> = members
            .iter()
            .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
            .collect();
        for m in member_ids {
            sqlx::query(
                "INSERT INTO module_members (id, module_id, member_id, project_id, workspace_id, created_by_id, updated_by_id)
                 VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $5) ON CONFLICT DO NOTHING"
            )
            .bind(id).bind(m).bind(pid).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;

    let Json(val) = v1_module(State(st), user, Path((slug, pid, id))).await?;
    created(val)
}

async fn v1_update_module(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, mid)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let cur: (Option<DateTime<Utc>>, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT m.archived_at, m.name, m.external_id, m.external_source FROM modules m
         WHERE m.id = $1 AND m.project_id = $2 AND m.workspace_id = $3 AND m.deleted_at IS NULL",
    )
    .bind(mid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;

    if cur.0.is_some() {
        return Err(ApiError::bad("Archived module cannot be edited"));
    }

    if let Some(n) = s(&b, "name").map(str::trim) {
        if n.is_empty() || n.len() > 255 {
            return Err(ApiError::bad("Invalid name"));
        }
        if Some(n) != cur.1.as_deref() {
            let dup: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM modules WHERE name = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL AND id <> $4"
            )
            .bind(n).bind(pid).bind(p.workspace_id()).bind(mid).fetch_optional(&st.db).await?;
            if dup.is_some() {
                return Err(ApiError::bad("Module with this name already exists"));
            }
        }
    }

    if let Some(ext_id) = s(&b, "external_id") {
        let ext_src = s(&b, "external_source").or(cur.3.as_deref());
        if cur.2.as_deref() != Some(ext_id) {
            let conflict: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM modules WHERE project_id=$1 AND workspace_id=$2 AND external_id=$3 AND external_source=$4 AND deleted_at IS NULL AND id <> $5"
            )
            .bind(pid).bind(p.workspace_id()).bind(ext_id).bind(ext_src).bind(mid).fetch_optional(&st.db).await?;
            if let Some(conflict_id) = conflict {
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    json!({
                        "error": "Module with the same external id and external source already exists",
                        "id": conflict_id.to_string()
                    }),
                ));
            }
        }
    }

    let has_start = b.get("start_date").is_some();
    let has_target = b.get("target_date").is_some();
    let start = parse_day(&b, "start_date")?;
    let target = parse_day(&b, "target_date")?;
    if has_start && has_target {
        if let (Some(s), Some(e)) = (&start, &target) {
            if s > e {
                return Err(ApiError::bad("Start date cannot exceed target date"));
            }
        }
    }

    let has_lead = b.get("lead").is_some();
    let lead_id = b
        .get("lead")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok());

    let mut tx = st.db.begin().await?;
    let res = sqlx::query(
        "UPDATE modules SET
            name = COALESCE($4, name),
            description = COALESCE($5, description),
            start_date = CASE WHEN $6 THEN $7::date ELSE start_date END,
            target_date = CASE WHEN $8 THEN $9::date ELSE target_date END,
            status = COALESCE($10, status),
            lead_id = CASE WHEN $11 THEN $12 ELSE lead_id END,
            view_props = COALESCE($13::jsonb, view_props),
            logo_props = COALESCE($14::jsonb, logo_props),
            external_source = COALESCE($15, external_source),
            external_id = COALESCE($16, external_id),
            sort_order = COALESCE($17::float8, sort_order),
            updated_by_id = $3, updated_at = now()
         WHERE id = $1 AND project_id = $2 AND workspace_id = $18
           AND deleted_at IS NULL AND archived_at IS NULL",
    )
    .bind(mid)
    .bind(pid)
    .bind(user.id)
    .bind(s(&b, "name"))
    .bind(s(&b, "description"))
    .bind(has_start)
    .bind(start)
    .bind(has_target)
    .bind(target)
    .bind(s(&b, "status"))
    .bind(has_lead)
    .bind(lead_id)
    .bind(b.get("view_props").filter(|v| !v.is_null()))
    .bind(b.get("logo_props").filter(|v| !v.is_null()))
    .bind(s(&b, "external_source"))
    .bind(s(&b, "external_id"))
    .bind(b.get("sort_order").and_then(Value::as_f64))
    .bind(p.workspace_id())
    .execute(&mut *tx)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }

    if let Some(members) = b.get("members").and_then(Value::as_array) {
        sqlx::query("DELETE FROM module_members WHERE module_id = $1")
            .bind(mid)
            .execute(&mut *tx)
            .await?;
        let member_ids: Vec<Uuid> = members
            .iter()
            .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
            .collect();
        for m in member_ids {
            sqlx::query(
                "INSERT INTO module_members (id, module_id, member_id, project_id, workspace_id, created_by_id, updated_by_id)
                 VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $5) ON CONFLICT DO NOTHING"
            )
            .bind(mid).bind(m).bind(pid).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;

    v1_module(State(st), user, Path((slug, pid, mid))).await
}

async fn v1_delete_module(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, mid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    let (created_by_id,): (Option<Uuid>,) = sqlx::query_as(
        "SELECT created_by_id FROM modules WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(mid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;

    if !p.is_admin() && created_by_id != Some(user.id) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            json!({"error": "Only admin or creator can delete the module"}),
        ));
    }

    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE modules SET deleted_at=now(), updated_at=now(), updated_by_id=$2 WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(mid)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE module_issues SET deleted_at=now(), updated_at=now(), updated_by_id=$2 WHERE module_id=$1 AND deleted_at IS NULL")
        .bind(mid).bind(user.id).execute(&mut *tx).await?;
    sqlx::query("UPDATE user_favorites SET deleted_at=now() WHERE entity_type='module' AND entity_identifier=$1 AND project_id=$2 AND deleted_at IS NULL")
        .bind(mid).bind(pid).execute(&mut *tx).await?;
    tx.commit().await?;
    no_content()
}

async fn v1_archive_module(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, mid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let (status,): (String,) = sqlx::query_as(
        "SELECT status FROM modules WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(mid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;

    if !["completed", "cancelled"].contains(&status.as_str()) {
        return Err(ApiError::bad("Only completed or cancelled modules can be archived"));
    }

    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE modules SET archived_at=now(), updated_at=now(), updated_by_id=$2 WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(mid)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE user_favorites SET deleted_at=now() WHERE entity_type='module' AND entity_identifier=$1 AND project_id=$2 AND deleted_at IS NULL")
        .bind(mid).bind(pid).execute(&mut *tx).await?;
    tx.commit().await?;
    no_content()
}

async fn v1_archived_modules(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {MODULE_DETAIL_JSON} FROM modules m {MODULE_COUNTS_FROM}
         WHERE m.project_id=$1 AND m.workspace_id=$2 AND m.deleted_at IS NULL AND m.archived_at IS NOT NULL
         ORDER BY m.created_at DESC, m.id"
    ))
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    paginate(rows, &q)
}

async fn v1_unarchive_module(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, mid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let res = sqlx::query(
        "UPDATE modules SET archived_at=NULL, updated_at=now(), updated_by_id=$4
         WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND archived_at IS NOT NULL",
    )
    .bind(mid)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(user.id)
    .execute(&st.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn v1_module_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, mid)): Path<(String, Uuid, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM modules WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
    )
    .bind(mid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;
    if !exists {
        return Err(ApiError::not_found());
    }

    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {MODULE_ISSUE_ITEM_JSON} FROM issues i
         JOIN module_issues mi ON mi.issue_id = i.id AND mi.module_id = $1 AND mi.deleted_at IS NULL
         WHERE i.project_id = $2 AND i.workspace_id = $3 AND i.deleted_at IS NULL
           AND (NOT $4 OR i.created_by_id = $5)
         ORDER BY i.created_at DESC, i.id"
    ))
    .bind(mid)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(p.restricted_guest())
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;
    paginate(rows, &q)
}

async fn v1_add_module_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, mid)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let issues: Vec<Uuid> = b
        .get("issues")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default();
    if issues.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            json!({"error": "Issues are required"}),
        ));
    }

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM modules WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
    )
    .bind(mid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;
    if !exists {
        return Err(ApiError::not_found());
    }

    let mut tx = st.db.begin().await?;
    sqlx::query(
        "INSERT INTO module_issues (id, module_id, issue_id, project_id, workspace_id, created_by_id, updated_by_id)
         SELECT gen_random_uuid(), $1, i.id, i.project_id, i.workspace_id, $2, $2
         FROM issues i
         WHERE i.id = ANY($3) AND i.project_id = $4 AND i.workspace_id = $5 AND i.deleted_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM module_issues mi WHERE mi.issue_id = i.id AND mi.module_id = $1 AND mi.deleted_at IS NULL)
         ON CONFLICT DO NOTHING"
    )
    .bind(mid).bind(user.id).bind(&issues).bind(pid).bind(p.workspace_id()).execute(&mut *tx).await?;
    tx.commit().await?;

    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {MODULE_ISSUE_JSON} FROM module_issues mi
         WHERE mi.module_id = $1 AND mi.project_id = $2 AND mi.workspace_id = $3 AND mi.deleted_at IS NULL
         ORDER BY mi.created_at DESC, mi.id"
    ))
    .bind(mid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    Ok(Json(Value::Array(rows)))
}

async fn v1_delete_module_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, mid, iid)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let res = sqlx::query(
        "UPDATE module_issues SET deleted_at=now(), updated_at=now(), updated_by_id=$5
         WHERE module_id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL",
    )
    .bind(mid)
    .bind(iid)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(user.id)
    .execute(&st.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_uses_plane_cursors() {
        let rows = vec![json!(1), json!(2), json!(3)];
        let query = HashMap::from([("per_page".to_string(), "2".to_string())]);
        let axum::Json(first) = paginate(rows.clone(), &query).unwrap();
        assert_eq!(first["next_cursor"], "2:1:0");
        assert_eq!(first["results"], json!([1, 2]));
        let query = HashMap::from([
            ("per_page".to_string(), "2".to_string()),
            ("cursor".to_string(), "2:1:0".to_string()),
        ]);
        let axum::Json(second) = paginate(rows, &query).unwrap();
        assert_eq!(second["results"], json!([3]));
        assert_eq!(second["prev_cursor"], "2:0:1");
    }
}
