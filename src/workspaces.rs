//! Workspaces, members, invitations, favorites, workspace-level user data.
//! TODO (worker): port the remaining `/api/workspaces/{slug}/...` and `/api/users/me/workspaces/` routes.
use axum::Router;
use uuid::Uuid;

use crate::common::*;

pub fn routes() -> Router<AppState> {
    Router::new()
}

/// Insert a workspace and its owner as admin member. Shared by the create endpoint and `saturn seed`.
pub async fn create_workspace(
    tx: &mut sqlx::PgConnection,
    owner: Uuid,
    name: &str,
    slug: &str,
    organization_size: &str,
) -> Result<Uuid, ApiError> {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO workspaces (name, slug, owner_id, created_by_id, updated_by_id, organization_size)
         VALUES ($1, $2, $3, $3, $3, $4) RETURNING id",
    )
    .bind(name)
    .bind(slug)
    .bind(owner)
    .bind(organization_size)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO workspace_members (workspace_id, member_id, role, created_by_id) VALUES ($1, $2, 20, $2)")
        .bind(id)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    Ok(id)
}
