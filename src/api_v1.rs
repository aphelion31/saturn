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
        .route("/api/v1/workspaces/{slug}/", get(v1_workspace))
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/members/",
            get(v1_project_members).post(v1_create_project_member),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/members/{pk}/",
            get(v1_project_member)
                .patch(v1_update_project_member)
                .delete(v1_delete_project_member),
        )
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
        .route(
            "/api/v1/workspaces/{slug}/members-lite/",
            get(v1_workspace_members_lite),
        )
        .route("/api/v1/workspaces/{slug}/projects-lite/", get(v1_projects_lite))
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/project-members/",
            get(v1_project_members).post(v1_create_project_member),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/project-members-lite/",
            get(v1_project_members_lite),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/project-members/{pk}/",
            get(v1_project_member)
                .patch(v1_update_project_member)
                .delete(v1_delete_project_member),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/summary/",
            get(v1_project_summary),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/intake-issues/",
            get(v1_intake_issues).post(v1_create_intake_issue),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/intake-issues/{issue_id}/",
            get(v1_intake_issue).patch(v1_update_intake_issue).delete(v1_delete_intake_issue),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/estimates/",
            get(v1_estimate).post(v1_create_estimate).patch(v1_update_estimate).delete(v1_delete_estimate),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/estimates/{estimate_id}/estimate-points/",
            get(v1_estimate_points).post(v1_create_estimate_points),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/estimates/{estimate_id}/estimate-points/{estimate_point_id}/",
            axum::routing::patch(v1_update_estimate_point).delete(v1_delete_estimate_point),
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

const MEMBER_LITE_JSON: &str = "jsonb_build_object('id',u.id,'first_name',u.first_name,'last_name',u.last_name,'email',u.email,'avatar',u.avatar,'avatar_url',CASE WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id || '/' ELSE NULLIF(u.avatar, '') END,'display_name',u.display_name,'role',m.role,'is_active',m.is_active,'is_bot',u.is_bot)";

const PROJECT_LITE_JSON: &str = "jsonb_build_object('id',p.id,'identifier',p.identifier,'name',p.name,'cover_image',p.cover_image,'icon_prop',p.icon_prop,'emoji',p.emoji,'description',p.description,'cover_image_url',p.cover_image,'archived_at',p.archived_at)";

async fn v1_workspace_members_lite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {MEMBER_LITE_JSON} FROM workspace_members m
         JOIN users u ON u.id = m.member_id
         WHERE m.workspace_id = $1 AND m.deleted_at IS NULL
         ORDER BY m.created_at, m.id"
    ))
    .bind(ws.workspace_id)
    .fetch_all(&st.db)
    .await?;
    paginate(rows, &q)
}

async fn v1_projects_lite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let include_archived = q
        .get("include_archived")
        .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1");
    let cond = if include_archived {
        ""
    } else {
        "AND p.archived_at IS NULL"
    };
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {PROJECT_LITE_JSON} FROM projects p
         LEFT JOIN project_members pm ON pm.project_id = p.id AND pm.workspace_id = p.workspace_id AND pm.member_id = $2 AND pm.is_active AND pm.deleted_at IS NULL
         WHERE p.workspace_id = $1 AND p.deleted_at IS NULL AND (pm.id IS NOT NULL OR p.network = 2) {cond}
         ORDER BY p.created_at DESC, p.id"
    ))
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;
    paginate(rows, &q)
}

// Same body as workspaces.rs WS_JSON (private there). Keep in sync.
const WS_V1_JSON: &str = "to_jsonb(w) - 'owner_id' - 'created_by_id' - 'updated_by_id' - 'logo_asset_id'
    || jsonb_build_object('owner', w.owner_id, 'created_by', w.created_by_id, 'updated_by', w.updated_by_id,
       'logo_asset', w.logo_asset_id, 'logo_url', NULLIF(w.logo, ''),
       'total_members', (SELECT count(*) FROM workspace_members m JOIN users u ON u.id = m.member_id
                         WHERE m.workspace_id = w.id AND m.is_active AND m.deleted_at IS NULL AND NOT u.is_bot))";

