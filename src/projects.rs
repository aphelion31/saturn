//! Projects, project members, invites, identifiers, favorites, deploy boards.
//! TODO (worker): port the `/api/workspaces/{slug}/projects/...` routes.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

use crate::common::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/workspaces/{slug}/estimates/", get(workspace_estimates))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/user-properties/",
            get(get_properties).patch(update_properties),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/estimates/",
            get(estimates),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issue-labels/",
            get(labels).post(create_label),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issue-labels/{label_id}/",
            get(label).patch(update_label).delete(delete_label),
        )
        .route("/api/workspaces/{slug}/project-identifiers/", get(project_identifiers))
        .route("/api/workspaces/{slug}/projects/{project_id}/members/", get(members))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/project-members/me/",
            get(my_membership),
        )
        .route(
            "/api/workspaces/{slug}/user-favorite-projects/",
            get(favorites).post(add_favorite),
        )
        .route(
            "/api/workspaces/{slug}/user-favorite-projects/{project_id}/",
            axum::routing::delete(remove_favorite),
        )
        .route("/api/workspaces/{slug}/projects/", get(list_projects).post(create))
        .route("/api/workspaces/{slug}/projects/details/", get(list_projects))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/",
            get(retrieve).patch(update).delete(destroy),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/archive/",
            axum::routing::post(archive).delete(unarchive),
        )
}

const LABEL_JSON: &str = "jsonb_build_object('id', id, 'name', name, 'color', color, 'parent', parent_id, 'project_id', project_id, 'workspace_id', workspace_id, 'sort_order', sort_order)";

async fn workspace_estimates(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(sqlx::query_scalar("SELECT (to_jsonb(e)-'workspace_id'-'project_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('workspace',e.workspace_id,'project',e.project_id,'created_by',e.created_by_id,'updated_by',e.updated_by_id,'points',COALESCE((SELECT jsonb_agg((to_jsonb(ep)-'workspace_id'-'project_id'-'estimate_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('workspace',ep.workspace_id,'project',ep.project_id,'estimate',ep.estimate_id,'created_by',ep.created_by_id,'updated_by',ep.updated_by_id) ORDER BY ep.key) FROM estimate_points ep WHERE ep.estimate_id=e.id AND ep.project_id=e.project_id AND ep.workspace_id=e.workspace_id AND ep.deleted_at IS NULL),'[]'::jsonb)) FROM estimates e JOIN projects p ON p.id=e.project_id AND p.workspace_id=e.workspace_id AND p.estimate_id=e.id AND p.deleted_at IS NULL WHERE e.workspace_id=$1 AND e.deleted_at IS NULL AND ($3=20 OR ($3=15 AND p.network=2) OR EXISTS(SELECT 1 FROM project_members pm WHERE pm.project_id=p.id AND pm.workspace_id=p.workspace_id AND pm.member_id=$2 AND pm.is_active AND pm.deleted_at IS NULL))").bind(ws.workspace_id).bind(user.id).bind(ws.role), &st.db).await
}

async fn estimates(State(st): State<AppState>, user: CurrentUser, Path((slug, id)): Path<(String, Uuid)>) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(sqlx::query_scalar("SELECT (to_jsonb(e)-'workspace_id'-'project_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('workspace', e.workspace_id, 'project', e.project_id, 'created_by', e.created_by_id, 'updated_by', e.updated_by_id, 'points', COALESCE((SELECT jsonb_agg((to_jsonb(ep)-'workspace_id'-'project_id'-'estimate_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('workspace', ep.workspace_id, 'project', ep.project_id, 'estimate', ep.estimate_id, 'created_by', ep.created_by_id, 'updated_by', ep.updated_by_id) ORDER BY ep.key) FROM estimate_points ep WHERE ep.estimate_id=e.id AND ep.project_id=e.project_id AND ep.workspace_id=e.workspace_id AND ep.deleted_at IS NULL), '[]'::jsonb)) FROM estimates e WHERE e.project_id=$1 AND e.workspace_id=$2 AND e.deleted_at IS NULL").bind(id).bind(p.workspace_id()), &st.db).await
}

async fn ensure_properties(st: &AppState, workspace: Uuid, project: Uuid, user: Uuid) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO project_user_properties (workspace_id,project_id,user_id,created_by_id,updated_by_id) VALUES ($1,$2,$3,$3,$3) ON CONFLICT DO NOTHING").bind(workspace).bind(project).bind(user).execute(&st.db).await?;
    Ok(())
}

