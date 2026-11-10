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
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/project-deploy-boards/",
            get(get_board).post(create_board),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/project-deploy-boards/{board_id}/",
            axum::routing::patch(update_board),
        )
        .route(
            "/api/users/me/workspaces/{slug}/projects/invitations/",
            axum::routing::post(join_projects),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/join/{invite_id}/",
            axum::routing::post(accept_invite),
        )
        .route("/api/workspaces/{slug}/estimates/", get(workspace_estimates))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/user-properties/",
            get(get_properties).patch(update_properties),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/estimates/",
            get(estimates).post(create_estimate),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/estimates/{estimate_id}/",
            get(get_estimate).patch(update_estimate).delete(delete_estimate),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/estimates/{estimate_id}/estimate-points/",
            axum::routing::post(create_point),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/estimates/{estimate_id}/estimate-points/{point_id}/",
            axum::routing::patch(update_point).delete(delete_point),
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
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/members/",
            get(members).post(add_members),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/members/{member_id}/",
            axum::routing::patch(update_member).delete(remove_member),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/members/leave/",
            axum::routing::post(leave_project),
        )
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

const BOARD_JSON: &str = "(to_jsonb(d)-'workspace_id'-'project_id'-'intake_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('workspace',d.workspace_id,'project',d.project_id,'intake',d.intake_id,'created_by',d.created_by_id,'updated_by',d.updated_by_id,'project_details',jsonb_build_object('id',p.id,'name',p.name,'identifier',p.identifier,'description',p.description,'cover_image',p.cover_image,'cover_image_url',p.cover_image,'logo_props',p.logo_props),'workspace_detail',jsonb_build_object('id',w.id,'name',w.name,'slug',w.slug))";

async fn get_board(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let sql = format!(
        "SELECT {BOARD_JSON} FROM deploy_boards d JOIN projects p ON p.id=d.project_id AND p.workspace_id=d.workspace_id JOIN workspaces w ON w.id=d.workspace_id WHERE d.workspace_id=$1 AND d.project_id=$2 AND d.entity_name='project' AND d.entity_identifier=$2 AND d.deleted_at IS NULL"
    );
    let row: Option<Value> = sqlx::query_scalar(&sql)
        .bind(p.workspace_id())
        .bind(project)
        .fetch_optional(&st.db)
        .await?;
    Ok(Json(row.unwrap_or_else(|| json!({}))))
}

async fn create_board(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER])?;
    let intake = uuid_of(&b, "intake");
    if b.get("intake").is_some_and(|x| !x.is_null()) && intake.is_none() {
        return Err(ApiError::bad("Invalid intake"));
    }
    if let Some(id) = intake {
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM intakes WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)").bind(id).bind(project).bind(p.workspace_id()).fetch_one(&st.db).await?;
        if !valid {
            return Err(ApiError::bad("Invalid intake"));
        }
    }
    let views = b
        .get("views")
        .cloned()
        .unwrap_or_else(|| json!({"list":true,"kanban":true,"calendar":true,"gantt":true,"spreadsheet":true}));
    sqlx::query("INSERT INTO deploy_boards (workspace_id,project_id,entity_identifier,entity_name,anchor,intake_id,view_props,is_comments_enabled,is_reactions_enabled,is_votes_enabled,created_by_id,updated_by_id) VALUES ($1,$2,$2,'project',replace(gen_random_uuid()::text,'-',''),$3,$4,$5,$6,$7,$8,$8) ON CONFLICT (entity_name,entity_identifier) WHERE deleted_at IS NULL DO UPDATE SET intake_id=EXCLUDED.intake_id,view_props=EXCLUDED.view_props,is_comments_enabled=EXCLUDED.is_comments_enabled,is_reactions_enabled=EXCLUDED.is_reactions_enabled,is_votes_enabled=EXCLUDED.is_votes_enabled,updated_by_id=EXCLUDED.updated_by_id,updated_at=now() WHERE deploy_boards.workspace_id=EXCLUDED.workspace_id AND deploy_boards.project_id=EXCLUDED.project_id").bind(p.workspace_id()).bind(project).bind(intake).bind(views).bind(b.get("is_comments_enabled").and_then(Value::as_bool).unwrap_or(false)).bind(b.get("is_reactions_enabled").and_then(Value::as_bool).unwrap_or(false)).bind(b.get("is_votes_enabled").and_then(Value::as_bool).unwrap_or(false)).bind(user.id).execute(&st.db).await?;
    get_board(State(st), user, Path((slug, project))).await
}