/// Plane WorkSpaceViewSet.retrieve: workspace detail for any member (role included).
async fn v1_workspace(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {WS_V1_JSON} || jsonb_build_object('role', $2::smallint) FROM workspaces w
             WHERE w.id = $1 AND w.deleted_at IS NULL"
        ))
        .bind(ws.workspace_id)
        .bind(ws.role),
        &st.db,
    )
    .await
}

async fn v1_project_members(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {USER_JSON} FROM project_members pm
         JOIN users u ON u.id = pm.member_id
         WHERE pm.project_id = $1 AND pm.workspace_id = $2 AND pm.is_active AND pm.deleted_at IS NULL
         ORDER BY pm.created_at, pm.id"
    ))
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    Ok(Json(Value::Array(rows)))
}

async fn v1_create_project_member(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN])?;

    let member_id = b
        .get("member")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| ApiError::bad("Member is required"))?;
    let role = b.get("role").and_then(Value::as_i64).unwrap_or(15) as i16;
    if ![20, 15, 5].contains(&role) {
        return Err(ApiError::bad("Invalid role"));
    }

    let ws_role: Option<i16> = sqlx::query_scalar(
        "SELECT role FROM workspace_members WHERE workspace_id = $1 AND member_id = $2 AND is_active AND deleted_at IS NULL"
    )
    .bind(p.workspace_id())
    .bind(member_id)
    .fetch_optional(&st.db)
    .await?;

    let ws_role = ws_role.ok_or_else(|| ApiError::bad("Member not found in workspace"))?;
    if ws_role == 5 && role > 5 {
        return Err(ApiError::bad(
            "You cannot assign a role higher than the member's workspace role",
        ));
    }

    let row: (Uuid, Uuid, i16) = sqlx::query_as(
        "INSERT INTO project_members (id, project_id, workspace_id, member_id, role, created_by_id, updated_by_id)
         VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $5)
         ON CONFLICT (project_id, member_id) WHERE deleted_at IS NULL
         DO UPDATE SET role = EXCLUDED.role, updated_at = now(), updated_by_id = EXCLUDED.updated_by_id
         RETURNING id, member_id, role",
    )
    .bind(pid)
    .bind(p.workspace_id())
    .bind(member_id)
    .bind(role)
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;

    created(json!({
        "id": row.0,
        "member": row.1,
        "role": row.2
    }))
}

async fn v1_project_members_lite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {MEMBER_LITE_JSON} FROM project_members m
         JOIN users u ON u.id = m.member_id
         WHERE m.project_id = $1 AND m.workspace_id = $2 AND m.deleted_at IS NULL
         ORDER BY m.created_at, m.id"
    ))
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    paginate(rows, &q)
}

async fn v1_project_member(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {USER_JSON} FROM project_members pm
             JOIN users u ON u.id = pm.member_id
             WHERE (pm.id = $1 OR pm.member_id = $1) AND pm.project_id = $2 AND pm.workspace_id = $3 AND pm.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(pid)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn v1_update_project_member(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, pk)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN])?;

    let role = b
        .get("role")
        .and_then(Value::as_i64)
        .ok_or_else(|| ApiError::bad("Role is required"))? as i16;
    if ![20, 15, 5].contains(&role) {
        return Err(ApiError::bad("Invalid role"));
    }

    let ws_role: Option<i16> = sqlx::query_scalar(
        "SELECT wm.role FROM project_members pm
         JOIN workspace_members wm ON wm.workspace_id = pm.workspace_id AND wm.member_id = pm.member_id AND wm.is_active AND wm.deleted_at IS NULL
         WHERE (pm.id = $1 OR pm.member_id = $1) AND pm.project_id = $2 AND pm.workspace_id = $3 AND pm.deleted_at IS NULL"
    )
    .bind(pk)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;

    let ws_role = ws_role.ok_or_else(ApiError::not_found)?;
    if ws_role == 5 && role > 5 {
        return Err(ApiError::bad(
            "You cannot assign a role higher than the member's workspace role",
        ));
    }

    let row: (Uuid, Uuid, i16) = sqlx::query_as(
        "UPDATE project_members SET role = $1, updated_at = now(), updated_by_id = $2
         WHERE (id = $3 OR member_id = $3) AND project_id = $4 AND workspace_id = $5 AND deleted_at IS NULL
         RETURNING id, member_id, role",
    )
    .bind(role)
    .bind(user.id)
    .bind(pk)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;

    Ok(Json(json!({
        "id": row.0,
        "member": row.1,
        "role": row.2
    })))
}

