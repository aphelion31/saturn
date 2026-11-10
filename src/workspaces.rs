//! Workspaces, members, invitations, favorites, workspace-level user data.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::*;
use crate::instance::{RESTRICTED_SLUGS, slug_ok};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/users/me/workspaces/", get(user_workspaces))
        .route("/api/workspaces/", get(user_workspaces).post(create))
        .route("/api/workspace-slug-check/", get(slug_check))
        .route("/api/workspaces/{slug}/", get(retrieve).patch(update).delete(destroy))
}

/// WorkSpaceSerializer fields, with `total_members` (active, non-bot) and `logo_url`.
/// `role` is added by the caller when the query knows it.
const WS_JSON: &str = "to_jsonb(w) - 'owner_id' - 'created_by_id' - 'updated_by_id' - 'logo_asset_id'
    || jsonb_build_object('owner', w.owner_id, 'created_by', w.created_by_id, 'updated_by', w.updated_by_id,
       'logo_asset', w.logo_asset_id, 'logo_url', NULLIF(w.logo, ''),
       'total_members', (SELECT count(*) FROM workspace_members m JOIN users u ON u.id = m.member_id
                         WHERE m.workspace_id = w.id AND m.is_active AND m.deleted_at IS NULL AND NOT u.is_bot))";

/// Plane's `WorkspaceUserSerializer` path, the caller's own workspaces with role and member count.
async fn user_workspaces(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {WS_JSON} || jsonb_build_object('role', m.role) FROM workspaces w
             JOIN workspace_members m ON m.workspace_id = w.id AND m.member_id = $1 AND m.is_active AND m.deleted_at IS NULL
             WHERE w.deleted_at IS NULL ORDER BY w.name"
        ))
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn slug_check(
    State(st): State<AppState>,
    _user: CurrentUser,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let slug = q.get("slug").map(String::as_str).unwrap_or_default();
    if slug.is_empty() {
        return Err(ApiError::bad("Workspace Slug is required"));
    }
    let taken: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM workspaces WHERE slug = $1)")
        .bind(slug)
        .fetch_one(&st.db)
        .await?;
    Ok(Json(json!({"status": !taken.0 && !RESTRICTED_SLUGS.contains(&slug)})))
}

async fn retrieve(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {WS_JSON} || jsonb_build_object('role', $2::smallint) FROM workspaces w
             WHERE w.id = $1 AND w.deleted_at IS NULL"
        ))
        .bind(ws.workspace_id)
        .bind(ws.role),
        &st.db,
    )
    .await
}

async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let disabled: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT value FROM instance_configurations WHERE key = 'DISABLE_WORKSPACE_CREATION' AND deleted_at IS NULL",
    )
    .fetch_optional(&st.db)
    .await?;
    if disabled.and_then(|(v,)| v).is_some_and(|v| v == "1") {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            json!({"error": "Workspace creation is not allowed"}),
        ));
    }
    let name = s(&b, "name").unwrap_or("");
    let slug = s(&b, "slug").unwrap_or("");
    if name.is_empty() || slug.is_empty() {
        return Err(ApiError::bad("Both name and slug are required"));
    }
    if name.chars().count() > 80 || slug.len() > 48 {
        return Err(ApiError::bad("The maximum length for name is 80 and for slug is 48"));
    }
    if !slug_ok(slug) {
        return Err(ApiError::bad_body(json!({"slug": ["Slug is not valid"]})));
    }
    let mut tx = st.db.begin().await?;
    let id = create_workspace(&mut tx, user.id, name, slug, s(&b, "organization_size").unwrap_or(""))
        .await
        .map_err(|e| {
            if e.0 == StatusCode::CONFLICT {
                ApiError(
                    StatusCode::CONFLICT,
                    json!({"slug": "The workspace with the slug already exists"}),
                )
            } else {
                e
            }
        })?;
    tx.commit().await?;
    let v: Value = sqlx::query_scalar(&format!(
        "SELECT {WS_JSON} || jsonb_build_object('role', 20::smallint) FROM workspaces w WHERE w.id = $1"
    ))
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    created(v)
}

/// Plane: PATCH needs admin. Only writable workspace columns; slug and owner are not editable here.
async fn update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    if let Some(n) = s(&b, "name")
        && (n.trim().is_empty() || n.chars().count() > 80)
    {
        return Err(ApiError::bad_body(json!({"name": ["Name is not valid"]})));
    }
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE workspaces w SET name = COALESCE($2, name), logo = COALESCE($3, logo),
                organization_size = COALESCE($4, organization_size), timezone = COALESCE($5, timezone),
                background_color = COALESCE($6, background_color), updated_at = now(), updated_by_id = $7
             WHERE w.id = $1 AND w.deleted_at IS NULL
             RETURNING {WS_JSON} || jsonb_build_object('role', $8::smallint)"
        ))
        .bind(ws.workspace_id)
        .bind(s(&b, "name").map(str::trim))
        .bind(s(&b, "logo"))
        .bind(s(&b, "organization_size"))
        .bind(s(&b, "timezone"))
        .bind(s(&b, "background_color"))
        .bind(user.id)
        .bind(ws.role),
        &st.db,
    )
    .await
}

/// Plane: DELETE needs admin. Soft delete; clear `last_workspace_id` pointers like Plane does.
async fn destroy(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let mut tx = st.db.begin().await?;
    sqlx::query("UPDATE profiles SET last_workspace_id = NULL WHERE last_workspace_id = $1")
        .bind(ws.workspace_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE workspaces SET deleted_at = now(), updated_at = now(), updated_by_id = $2 WHERE id = $1")
        .bind(ws.workspace_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    no_content()
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