async fn update_board(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER])?;
    if b.get("intake").is_some() {
        return Err(ApiError::bad("Use board creation to change intake"));
    }
    for key in [
        "is_comments_enabled",
        "is_reactions_enabled",
        "is_votes_enabled",
        "is_activity_enabled",
        "is_disabled",
    ] {
        if b.get(key).is_some_and(|v| !v.is_boolean()) {
            return Err(ApiError::bad(format!("Invalid {key}")));
        }
    }
    let r=sqlx::query("UPDATE deploy_boards SET is_comments_enabled=COALESCE($4,is_comments_enabled),is_reactions_enabled=COALESCE($5,is_reactions_enabled),is_votes_enabled=COALESCE($6,is_votes_enabled),is_activity_enabled=COALESCE($7,is_activity_enabled),is_disabled=COALESCE($8,is_disabled),view_props=COALESCE($9,view_props),updated_by_id=$10,updated_at=now() WHERE workspace_id=$1 AND project_id=$2 AND id=$3 AND entity_name='project' AND entity_identifier=$2 AND deleted_at IS NULL").bind(p.workspace_id()).bind(project).bind(id).bind(b.get("is_comments_enabled").and_then(Value::as_bool)).bind(b.get("is_reactions_enabled").and_then(Value::as_bool)).bind(b.get("is_votes_enabled").and_then(Value::as_bool)).bind(b.get("is_activity_enabled").and_then(Value::as_bool)).bind(b.get("is_disabled").and_then(Value::as_bool)).bind(b.get("view_props")).bind(user.id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    get_board(State(st), user, Path((slug, project))).await
}

async fn join_projects(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let ids = b
        .get("project_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad("project_ids is required"))?;
    let mut tx = st.db.begin().await?;
    for v in ids {
        let id = v
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| ApiError::bad("Invalid project id"))?;
        let network: i16 =
            sqlx::query_scalar("SELECT network FROM projects WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL")
                .bind(id)
                .bind(ws.workspace_id)
                .fetch_one(&mut *tx)
                .await?;
        if network != 2 && !ws.is_admin() {
            return Err(ApiError::forbidden());
        }
        sqlx::query("INSERT INTO project_members (workspace_id,project_id,member_id,role,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$3,$3) ON CONFLICT (project_id,member_id) WHERE deleted_at IS NULL DO UPDATE SET is_active=true,role=EXCLUDED.role,updated_at=now(),updated_by_id=EXCLUDED.updated_by_id WHERE NOT project_members.is_active").bind(ws.workspace_id).bind(id).bind(user.id).bind(ws.role).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO project_user_properties (workspace_id,project_id,user_id,created_by_id,updated_by_id) VALUES ($1,$2,$3,$3,$3) ON CONFLICT DO NOTHING").bind(ws.workspace_id).bind(id).bind(user.id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"message":"Projects joined successfully"})),
    ))
}