async fn v1_delete_project_member(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN])?;

    let res = sqlx::query(
        "UPDATE project_members SET deleted_at = now(), updated_at = now(), updated_by_id = $2
         WHERE (id = $1 OR member_id = $1) AND project_id = $3 AND workspace_id = $4 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(user.id)
    .bind(pid)
    .bind(p.workspace_id())
    .execute(&st.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn v1_project_summary(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;

    let proj: (String, String) = sqlx::query_as(
        "SELECT name, identifier FROM projects WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
    )
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;

    let allowed = [
        "members", "states", "labels", "cycles", "modules", "issues", "intakes", "pages",
    ];
    let filter_fields: Option<Vec<String>> = q.get("fields").and_then(|f| {
        let v: Vec<String> = f
            .split(',')
            .map(str::trim)
            .filter(|s| allowed.contains(s))
            .map(str::to_string)
            .collect();
        if v.is_empty() { None } else { Some(v) }
    });

    let counts: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'members', (SELECT count(*) FROM project_members WHERE project_id = $1 AND workspace_id = $2 AND is_active AND deleted_at IS NULL),
            'states', (SELECT count(*) FROM states WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL),
            'labels', (SELECT count(*) FROM labels WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL),
            'cycles', (SELECT count(*) FROM cycles WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL),
            'modules', (SELECT count(*) FROM modules WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL),
            'issues', (SELECT count(*) FROM issues i LEFT JOIN states s ON s.id = i.state_id WHERE i.project_id = $1 AND i.workspace_id = $2 AND i.deleted_at IS NULL AND (s.group IS NULL OR s.group <> 'triage')),
            'intakes', (SELECT count(*) FROM intake_issues WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL),
            'pages', (SELECT count(*) FROM project_pages pp JOIN pages pg ON pg.id = pp.page_id WHERE pp.project_id = $1 AND pp.workspace_id = $2 AND pp.deleted_at IS NULL AND pg.deleted_at IS NULL)
        )"
    )
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;

    let counts_obj = if let Some(fields) = filter_fields {
        let mut map = serde_json::Map::new();
        if let Some(obj) = counts.as_object() {
            for f in fields {
                if let Some(v) = obj.get(&f) {
                    map.insert(f, v.clone());
                }
            }
        }
        Value::Object(map)
    } else {
        counts
    };

    Ok(Json(json!({
        "id": pid,
        "name": proj.0,
        "identifier": proj.1,
        "counts": counts_obj
    })))
}

