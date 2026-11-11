use axum::{
    Router,
    extract::{Path, State},
    routing::get,
};

use crate::common::*;

const USER_JSON: &str = "jsonb_build_object('id', u.id, 'first_name', u.first_name, 'last_name', u.last_name,
    'email', u.email, 'avatar', u.avatar, 'avatar_url',
    CASE WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id || '/' ELSE NULLIF(u.avatar, '') END,
    'display_name', u.display_name)";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/users/me/", get(me))
        .route("/api/v1/workspaces/{slug}/members/", get(workspace_members))
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