async fn get_properties(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    ensure_properties(&st, p.workspace_id(), id, user.id).await?;
    json_row(sqlx::query_scalar("SELECT (to_jsonb(p)-'workspace_id'-'project_id'-'user_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('workspace',workspace_id,'project',project_id,'user',user_id,'created_by',created_by_id,'updated_by',updated_by_id) FROM project_user_properties p WHERE project_id=$1 AND workspace_id=$2 AND user_id=$3 AND deleted_at IS NULL").bind(id).bind(p.workspace_id()).bind(user.id), &st.db).await
}

async fn update_properties(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    for key in [
        "filters",
        "display_filters",
        "display_properties",
        "rich_filters",
        "preferences",
    ] {
        if b.get(key).is_some_and(Value::is_null) {
            return Err(ApiError::bad(format!("{key} may not be null")));
        }
    }
    ensure_properties(&st, p.workspace_id(), id, user.id).await?;
    sqlx::query("UPDATE project_user_properties SET filters=COALESCE($4,filters),display_filters=COALESCE($5,display_filters),display_properties=COALESCE($6,display_properties),rich_filters=COALESCE($7,rich_filters),preferences=COALESCE($8,preferences),updated_at=now(),updated_by_id=$3 WHERE project_id=$1 AND workspace_id=$2 AND user_id=$3 AND deleted_at IS NULL").bind(id).bind(p.workspace_id()).bind(user.id).bind(b.get("filters")).bind(b.get("display_filters")).bind(b.get("display_properties")).bind(b.get("rich_filters")).bind(b.get("preferences")).execute(&st.db).await?;
    get_properties(State(st), user, Path((slug, id))).await
}

async fn labels(State(st): State<AppState>, user: CurrentUser, Path((slug, id)): Path<(String, Uuid)>) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(sqlx::query_scalar(&format!("SELECT {LABEL_JSON} FROM labels WHERE project_id=$1 AND workspace_id=$2 AND deleted_at IS NULL ORDER BY sort_order")).bind(id).bind(p.workspace_id()), &st.db).await
}

async fn label(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id, label_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {LABEL_JSON} FROM labels WHERE project_id=$1 AND workspace_id=$2 AND id=$3 AND deleted_at IS NULL"
        ))
        .bind(id)
        .bind(p.workspace_id())
        .bind(label_id),
        &st.db,
    )
    .await
}

async fn create_label(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN])?;
    let name = s(&b, "name")
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 255)
        .ok_or_else(|| ApiError::bad("Invalid name"))?;
    let color = s(&b, "color")
        .filter(|s| !s.is_empty() && s.len() <= 255)
        .ok_or_else(|| ApiError::bad("Invalid color"))?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM labels WHERE project_id=$1 AND workspace_id=$2 AND lower(name)=lower($3) AND deleted_at IS NULL)").bind(id).bind(p.workspace_id()).bind(name).fetch_one(&st.db).await?;
    if exists {
        return Err(ApiError::bad_body(json!({"name":["LABEL_NAME_ALREADY_EXISTS"]})));
    }
    let parent = uuid_of(&b, "parent");
    if b.get("parent").is_some_and(|v| !v.is_null()) {
        let parent = parent.ok_or_else(|| ApiError::bad("Invalid parent"))?;
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM labels WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)").bind(parent).bind(id).bind(p.workspace_id()).fetch_one(&st.db).await?;
        if !valid {
            return Err(ApiError::bad("Invalid parent"));
        }
    }
    let value: Value = sqlx::query_scalar(&format!("INSERT INTO labels (project_id,workspace_id,name,color,parent_id,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$5,$6,$6) RETURNING {LABEL_JSON}")).bind(id).bind(p.workspace_id()).bind(name).bind(color).bind(parent).bind(user.id).fetch_one(&st.db).await?;
    created(value)
}