const INTAKE_ISSUE_V1_JSON: &str = "(to_jsonb(ii)-'workspace_id'-'project_id'-'intake_id'-'issue_id'-'duplicate_to_id'-'created_by_id'-'updated_by_id'-'deleted_at') || jsonb_build_object('workspace',ii.workspace_id,'project',ii.project_id,'issue',ii.issue_id,'inbox',ii.intake_id,'duplicate_to',ii.duplicate_to_id,'created_by',ii.created_by_id,'updated_by',ii.updated_by_id,'issue_detail',jsonb_build_object('id',i.id,'name',i.name,'description_html',i.description_html,'description_stripped',i.description_stripped,'priority',i.priority,'state_id',i.state_id,'sequence_id',i.sequence_id,'project_id',i.project_id,'parent_id',i.parent_id,'start_date',i.start_date,'target_date',i.target_date,'completed_at',i.completed_at,'sort_order',i.sort_order,'is_draft',i.is_draft,'archived_at',i.archived_at,'created_at',i.created_at,'updated_at',i.updated_at,'created_by',i.created_by_id,'updated_by',i.updated_by_id,'estimate_point',i.estimate_point_id,'type_id',i.type_id,'label_ids',COALESCE((SELECT jsonb_agg(il.label_id) FROM issue_labels il WHERE il.issue_id=i.id AND il.deleted_at IS NULL),'[]'::jsonb),'assignee_ids',COALESCE((SELECT jsonb_agg(a.assignee_id) FROM issue_assignees a WHERE a.issue_id=i.id AND a.deleted_at IS NULL),'[]'::jsonb),'module_ids',COALESCE((SELECT jsonb_agg(m.module_id) FROM module_issues m WHERE m.issue_id=i.id AND m.deleted_at IS NULL),'[]'::jsonb)))";

async fn ensure_default_intake_v1(
    db: &sqlx::PgPool,
    workspace_id: Uuid,
    project_id: Uuid,
    user_id: Uuid,
) -> ApiResult<Uuid> {
    let id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO intakes (id, name, description, is_default, view_props, logo_props, project_id, workspace_id,
             created_by_id, updated_by_id, created_at, updated_at)
         SELECT gen_random_uuid(), 'Intake', '', true, '{}'::jsonb, '{}'::jsonb, $1, $2, $3, $3, now(), now()
         WHERE NOT EXISTS (SELECT 1 FROM intakes WHERE project_id = $1 AND deleted_at IS NULL)
         ON CONFLICT DO NOTHING
         RETURNING id",
    )
    .bind(project_id)
    .bind(workspace_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    match id {
        Some(id) => Ok(id),
        None => sqlx::query_scalar(
            "SELECT id FROM intakes WHERE project_id = $1 AND deleted_at IS NULL ORDER BY created_at LIMIT 1",
        )
        .bind(project_id)
        .fetch_one(db)
        .await
        .map_err(Into::into),
    }
}

async fn v1_intake_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;

    let intake_id = ensure_default_intake_v1(&st.db, p.workspace_id(), pid, user.id).await?;
    let statuses: Vec<i32> = q
        .get("status")
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_default();

    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {INTAKE_ISSUE_V1_JSON} FROM intake_issues ii
         JOIN issues i ON i.id = ii.issue_id AND i.deleted_at IS NULL
         WHERE ii.project_id = $1 AND ii.workspace_id = $2 AND ii.intake_id = $3
           AND ii.deleted_at IS NULL
           AND (ii.snoozed_till >= now() OR ii.snoozed_till IS NULL)
           AND ($4::int[] = '{{}}' OR ii.status = ANY($4))
           AND (NOT $5 OR ii.created_by_id = $6)
         ORDER BY ii.created_at DESC, ii.id"
    ))
    .bind(pid)
    .bind(p.workspace_id())
    .bind(intake_id)
    .bind(&statuses)
    .bind(p.restricted_guest())
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;

    paginate(rows, &q)
}