async fn accept_invite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, invite)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let token = s(&b, "token")
        .filter(|v| !v.is_empty())
        .ok_or_else(ApiError::forbidden)?;
    let accepted = b
        .get("accepted")
        .and_then(Value::as_bool)
        .ok_or_else(|| ApiError::bad("`accepted` must be a boolean"))?;
    let mut tx = st.db.begin().await?;
    let row:Option<(Uuid,i16,String,String,Option<chrono::DateTime<chrono::Utc>>)>=sqlx::query_as("SELECT i.workspace_id,i.role,i.email,i.token,i.responded_at FROM project_member_invites i JOIN workspaces w ON w.id=i.workspace_id AND w.deleted_at IS NULL JOIN projects p ON p.id=i.project_id AND p.workspace_id=i.workspace_id AND p.deleted_at IS NULL WHERE i.id=$1 AND i.project_id=$2 AND w.slug=$3 AND i.deleted_at IS NULL FOR UPDATE OF i").bind(invite).bind(project).bind(&slug).fetch_optional(&mut *tx).await?;
    let (workspace, role, email, expected, responded) = row.ok_or_else(ApiError::not_found)?;
    if token != expected || !email.eq_ignore_ascii_case(&user.email) {
        return Err(ApiError::forbidden());
    }
    if responded.is_some() {
        return Err(ApiError::bad("You have already responded to the invitation request"));
    }
    if ![5, 15, 20].contains(&role) {
        return Err(ApiError::bad("Invalid invitation role"));
    }
    sqlx::query("UPDATE project_member_invites SET accepted=$4,responded_at=now(),updated_at=now(),updated_by_id=$5 WHERE id=$1 AND project_id=$2 AND workspace_id=$3").bind(invite).bind(project).bind(workspace).bind(accepted).bind(user.id).execute(&mut *tx).await?;
    if accepted {
        sqlx::query("INSERT INTO workspace_members (workspace_id,member_id,role,created_by_id,updated_by_id) VALUES ($1,$2,$3,$2,$2) ON CONFLICT (workspace_id,member_id) WHERE deleted_at IS NULL DO UPDATE SET is_active=true,updated_at=now()").bind(workspace).bind(user.id).bind(if role>=15 {15i16} else {role}).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO project_members (workspace_id,project_id,member_id,role,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$3,$3) ON CONFLICT (project_id,member_id) WHERE deleted_at IS NULL DO UPDATE SET is_active=true,role=EXCLUDED.role,updated_at=now()").bind(workspace).bind(project).bind(user.id).bind(role).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({"message":if accepted {"Project Invitation Accepted"} else {"Project Invitation was not accepted"}}),
    ))
}

