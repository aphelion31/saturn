//! Project states. Reference module: copy its patterns (permission check first, tenant filter on
//! every query, JSON built in SQL, DRF-shaped errors) when porting other areas.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/states/",
            get(list).post(create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/states/{pk}/",
            get(retrieve).patch(update).delete(destroy),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/states/{pk}/mark-default/",
            post(mark_default),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/intake-state/",
            get(intake_state),
        )
        .route("/api/workspaces/{slug}/states/", get(workspace_states))
}

/// StateSerializer fields.
pub const STATE_JSON: &str =
    "jsonb_build_object('id', s.id, 'project_id', s.project_id, 'workspace_id', s.workspace_id,
    'name', s.name, 'color', s.color, 'group', s.group, 'default', s.default, 'description', s.description,
    'sequence', s.sequence)";

/// Plane adds `order` = position / count within each group.
const STATE_ORDERED: &str = "SELECT jsonb_build_object('id', s.id, 'project_id', s.project_id, 'workspace_id', s.workspace_id,
    'name', s.name, 'color', s.color, 'group', s.group, 'default', s.default, 'description', s.description,
    'sequence', s.sequence,
    'order', (row_number() OVER (PARTITION BY s.group ORDER BY s.sequence))::float8 / count(*) OVER (PARTITION BY s.group))
    FROM states s";

#[derive(Deserialize)]
struct ListQ {
    grouped: Option<String>,
}

async fn list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<ListQ>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "{STATE_ORDERED} WHERE s.project_id = $1 AND s.workspace_id = $2 AND NOT s.is_triage AND s.deleted_at IS NULL
         ORDER BY s.sequence"
    ))
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    if q.grouped.as_deref() == Some("true") {
        let mut grouped = serde_json::Map::new();
        for r in rows {
            let g = r["group"].as_str().unwrap_or_default().to_string();
            grouped
                .entry(g)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap()
                .push(r);
        }
        return Ok(Json(Value::Object(grouped)));
    }
    Ok(Json(Value::Array(rows)))
}

async fn workspace_states(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    json_rows(
        sqlx::query_scalar(&format!(
            "{STATE_ORDERED} JOIN project_members pm ON pm.project_id = s.project_id AND pm.member_id = $2 AND pm.is_active
             AND pm.deleted_at IS NULL
             WHERE s.workspace_id = $1 AND NOT s.is_triage AND s.deleted_at IS NULL ORDER BY s.sequence"
        ))
        .bind(ws.workspace_id)
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {STATE_JSON} FROM states s WHERE s.id = $1 AND s.project_id = $2 AND NOT s.is_triage AND s.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(project_id),
        &st.db,
    )
    .await
}

/// Django `slugify`: lowercase, keep alphanumerics, spaces and dashes collapse to one dash.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else if (c.is_whitespace() || c == '-') && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

const GROUPS: &[&str] = &["backlog", "unstarted", "started", "completed", "cancelled", "triage"];

async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN])?;
    let name = s(&b, "name").map(str::trim).filter(|n| !n.is_empty());
    let Some(name) = name else {
        return Err(ApiError::bad_body(json!({"name": ["This field is required."]})));
    };
    let Some(color) = s(&b, "color") else {
        return Err(ApiError::bad_body(json!({"color": ["This field is required."]})));
    };
    let group = s(&b, "group").unwrap_or("backlog");
    if !GROUPS.contains(&group) {
        return Err(ApiError::bad_body(
            json!({"group": [format!("\"{group}\" is not a valid choice.")]}),
        ));
    }
    let taken: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM states WHERE project_id = $1 AND name = $2 AND deleted_at IS NULL)",
    )
    .bind(project_id)
    .bind(name)
    .fetch_one(&st.db)
    .await?;
    if taken.0 {
        return Err(ApiError::bad_body(json!({"name": "The state name is already taken"})));
    }
    let v: Value = sqlx::query_scalar(&format!(
        "INSERT INTO states AS s (name, color, \"group\", description, slug, project_id, workspace_id, created_by_id, updated_by_id,
                               sequence)
         VALUES ($1, $2, $3, COALESCE($4, ''), $5, $6, $7, $8, $8,
                 COALESCE((SELECT max(sequence) + 15000 FROM states WHERE project_id = $6), 65535))
         RETURNING {STATE_JSON}"
    ))
    .bind(name)
    .bind(color)
    .bind(group)
    .bind(s(&b, "description"))
    .bind(slugify(name))
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    Ok(Json(v))
}

