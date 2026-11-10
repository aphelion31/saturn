//! Projects, project members, invites, identifiers, favorites, deploy boards.
//! TODO (worker): port the `/api/workspaces/{slug}/projects/...` routes.
use axum::Router;
use serde_json::Value;
use uuid::Uuid;

use crate::common::*;

pub fn routes() -> Router<AppState> {
    Router::new()
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