async fn add_members(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN])?;
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let requested = b
        .get("members")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ApiError::bad("At least one member is required"))?;
    let mut tx = st.db.begin().await?;
    for member in requested {
        let id = uuid_of(member, "member_id").ok_or_else(|| ApiError::bad("Invalid member"))?;
        let role = member.get("role").and_then(Value::as_i64).unwrap_or(5);
        if ![5, 15, 20].contains(&role) {
            return Err(ApiError::bad("Invalid role"));
        }
        if !ws.is_admin() {
            let existing:Option<i16>=sqlx::query_scalar("SELECT role FROM project_members WHERE workspace_id=$1 AND project_id=$2 AND member_id=$3 AND is_active AND deleted_at IS NULL").bind(p.workspace_id()).bind(project).bind(id).fetch_optional(&mut *tx).await?;
            if role >= 20 || existing.is_some_and(|r| r >= 20) {
                return Err(ApiError::forbidden());
            }
        }
        let ws_role:i16=sqlx::query_scalar("SELECT role FROM workspace_members WHERE workspace_id=$1 AND member_id=$2 AND is_active AND deleted_at IS NULL").bind(p.workspace_id()).bind(id).fetch_optional(&mut *tx).await?.ok_or_else(|| ApiError::bad("Invalid workspace member"))?;
        if (ws_role == 20 && role != 20) || (ws_role == 5 && role != 5) {
            return Err(ApiError::bad("Invalid role for workspace member"));
        }
        sqlx::query("INSERT INTO project_members (workspace_id,project_id,member_id,role,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$5,$5) ON CONFLICT (project_id,member_id) WHERE deleted_at IS NULL DO UPDATE SET role=EXCLUDED.role,is_active=true,updated_at=now(),updated_by_id=EXCLUDED.updated_by_id").bind(p.workspace_id()).bind(project).bind(id).bind(role as i16).bind(user.id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO project_user_properties (workspace_id,project_id,user_id,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$4) ON CONFLICT DO NOTHING").bind(p.workspace_id()).bind(project).bind(id).bind(user.id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    let Json(rows) = members(State(st), user, Path((slug, project))).await?;
    Ok((StatusCode::CREATED, Json(rows)))
}

async fn update_member(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN])?;
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let (target,target_role,workspace_role):(Uuid,i16,i16)=sqlx::query_as("SELECT pm.member_id,pm.role,wm.role FROM project_members pm JOIN workspace_members wm ON wm.member_id=pm.member_id AND wm.workspace_id=pm.workspace_id AND wm.is_active AND wm.deleted_at IS NULL WHERE pm.id=$1 AND pm.project_id=$2 AND pm.workspace_id=$3 AND pm.is_active AND pm.deleted_at IS NULL").bind(id).bind(project).bind(p.workspace_id()).fetch_one(&st.db).await?;
    if b.get("preferences").is_some() || b.get("view_props").is_some() || b.get("default_props").is_some() {
        if target != user.id {
            return Err(ApiError::forbidden());
        }
        return Err(ApiError::bad("Personal settings are not supported here"));
    }
    if !ws.is_admin() && (target == user.id || target_role >= 20) {
        return Err(ApiError::forbidden());
    }
    let role = if let Some(v) = b.get("role") {
        let r = v
            .as_i64()
            .filter(|r| [5, 15, 20].contains(r))
            .ok_or_else(|| ApiError::bad("Invalid role"))?;
        if (workspace_role == 5 && r != 5) || (!ws.is_admin() && r >= 20) {
            return Err(ApiError::forbidden());
        }
        Some(r as i16)
    } else {
        None
    };
    let active = if let Some(v) = b.get("is_active") {
        Some(v.as_bool().ok_or_else(|| ApiError::bad("Invalid active status"))?)
    } else {
        None
    };
    let sql = "UPDATE project_members SET role=COALESCE($4,role),is_active=COALESCE($5,is_active),updated_by_id=$6,updated_at=now() WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL RETURNING to_jsonb(project_members) - 'preferences' - 'view_props' - 'default_props'";
    json_row(
        sqlx::query_scalar(&sql)
            .bind(id)
            .bind(project)
            .bind(p.workspace_id())
            .bind(role)
            .bind(active)
            .bind(user.id),
        &st.db,
    )
    .await
}