async fn v1_create_intake_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;

    let issue = b.get("issue").cloned().unwrap_or_else(|| json!({}));
    let name = match issue.get("name") {
        Some(Value::String(n)) if !n.trim().is_empty() => {
            let trimmed = n.trim();
            if trimmed.chars().count() > 255 {
                return Err(ApiError::bad("Name must be at most 255 characters"));
            }
            trimmed.to_string()
        }
        _ => return Err(ApiError::bad("Name is required")),
    };
    let priority = s(&issue, "priority").unwrap_or("none");
    if !["low", "medium", "high", "urgent", "none"].contains(&priority) {
        return Err(ApiError::bad("Invalid priority"));
    }
    let description = match s(&issue, "description_html") {
        Some(h) => sanitize_html(h)?,
        None => "<p></p>".to_string(),
    };
    let stripped = strip_tags(&description);
    let intake_id = ensure_default_intake_v1(&st.db, p.workspace_id(), pid, user.id).await?;

    let mut tx = st.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1::text))")
        .bind(pid)
        .execute(&mut *tx)
        .await?;

    let triage: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM states WHERE project_id = $1 AND is_triage AND deleted_at IS NULL LIMIT 1")
            .bind(pid)
            .fetch_optional(&mut *tx)
            .await?;
    let triage = match triage {
        Some(t) => t,
        None => {
            sqlx::query_scalar(
                "INSERT INTO states (name, color, \"group\", \"default\", is_triage, sequence, slug, project_id, workspace_id,
                     created_by_id, updated_by_id)
                 VALUES ('Triage', '#4E5355', 'triage', false, true, 65000, 'triage', $1, $2, $3, $3) RETURNING id",
            )
            .bind(pid)
            .bind(p.workspace_id())
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?
        }
    };

    let seq: i64 =
        sqlx::query_scalar("SELECT COALESCE(max(sequence), 0) + 1 FROM issue_sequences WHERE project_id = $1")
            .bind(pid)
            .fetch_one(&mut *tx)
            .await?;

    let issue_id: Uuid = sqlx::query_scalar(
        "INSERT INTO issues (id, name, description_html, description_stripped, description_json, priority, sequence_id,
             project_id, workspace_id, state_id, sort_order, is_draft, created_by_id, updated_by_id, created_at, updated_at)
         VALUES (gen_random_uuid(), $1, $2, $3, '{}'::jsonb, $4, $5, $6, $7, $8, 65535, false, $9, $9, now(), now())
         RETURNING id",
    )
    .bind(&name)
    .bind(&description)
    .bind(&stripped)
    .bind(priority)
    .bind(seq as i32)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(triage)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO issue_sequences (id, sequence, deleted, issue_id, project_id, workspace_id, created_by_id, updated_by_id, created_at, updated_at)
         VALUES (gen_random_uuid(), $1, false, $2, $3, $4, $5, $5, now(), now())",
    )
    .bind(seq)
    .bind(issue_id)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(user.id)
    .execute(&mut *tx)
    .await?;

    let ii_id: Uuid = sqlx::query_scalar(
        "INSERT INTO intake_issues (id, status, source, intake_id, issue_id, project_id, workspace_id, created_by_id,
             updated_by_id, created_at, updated_at)
         VALUES (gen_random_uuid(), -2, 'IN_APP', $1, $2, $3, $4, $5, $5, now(), now())
         RETURNING id",
    )
    .bind(intake_id)
    .bind(issue_id)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let val: Value = sqlx::query_scalar(&format!(
        "SELECT {INTAKE_ISSUE_V1_JSON} FROM intake_issues ii
         JOIN issues i ON i.id = ii.issue_id AND i.deleted_at IS NULL
         WHERE ii.id = $1 AND ii.project_id = $2 AND ii.workspace_id = $3 AND ii.deleted_at IS NULL"
    ))
    .bind(ii_id)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;

    created(val)
}

