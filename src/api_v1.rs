use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};

use crate::common::*;
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
