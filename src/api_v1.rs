use axum::{
    Router,
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
        .route("/api/v1/workspaces/{slug}/projects/", get(projects))
        .route("/api/v1/workspaces/{slug}/projects/{project_id}/", get(project))
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/archive/",
            axum::routing::post(archive_project).delete(unarchive_project),
        )
        .route("/api/v1/workspaces/{slug}/projects/{project_id}/states/", get(states))
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/states/{state_id}/",
            get(state),
        )
        .route("/api/v1/workspaces/{slug}/projects/{project_id}/labels/", get(labels))
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/labels/{label_id}/",
            get(label),
        )
}

async fn set_archived(st: AppState, user: CurrentUser, slug: String, id: Uuid, archive: bool) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let result = sqlx::query("UPDATE projects SET archived_at=CASE WHEN $3 THEN now() ELSE NULL END, updated_at=now(), updated_by_id=$4 WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL")
        .bind(id).bind(p.workspace_id()).bind(archive).bind(user.id).execute(&st.db).await?;
    if result.rows_affected() == 0 {
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