async fn remove_member(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN])?;
    let r=sqlx::query("UPDATE project_members SET is_active=false,updated_at=now(),updated_by_id=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND member_id<>$4 AND role<=20 AND is_active AND deleted_at IS NULL").bind(id).bind(project).bind(p.workspace_id()).bind(user.id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn leave_project(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let mut tx = st.db.begin().await?;
    sqlx::query("SELECT id FROM projects WHERE id=$1 AND workspace_id=$2 FOR UPDATE")
        .bind(project)
        .bind(p.workspace_id())
        .fetch_one(&mut *tx)
        .await?;
    let (role,):(i16,)=sqlx::query_as("SELECT role FROM project_members WHERE workspace_id=$1 AND project_id=$2 AND member_id=$3 AND is_active AND deleted_at IS NULL").bind(p.workspace_id()).bind(project).bind(user.id).fetch_one(&mut *tx).await?;
    if role == 20 {
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM project_members WHERE workspace_id=$1 AND project_id=$2 AND role=20 AND is_active AND deleted_at IS NULL").bind(p.workspace_id()).bind(project).fetch_one(&mut *tx).await?;
        if count <= 1 {
            return Err(ApiError::bad("The only project admin cannot leave"));
        }
    }
    sqlx::query("UPDATE project_members SET is_active=false,updated_at=now(),updated_by_id=$3 WHERE workspace_id=$1 AND project_id=$2 AND member_id=$3 AND is_active AND deleted_at IS NULL").bind(p.workspace_id()).bind(project).bind(user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    no_content()
}

async fn get_estimate(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, estimate)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let Json(rows) = estimates(State(st), user, Path((slug, project))).await?;
    rows.as_array()
        .and_then(|v| v.iter().find(|x| x["id"].as_str() == Some(&estimate.to_string())))
        .cloned()
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

async fn create_estimate(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER])?;
    let e = b
        .get("estimate")
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError::bad("Estimate is required"))?;
    let name = e
        .get("name")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 255)
        .ok_or_else(|| ApiError::bad("Invalid estimate name"))?;
    let kind = e.get("type").and_then(Value::as_str).unwrap_or("categories");
    if kind.len() > 255 {
        return Err(ApiError::bad("Invalid estimate type"));
    }
    let points = b
        .get("estimate_points")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad("Estimate points are required"))?;
    for point in points {
        validate_point(point, true)?;
    }
    let mut tx = st.db.begin().await?;
    let id:Uuid=sqlx::query_scalar("INSERT INTO estimates (workspace_id,project_id,name,type,last_used,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$5,$6,$6) RETURNING id").bind(p.workspace_id()).bind(project).bind(name).bind(kind).bind(e.get("last_used").and_then(Value::as_bool).unwrap_or(false)).bind(user.id).fetch_one(&mut *tx).await?;
    for point in points {
        sqlx::query("INSERT INTO estimate_points (workspace_id,project_id,estimate_id,key,value,description,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$7)").bind(p.workspace_id()).bind(project).bind(id).bind(point["key"].as_i64().unwrap() as i32).bind(s(point,"value").unwrap()).bind(s(point,"description").unwrap_or("")).bind(user.id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    get_estimate(State(st), user, Path((slug, project, id))).await
}

fn validate_point(b: &Value, required: bool) -> Result<(), ApiError> {
    if required || b.get("key").is_some() {
        let k = b
            .get("key")
            .and_then(Value::as_i64)
            .ok_or_else(|| ApiError::bad("Invalid key"))?;
        if i32::try_from(k).is_err() {
            return Err(ApiError::bad("Invalid key"));
        }
    }
    if required || b.get("value").is_some() {
        s(b, "value")
            .filter(|s| !s.is_empty() && s.len() <= 20)
            .ok_or_else(|| ApiError::bad("Invalid value"))?;
    }
    Ok(())
}

async fn update_estimate(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER])?;
    let points = b
        .get("estimate_points")
        .and_then(Value::as_array)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| ApiError::bad("Estimate points are required"))?;
    for point in points {
        validate_point(point, false)?;
        if uuid_of(point, "id").is_none() {
            return Err(ApiError::bad("Invalid point id"));
        }
    }
    let e = b.get("estimate").unwrap_or(&Value::Null);
    let mut tx = st.db.begin().await?;
    let r=sqlx::query("UPDATE estimates SET name=COALESCE($4,name),type=COALESCE($5,type),updated_at=now(),updated_by_id=$6 WHERE workspace_id=$1 AND project_id=$2 AND id=$3 AND deleted_at IS NULL").bind(p.workspace_id()).bind(project).bind(id).bind(s(e,"name")).bind(s(e,"type")).bind(user.id).execute(&mut *tx).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    for point in points {
        let r=sqlx::query("UPDATE estimate_points SET key=COALESCE($5,key),value=COALESCE($6,value),updated_at=now(),updated_by_id=$7 WHERE workspace_id=$1 AND project_id=$2 AND estimate_id=$3 AND id=$4 AND deleted_at IS NULL").bind(p.workspace_id()).bind(project).bind(id).bind(uuid_of(point,"id")).bind(point.get("key").and_then(Value::as_i64).map(|n|n as i32)).bind(s(point,"value")).bind(user.id).execute(&mut *tx).await?;
        if r.rows_affected() == 0 {
            return Err(ApiError::not_found());
        }
    }
    tx.commit().await?;
    get_estimate(State(st), user, Path((slug, project, id))).await
}