async fn v1_intake_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {INTAKE_ISSUE_V1_JSON} FROM intake_issues ii
             JOIN issues i ON i.id = ii.issue_id AND i.deleted_at IS NULL
             WHERE (ii.issue_id = $1 OR ii.id = $1) AND ii.project_id = $2 AND ii.workspace_id = $3 AND ii.deleted_at IS NULL"
        ))
        .bind(iid)
        .bind(pid)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn v1_update_intake_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let cur: Option<(Uuid, Uuid, i32)> = sqlx::query_as(
        "SELECT ii.id, ii.issue_id, ii.status FROM intake_issues ii
         WHERE (ii.issue_id = $1 OR ii.id = $1) AND ii.project_id = $2 AND ii.workspace_id = $3 AND ii.deleted_at IS NULL"
    )
    .bind(iid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;

    let (ii_id, issue_id, old_status) = cur.ok_or_else(ApiError::not_found)?;
    let mut tx = st.db.begin().await?;

    if let Some(status) = b.get("status").and_then(Value::as_i64).map(|v| v as i32) {
        if status == 1 && old_status != 1 {
            let def_state: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM states WHERE project_id = $1 AND workspace_id = $2 AND \"default\" AND deleted_at IS NULL LIMIT 1"
            )
            .bind(pid)
            .bind(p.workspace_id())
            .fetch_optional(&mut *tx)
            .await?;

            let def_state = def_state
                .ok_or_else(|| ApiError::bad("Cannot accept intake issue: No default state found for the project"))?;
            sqlx::query("UPDATE issues SET state_id = $1, updated_at = now(), updated_by_id = $2 WHERE id = $3 AND project_id = $4 AND workspace_id = $5")
                .bind(def_state)
                .bind(user.id)
                .bind(issue_id)
                .bind(pid)
                .bind(p.workspace_id())
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE intake_issues SET status = $1, updated_at = now(), updated_by_id = $2 WHERE id = $3")
            .bind(status)
            .bind(user.id)
            .bind(ii_id)
            .execute(&mut *tx)
            .await?;
    }

    if let Some(issue_b) = b.get("issue").and_then(Value::as_object) {
        if let Some(n) = issue_b.get("name").and_then(Value::as_str) {
            sqlx::query("UPDATE issues SET name = $1, updated_at = now(), updated_by_id = $2 WHERE id = $3 AND project_id = $4 AND workspace_id = $5")
                .bind(n)
                .bind(user.id)
                .bind(issue_id)
                .bind(pid)
                .bind(p.workspace_id())
                .execute(&mut *tx)
                .await?;
        }
        if let Some(pr) = issue_b.get("priority").and_then(Value::as_str) {
            if ["low", "medium", "high", "urgent", "none"].contains(&pr) {
                sqlx::query("UPDATE issues SET priority = $1, updated_at = now(), updated_by_id = $2 WHERE id = $3 AND project_id = $4 AND workspace_id = $5")
                    .bind(pr)
                    .bind(user.id)
                    .bind(issue_id)
                    .bind(pid)
                    .bind(p.workspace_id())
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    tx.commit().await?;

    v1_intake_issue(State(st), user, Path((slug, pid, iid))).await
}

async fn v1_delete_intake_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let row: Option<(Uuid, Uuid, i32, Option<Uuid>)> = sqlx::query_as(
        "SELECT ii.id, ii.issue_id, ii.status, ii.created_by_id FROM intake_issues ii
         WHERE (ii.issue_id = $1 OR ii.id = $1) AND ii.project_id = $2 AND ii.workspace_id = $3 AND ii.deleted_at IS NULL"
    )
    .bind(iid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;

    let (ii_id, issue_id, status, creator) = row.ok_or_else(ApiError::not_found)?;
    if !(p.is_admin() || creator == Some(user.id)) {
        return Err(ApiError::forbidden());
    }

    let mut tx = st.db.begin().await?;
    sqlx::query("UPDATE intake_issues SET deleted_at = now(), updated_at = now(), updated_by_id = $2 WHERE id = $1")
        .bind(ii_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;

    if [-2, -1, 0, 2].contains(&status) {
        sqlx::query("UPDATE issues SET deleted_at = now(), updated_at = now(), updated_by_id = $4 WHERE id = $1 AND project_id = $2 AND workspace_id = $3")
            .bind(issue_id)
            .bind(pid)
            .bind(p.workspace_id())
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    no_content()
}

const ESTIMATE_JSON: &str = "(to_jsonb(e)-'workspace_id'-'project_id'-'created_by_id'-'updated_by_id'-'deleted_at') || jsonb_build_object('workspace',e.workspace_id,'project',e.project_id,'created_by',e.created_by_id,'updated_by',e.updated_by_id)";

const ESTIMATE_POINT_JSON: &str = "(to_jsonb(ep)-'workspace_id'-'project_id'-'estimate_id'-'created_by_id'-'updated_by_id'-'deleted_at') || jsonb_build_object('workspace',ep.workspace_id,'project',ep.project_id,'estimate',ep.estimate_id,'created_by',ep.created_by_id,'updated_by',ep.updated_by_id)";

async fn v1_estimate(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let row: Option<Value> = sqlx::query_scalar(&format!(
        "SELECT {ESTIMATE_JSON} FROM estimates e
         WHERE e.project_id = $1 AND e.workspace_id = $2 AND e.deleted_at IS NULL
         ORDER BY e.created_at DESC LIMIT 1"
    ))
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;

    match row {
        Some(v) => Ok(Json(v)),
        None => Err(ApiError(StatusCode::NOT_FOUND, json!({"error": "Estimate not found"}))),
    }
}

async fn v1_create_estimate(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM estimates WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;

    if let Some(eid) = existing {
        return Err(ApiError(
            StatusCode::CONFLICT,
            json!({
                "error": "An estimate already exists for this project",
                "id": eid.to_string()
            }),
        ));
    }

    let name = s(&b, "name")
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 255)
        .ok_or_else(|| ApiError::bad("Name is required"))?;
    let est_type = s(&b, "type").unwrap_or("categories");

    let mut tx = st.db.begin().await?;
    let eid: Uuid = sqlx::query_scalar(
        "INSERT INTO estimates (id, name, description, type, project_id, workspace_id, created_by_id, updated_by_id, created_at, updated_at)
         VALUES (gen_random_uuid(), $1, COALESCE($2, ''), $3, $4, $5, $6, $6, now(), now())
         RETURNING id",
    )
    .bind(name)
    .bind(s(&b, "description"))
    .bind(est_type)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query("UPDATE projects SET estimate_id = $1 WHERE id = $2 AND workspace_id = $3")
        .bind(eid)
        .bind(pid)
        .bind(p.workspace_id())
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    let val: Value = sqlx::query_scalar(&format!("SELECT {ESTIMATE_JSON} FROM estimates e WHERE e.id = $1"))
        .bind(eid)
        .fetch_one(&st.db)
        .await?;

    created(val)
}

async fn v1_update_estimate(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let row = sqlx::query(
        "UPDATE estimates SET
            name = COALESCE($3, name),
            description = COALESCE($4, description),
            updated_at = now(),
            updated_by_id = $5
         WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
    )
    .bind(pid)
    .bind(p.workspace_id())
    .bind(s(&b, "name"))
    .bind(s(&b, "description"))
    .bind(user.id)
    .execute(&st.db)
    .await?;

    if row.rows_affected() == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, json!({"error": "Estimate not found"})));
    }

    v1_estimate(State(st), user, Path((slug, pid))).await
}

async fn v1_delete_estimate(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let mut tx = st.db.begin().await?;
    let res = sqlx::query(
        "UPDATE estimates SET deleted_at = now(), updated_at = now(), updated_by_id = $3
         WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
    )
    .bind(pid)
    .bind(p.workspace_id())
    .bind(user.id)
    .execute(&mut *tx)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, json!({"error": "Estimate not found"})));
    }

    sqlx::query("UPDATE estimate_points SET deleted_at = now(), updated_at = now(), updated_by_id = $3 WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL")
        .bind(pid)
        .bind(p.workspace_id())
        .bind(user.id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE projects SET estimate_id = NULL WHERE id = $1 AND workspace_id = $2")
        .bind(pid)
        .bind(p.workspace_id())
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    no_content()
}