async fn update_label(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id, label_id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN])?;
    if b.get("parent").is_some() {
        return Err(ApiError::bad("Updating parent is not supported"));
    }
    if let Some(name) = s(&b, "name") {
        if name.trim().is_empty() || name.len() > 255 {
            return Err(ApiError::bad("Invalid name"));
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM labels WHERE project_id=$1 AND workspace_id=$2 AND id<>$3 AND lower(name)=lower($4) AND deleted_at IS NULL)").bind(id).bind(p.workspace_id()).bind(label_id).bind(name).fetch_one(&st.db).await?;
        if exists {
            return Err(ApiError::bad_body(json!({"name":["LABEL_NAME_ALREADY_EXISTS"]})));
        }
    }
    json_row(sqlx::query_scalar(&format!("UPDATE labels SET name=COALESCE($4,name),color=COALESCE($5,color),sort_order=COALESCE($6,sort_order),updated_by_id=$7,updated_at=now() WHERE project_id=$1 AND workspace_id=$2 AND id=$3 AND deleted_at IS NULL RETURNING {LABEL_JSON}")).bind(id).bind(p.workspace_id()).bind(label_id).bind(s(&b,"name")).bind(s(&b,"color")).bind(b.get("sort_order").and_then(Value::as_f64)).bind(user.id), &st.db).await
}

async fn delete_label(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id, label_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN])?;
    let r=sqlx::query("UPDATE labels SET deleted_at=now(),updated_by_id=$4 WHERE project_id=$1 AND workspace_id=$2 AND id=$3 AND deleted_at IS NULL").bind(id).bind(p.workspace_id()).bind(label_id).bind(user.id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn members(State(st): State<AppState>, user: CurrentUser, Path((slug, id)): Path<(String, Uuid)>) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let sql = format!(
        "SELECT jsonb_build_object('id', pm.id, 'member', {}, 'role', pm.role) FROM project_members pm JOIN users u ON u.id=pm.member_id JOIN workspace_members wm ON wm.member_id=u.id AND wm.workspace_id=pm.workspace_id AND wm.is_active AND wm.deleted_at IS NULL WHERE pm.project_id=$1 AND pm.workspace_id=$2 AND pm.is_active AND pm.deleted_at IS NULL AND NOT u.is_bot",
        crate::users::USER_LITE_JSON
    );
    json_rows(sqlx::query_scalar(&sql).bind(id).bind(p.workspace_id()), &st.db).await
}

async fn my_membership(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let sql = format!(
        "SELECT (to_jsonb(pm) - 'member_id' - 'project_id' - 'workspace_id') || jsonb_build_object('member', {}, 'project', jsonb_build_object('id', p.id, 'name', p.name, 'identifier', p.identifier, 'description', p.description, 'logo_props', p.logo_props, 'cover_image', p.cover_image, 'cover_image_url', p.cover_image), 'workspace', jsonb_build_object('id', w.id, 'name', w.name, 'slug', w.slug)) FROM project_members pm JOIN users u ON u.id=pm.member_id JOIN projects p ON p.id=pm.project_id AND p.workspace_id=pm.workspace_id JOIN workspaces w ON w.id=pm.workspace_id WHERE pm.project_id=$1 AND pm.workspace_id=$2 AND pm.member_id=$3 AND pm.is_active AND pm.deleted_at IS NULL",
        crate::users::USER_LITE_JSON
    );
    json_row(
        sqlx::query_scalar(&sql).bind(id).bind(p.workspace_id()).bind(user.id),
        &st.db,
    )
    .await
}

async fn favorites(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(sqlx::query_scalar("SELECT to_jsonb(f) FROM user_favorites f JOIN projects p ON p.id=f.project_id AND p.workspace_id=f.workspace_id AND p.deleted_at IS NULL WHERE f.workspace_id=$1 AND f.user_id=$2 AND f.entity_type='project' AND f.deleted_at IS NULL AND (EXISTS(SELECT 1 FROM project_members pm WHERE pm.project_id=p.id AND pm.workspace_id=p.workspace_id AND pm.member_id=$2 AND pm.is_active AND pm.deleted_at IS NULL) OR $3=20 OR ($3=15 AND p.network=2))").bind(ws.workspace_id).bind(user.id).bind(ws.role), &st.db).await
}

async fn add_favorite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<StatusCode> {
    let id = uuid_of(&b, "project").ok_or_else(|| ApiError::bad("Project is required"))?;
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    sqlx::query("INSERT INTO user_favorites (workspace_id, project_id, user_id, entity_type, entity_identifier, created_by_id) VALUES ($1,$2,$3,'project',$2,$3) ON CONFLICT DO NOTHING").bind(p.workspace_id()).bind(id).bind(user.id).execute(&st.db).await?;
    no_content()
}