async fn delete_estimate(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER])?;
    let r=sqlx::query("UPDATE estimates SET deleted_at=now(),updated_by_id=$4 WHERE workspace_id=$1 AND project_id=$2 AND id=$3 AND deleted_at IS NULL").bind(p.workspace_id()).bind(project).bind(id).bind(user.id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

const POINT_JSON: &str = "(to_jsonb(estimate_points)-'workspace_id'-'project_id'-'estimate_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('workspace',workspace_id,'project',project_id,'estimate',estimate_id,'created_by',created_by_id,'updated_by',updated_by_id)";

async fn create_point(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, estimate)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER])?;
    validate_point(&b, true)?;
    json_row(sqlx::query_scalar(&format!("INSERT INTO estimate_points (workspace_id,project_id,estimate_id,key,value,description,created_by_id,updated_by_id) SELECT $1,$2,e.id,$4,$5,$6,$7,$7 FROM estimates e WHERE e.id=$3 AND e.project_id=$2 AND e.workspace_id=$1 AND e.deleted_at IS NULL RETURNING {POINT_JSON}")).bind(p.workspace_id()).bind(project).bind(estimate).bind(b["key"].as_i64().unwrap() as i32).bind(s(&b,"value")).bind(s(&b,"description").unwrap_or("")).bind(user.id),&st.db).await
}

async fn update_point(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, estimate, id)): Path<(String, Uuid, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER])?;
    validate_point(&b, false)?;
    json_row(sqlx::query_scalar(&format!("UPDATE estimate_points SET key=COALESCE($5,key),value=COALESCE($6,value),description=COALESCE($7,description),updated_at=now(),updated_by_id=$8 WHERE workspace_id=$1 AND project_id=$2 AND estimate_id=$3 AND id=$4 AND deleted_at IS NULL RETURNING {POINT_JSON}")).bind(p.workspace_id()).bind(project).bind(estimate).bind(id).bind(b.get("key").and_then(Value::as_i64).map(|v|v as i32)).bind(s(&b,"value")).bind(s(&b,"description")).bind(user.id),&st.db).await
}

async fn delete_point(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, estimate, id)): Path<(String, Uuid, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER])?;
    let replacement = uuid_of(&b, "new_estimate_id");
    if b.get("new_estimate_id").is_some_and(|v| !v.is_null()) && replacement.is_none() {
        return Err(ApiError::bad("Invalid new estimate point"));
    }
    let mut tx = st.db.begin().await?;
    let key: i32=sqlx::query_scalar("SELECT key FROM estimate_points WHERE workspace_id=$1 AND project_id=$2 AND estimate_id=$3 AND id=$4 AND deleted_at IS NULL FOR UPDATE").bind(p.workspace_id()).bind(project).bind(estimate).bind(id).fetch_one(&mut *tx).await?;
    if let Some(new_id) = replacement {
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM estimate_points WHERE workspace_id=$1 AND project_id=$2 AND estimate_id=$3 AND id=$4 AND id<>$5 AND deleted_at IS NULL)").bind(p.workspace_id()).bind(project).bind(estimate).bind(new_id).bind(id).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(ApiError::bad("Invalid new estimate point"));
        }
    }
    sqlx::query("UPDATE issues SET estimate_point_id=$4,updated_at=now(),updated_by_id=$5 WHERE workspace_id=$1 AND project_id=$2 AND estimate_point_id=$3 AND deleted_at IS NULL").bind(p.workspace_id()).bind(project).bind(id).bind(replacement).bind(user.id).execute(&mut *tx).await?;
    sqlx::query("UPDATE estimate_points SET deleted_at=now(),updated_by_id=$5 WHERE workspace_id=$1 AND project_id=$2 AND estimate_id=$3 AND id=$4 AND deleted_at IS NULL").bind(p.workspace_id()).bind(project).bind(estimate).bind(id).bind(user.id).execute(&mut *tx).await?;
    let rows:Vec<Value>=sqlx::query_scalar(&format!("UPDATE estimate_points SET key=key-1,updated_at=now() WHERE workspace_id=$1 AND project_id=$2 AND estimate_id=$3 AND key>$4 AND deleted_at IS NULL RETURNING {POINT_JSON}")).bind(p.workspace_id()).bind(project).bind(estimate).bind(key).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!(rows)))
}

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