async fn v1_estimate_points(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, eid)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM estimates WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL)"
    )
    .bind(eid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;

    if !exists {
        return Err(ApiError(StatusCode::NOT_FOUND, json!({"error": "Estimate not found"})));
    }

    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {ESTIMATE_POINT_JSON} FROM estimate_points ep
         WHERE ep.estimate_id = $1 AND ep.project_id = $2 AND ep.workspace_id = $3 AND ep.deleted_at IS NULL
         ORDER BY ep.key, ep.created_at, ep.id"
    ))
    .bind(eid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;

    Ok(Json(Value::Array(rows)))
}

async fn v1_create_estimate_points(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, eid)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM estimates WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL)"
    )
    .bind(eid)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;

    if !exists {
        return Err(ApiError(StatusCode::NOT_FOUND, json!({"error": "Estimate not found"})));
    }

    let points: Vec<Value> = if let Some(arr) = b.as_array() {
        arr.clone()
    } else if let Some(arr) = b.get("estimate_points").and_then(Value::as_array) {
        arr.clone()
    } else {
        vec![b]
    };

    if points.is_empty() {
        return Err(ApiError::bad("Estimate points are required"));
    }

    let mut tx = st.db.begin().await?;
    let mut created_ids = Vec::new();
    for pt in points {
        let key = pt.get("key").and_then(Value::as_i64).unwrap_or(0) as i32;
        let value = s(&pt, "value").unwrap_or("");
        let desc = s(&pt, "description");
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO estimate_points (id, key, value, description, estimate_id, project_id, workspace_id, created_by_id, updated_by_id, created_at, updated_at)
             VALUES (gen_random_uuid(), $1, $2, COALESCE($3, ''), $4, $5, $6, $7, $7, now(), now())
             RETURNING id"
        )
        .bind(key)
        .bind(value)
        .bind(desc)
        .bind(eid)
        .bind(pid)
        .bind(p.workspace_id())
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
        created_ids.push(id);
    }
    tx.commit().await?;

    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {ESTIMATE_POINT_JSON} FROM estimate_points ep
         WHERE ep.id = ANY($1)
         ORDER BY ep.key, ep.created_at, ep.id"
    ))
    .bind(&created_ids)
    .fetch_all(&st.db)
    .await?;

    created(Value::Array(rows))
}