async fn remove_favorite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let result = sqlx::query("UPDATE user_favorites SET deleted_at=now() WHERE workspace_id=$1 AND project_id=$2 AND entity_identifier=$2 AND user_id=$3 AND entity_type='project' AND deleted_at IS NULL").bind(ws.workspace_id).bind(id).bind(user.id).execute(&st.db).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn project_identifiers(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let name = query.get("name").map(|s| s.trim().to_uppercase()).unwrap_or_default();
    if name.is_empty() {
        return Err(ApiError::bad("Name is required"));
    }
    let identifiers: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'name', name, 'project', project_id) FROM project_identifiers WHERE workspace_id=$1 AND name=$2 AND deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .bind(name)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(json!({"exists": identifiers.len(), "identifiers": identifiers})))
}

async fn update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    if !ws.is_admin() {
        require_project(&st.db, &user, &slug, id).await?.require(&[ADMIN])?;
    }
    let result = sqlx::query("UPDATE projects SET name=COALESCE($3,name), description=COALESCE($4,description), network=COALESCE($5,network), updated_by_id=$6, updated_at=now() WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL AND archived_at IS NULL")
        .bind(id).bind(ws.workspace_id).bind(s(&b,"name")).bind(s(&b,"description"))
        .bind(b.get("network").and_then(Value::as_i64).map(|n| n as i16)).bind(user.id).execute(&st.db).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    retrieve(State(st), user, Path((slug, id))).await
}

async fn archive(State(st): State<AppState>, user: CurrentUser, Path((slug, id)): Path<(String, Uuid)>) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER])?;
    json_row(sqlx::query_scalar("UPDATE projects SET archived_at=now() WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL RETURNING jsonb_build_object('archived_at',archived_at)").bind(id).bind(p.workspace_id()), &st.db).await
}

async fn unarchive(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER])?;
    sqlx::query("UPDATE projects SET archived_at=NULL WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL")
        .bind(id)
        .bind(p.workspace_id())
        .execute(&st.db)
        .await?;
    no_content()
}

async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    if let Some(lead) = uuid_of(&b, "project_lead") {
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM workspace_members WHERE workspace_id=$1 AND member_id=$2 AND is_active AND deleted_at IS NULL)")
            .bind(ws.workspace_id).bind(lead).fetch_one(&st.db).await?;
        if !valid {
            return Err(ApiError::bad_body(
                json!({"project_lead": ["Invalid workspace member."]}),
            ));
        }
    }
    let name = s(&b, "name")
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .ok_or_else(|| ApiError::bad_body(json!({"name": ["This field is required."]})))?;
    let identifier = s(&b, "identifier")
        .map(str::trim)
        .filter(|x| !x.is_empty() && x.len() <= 12)
        .ok_or_else(|| ApiError::bad_body(json!({"identifier": ["Invalid identifier."]})))?;
    for (field, value) in [("name", name), ("identifier", identifier)] {
        let taken: bool = if field == "name" {
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE workspace_id=$1 AND name=$2 AND deleted_at IS NULL)",
            )
            .bind(ws.workspace_id)
            .bind(value)
            .fetch_one(&st.db)
            .await?
        } else {
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE workspace_id=$1 AND identifier=$2 AND deleted_at IS NULL)",
            )
            .bind(ws.workspace_id)
            .bind(value.to_uppercase())
            .fetch_one(&st.db)
            .await?
        };
        if taken {
            return Err(ApiError::bad_body(
                json!({field: [if field == "name" {"PROJECT_NAME_ALREADY_EXIST"} else {"PROJECT_IDENTIFIER_ALREADY_EXIST"}]}),
            ));
        }
    }
    let mut tx = st.db.begin().await?;
    let id = create_project(&mut tx, ws.workspace_id, user.id, name, identifier, &b).await?;
    tx.commit().await?;
    let Json(project) = retrieve(State(st), user, Path((slug, id))).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

async fn retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let Json(projects) = list_projects(State(st), user, Path(slug)).await?;
    projects
        .as_array()
        .and_then(|rows| rows.iter().find(|v| v["id"].as_str() == Some(&project_id.to_string())))
        .cloned()
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