async fn update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN])?;
    if let Some(g) = s(&b, "group")
        && !GROUPS.contains(&g)
    {
        return Err(ApiError::bad_body(
            json!({"group": [format!("\"{g}\" is not a valid choice.")]}),
        ));
    }
    let name = s(&b, "name").map(str::trim);
    if let Some(n) = name {
        let taken: (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM states WHERE project_id = $1 AND name = $2 AND id <> $3 AND deleted_at IS NULL)",
        )
        .bind(project_id)
        .bind(n)
        .bind(pk)
        .fetch_one(&st.db)
        .await?;
        if taken.0 {
            return Err(ApiError::bad_body(json!({"name": "The state name is already taken"})));
        }
    }
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE states AS s SET name = COALESCE($3, name), slug = COALESCE($4, slug), color = COALESCE($5, color),
                \"group\" = COALESCE($6, \"group\"), description = COALESCE($7, description),
                sequence = COALESCE($8, sequence), updated_by_id = $9, updated_at = now()
             WHERE s.id = $1 AND s.project_id = $2 AND NOT s.is_triage AND s.deleted_at IS NULL
             RETURNING {STATE_JSON}"
        ))
        .bind(pk)
        .bind(project_id)
        .bind(name)
        .bind(name.map(slugify))
        .bind(s(&b, "color"))
        .bind(s(&b, "group"))
        .bind(s(&b, "description"))
        .bind(b.get("sequence").and_then(Value::as_f64))
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn mark_default(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN])?;
    let r = sqlx::query("UPDATE states SET \"default\" = (id = $1) WHERE project_id = $2 AND deleted_at IS NULL
                         AND EXISTS (SELECT 1 FROM states WHERE id = $1 AND project_id = $2 AND NOT is_triage AND deleted_at IS NULL)")
        .bind(pk)
        .bind(project_id)
        .execute(&st.db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN])?;
    let row: Option<(bool,)> = sqlx::query_as(
        "SELECT \"default\" FROM states WHERE id = $1 AND project_id = $2 AND NOT is_triage AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .fetch_optional(&st.db)
    .await?;
    let (is_default,) = row.ok_or_else(ApiError::not_found)?;
    if is_default {
        return Err(ApiError::bad("Default state cannot be deleted"));
    }
    let used: (bool,) =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM issues WHERE state_id = $1 AND deleted_at IS NULL)")
            .bind(pk)
            .fetch_one(&st.db)
            .await?;
    if used.0 {
        return Err(ApiError::bad(
            "The state is not empty, only empty states can be deleted",
        ));
    }
    // Plane soft-deletes: set deleted_at instead of DELETE.
    sqlx::query("UPDATE states SET deleted_at = now() WHERE id = $1 AND project_id = $2")
        .bind(pk)
        .bind(project_id)
        .execute(&st.db)
        .await?;
    no_content()
}

async fn intake_state(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let v: Option<Value> = sqlx::query_scalar(&format!(
        "SELECT {STATE_JSON} FROM states s WHERE s.project_id = $1 AND s.is_triage AND s.deleted_at IS NULL LIMIT 1"
    ))
    .bind(project_id)
    .fetch_optional(&st.db)
    .await?;
    v.map(Json)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, json!({"error": "Triage state not found"})))
}

/// Default states for a new project (Plane's `ProjectViewSet.create`). Used by projects.rs.
pub async fn create_defaults(
    tx: &mut sqlx::PgConnection,
    workspace_id: Uuid,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(), ApiError> {
    let defaults = [
        ("Backlog", "#60646C", 15000.0, "backlog", true, false),
        ("Todo", "#60646C", 25000.0, "unstarted", false, false),
        ("In Progress", "#F59E0B", 35000.0, "started", false, false),
        ("Done", "#46A758", 45000.0, "completed", false, false),
        ("Cancelled", "#9AA4BC", 55000.0, "cancelled", false, false),
        ("Triage", "#4E5355", 65000.0, "triage", false, true),
    ];
    for (name, color, seq, group, default, triage) in defaults {
        sqlx::query(
            "INSERT INTO states (name, color, sequence, \"group\", \"default\", is_triage, slug, project_id, workspace_id,
                                 created_by_id, updated_by_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)",
        )
        .bind(name)
        .bind(color)
        .bind(seq)
        .bind(group)
        .bind(default)
        .bind(triage)
        .bind(slugify(name))
        .bind(project_id)
        .bind(workspace_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn slugify_matches_django() {
        assert_eq!(super::slugify("In Progress"), "in-progress");
        assert_eq!(super::slugify("  Done!! "), "done");
    }
}