async fn v1_update_estimate_point(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, eid, epid)): Path<(String, Uuid, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let res = sqlx::query(
        "UPDATE estimate_points SET
            value = COALESCE($5, value),
            description = COALESCE($6, description),
            key = COALESCE($7, key),
            updated_at = now(),
            updated_by_id = $8
         WHERE id = $1 AND estimate_id = $2 AND project_id = $3 AND workspace_id = $4 AND deleted_at IS NULL",
    )
    .bind(epid)
    .bind(eid)
    .bind(pid)
    .bind(p.workspace_id())
    .bind(s(&b, "value"))
    .bind(s(&b, "description"))
    .bind(b.get("key").and_then(Value::as_i64).map(|v| v as i32))
    .bind(user.id)
    .execute(&st.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }

    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {ESTIMATE_POINT_JSON} FROM estimate_points ep
             WHERE ep.id = $1 AND ep.estimate_id = $2 AND ep.project_id = $3 AND ep.workspace_id = $4 AND ep.deleted_at IS NULL"
        ))
        .bind(epid)
        .bind(eid)
        .bind(pid)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn v1_delete_estimate_point(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, eid, epid)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;

    let res = sqlx::query(
        "UPDATE estimate_points SET deleted_at = now(), updated_at = now(), updated_by_id = $5
         WHERE id = $1 AND estimate_id = $2 AND project_id = $3 AND workspace_id = $4 AND deleted_at IS NULL",
    )
    .bind(epid)
    .bind(eid)
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