async fn destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    if !ws.is_admin() {
        let p = require_project(&st.db, &user, &slug, project_id).await?;
        p.require(&[ADMIN])?;
    }
    let r = sqlx::query(
        "UPDATE projects SET deleted_at=now(), updated_by_id=$3 WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL",
    )
    .bind(project_id)
    .bind(ws.workspace_id)
    .bind(user.id)
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn list_projects(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(sqlx::query_scalar(
        "SELECT (to_jsonb(p) - 'workspace_id' - 'created_by_id' - 'updated_by_id' - 'default_assignee_id' - 'project_lead_id' - 'estimate_id' - 'default_state_id' - 'cover_image_asset_id') ||
         jsonb_build_object('workspace', p.workspace_id, 'created_by', p.created_by_id, 'updated_by', p.updated_by_id,
         'default_assignee', p.default_assignee_id, 'project_lead', p.project_lead_id, 'estimate', p.estimate_id,
         'default_state', p.default_state_id, 'cover_image_asset', p.cover_image_asset_id, 'inbox_view', p.intake_view,
         'cover_image_url', p.cover_image, 'member_role', pm.role, 'is_member', pm.id IS NOT NULL,
         'sort_order', pup.sort_order,
         'members', COALESCE((SELECT jsonb_agg(m.member_id) FROM project_members m JOIN users u ON u.id=m.member_id
             WHERE m.project_id=p.id AND m.workspace_id=p.workspace_id AND m.deleted_at IS NULL AND m.is_active AND NOT u.is_bot), '[]'::jsonb),
         'is_favorite', EXISTS (SELECT 1 FROM user_favorites f WHERE f.workspace_id=p.workspace_id AND f.project_id=p.id AND f.user_id=$2 AND f.entity_type='project' AND f.deleted_at IS NULL),
         'next_work_item_sequence', COALESCE((SELECT MAX(seq.sequence) + 1 FROM issue_sequences seq WHERE seq.project_id=p.id AND seq.workspace_id=p.workspace_id AND seq.deleted_at IS NULL), 1))
         FROM projects p LEFT JOIN project_members pm ON pm.project_id=p.id AND pm.workspace_id=p.workspace_id AND pm.member_id=$2 AND pm.is_active AND pm.deleted_at IS NULL
         LEFT JOIN project_user_properties pup ON pup.project_id=p.id AND pup.workspace_id=p.workspace_id AND pup.user_id=$2 AND pup.deleted_at IS NULL
         WHERE p.workspace_id=$1 AND p.deleted_at IS NULL AND (pm.id IS NOT NULL OR $3 = 20 OR ($3 = 15 AND p.network=2))
         ORDER BY pup.sort_order, p.name")
        .bind(ws.workspace_id).bind(user.id).bind(ws.role), &st.db).await
}

/// Insert a project with its creator as admin member, default states and identifier row.
/// Shared by the create endpoint and `saturn seed`. `extra` may carry optional fields
/// (`description`, `network`, `project_lead`, `logo_props`, `timezone`).
pub async fn create_project(
    tx: &mut sqlx::PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
    name: &str,
    identifier: &str,
    extra: &Value,
) -> Result<Uuid, ApiError> {
    let identifier = identifier.trim().to_uppercase();
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO projects (name, identifier, workspace_id, created_by_id, updated_by_id, description, network,
                               project_lead_id, logo_props, timezone)
         VALUES ($1, $2, $3, $4, $4, COALESCE($5, ''), COALESCE($6, 2), $7, COALESCE($8, '{}'::jsonb), COALESCE($9, 'UTC'))
         RETURNING id",
    )
    .bind(name)
    .bind(&identifier)
    .bind(workspace_id)
    .bind(user_id)
    .bind(s(extra, "description"))
    .bind(extra.get("network").and_then(Value::as_i64).map(|n| n as i16))
    .bind(uuid_of(extra, "project_lead"))
    .bind(extra.get("logo_props"))
    .bind(s(extra, "timezone"))
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO project_identifiers (name, project_id, workspace_id, created_by_id, updated_by_id) VALUES ($1, $2, $3, $4, $4)")
        .bind(&identifier)
        .bind(id)
        .bind(workspace_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO project_members (workspace_id, project_id, member_id, role, created_by_id) VALUES ($1, $2, $3, 20, $3)",
    )
    .bind(workspace_id)
    .bind(id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    if let Some(lead) = uuid_of(extra, "project_lead")
        && lead != user_id
    {
        sqlx::query(
            "INSERT INTO project_members (workspace_id, project_id, member_id, role, created_by_id) VALUES ($1, $2, $3, 20, $4)",
        )
        .bind(workspace_id)
        .bind(id)
        .bind(lead)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    crate::states::create_defaults(tx, workspace_id, id, user_id).await?;
    Ok(id)
}
