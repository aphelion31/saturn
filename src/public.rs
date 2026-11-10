//! Public boards API: Plane's /api/public/ routes for public project spaces.
//! Authenticated or anonymous reads guarded by deploy_boards anchor.
//! Mutations guarded by sign-in and deploy board feature flags.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    common::*,
    workitems::{self, ACTIVE, Arg, Base, Scope},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        // Anchor settings & metadata
        .route("/api/public/anchor/{anchor}/settings/", get(settings))
        .route("/api/public/anchor/{anchor}/meta/", get(meta))
        .route("/api/public/anchor/{anchor}/project-members/", get(project_members))
        .route("/api/public/anchor/{anchor}/members/", get(project_members))
        // Taxonomy endpoints
        .route("/api/public/anchor/{anchor}/states/", get(states))
        .route("/api/public/anchor/{anchor}/labels/", get(labels))
        .route("/api/public/anchor/{anchor}/cycles/", get(cycles))
        .route("/api/public/anchor/{anchor}/modules/", get(modules))
        // Issues list
        .route("/api/public/anchor/{anchor}/issues/", get(issues_list))
        // Workspace project anchor reverse-lookup
        .route(
            "/api/public/workspaces/{slug}/projects/{project_id}/anchor/",
            get(workspace_project_anchor),
        )
}

/// Anchor deploy board context.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct BoardCtx {
    pub id: Uuid,
    pub anchor: String,
    pub project_id: Uuid,
    pub workspace_id: Uuid,
    pub entity_identifier: Option<Uuid>,
    pub entity_name: Option<String>,
    pub is_comments_enabled: bool,
    pub is_reactions_enabled: bool,
    pub is_votes_enabled: bool,
    pub is_activity_enabled: bool,
    pub is_disabled: bool,
    pub intake_id: Option<Uuid>,
    pub view_props: Value,
    pub workspace_slug: String,
}

pub async fn require_anchor(db: &sqlx::PgPool, anchor: &str) -> Result<BoardCtx, ApiError> {
    sqlx::query_as::<_, BoardCtx>(
        "SELECT b.id, b.anchor, b.project_id, b.workspace_id, b.entity_identifier, b.entity_name,
                b.is_comments_enabled, b.is_reactions_enabled, b.is_votes_enabled,
                b.is_activity_enabled, b.is_disabled, b.intake_id, b.view_props,
                w.slug AS workspace_slug
         FROM deploy_boards b
         JOIN workspaces w ON w.id = b.workspace_id
         JOIN projects p ON p.id = b.project_id
         WHERE b.anchor = $1
           AND b.deleted_at IS NULL
           AND NOT b.is_disabled
           AND w.deleted_at IS NULL
           AND p.deleted_at IS NULL
           AND p.archived_at IS NULL
         LIMIT 1",
    )
    .bind(anchor)
    .fetch_optional(db)
    .await?
    .ok_or_else(ApiError::not_found)
}

/// GET /api/public/anchor/{anchor}/settings/
async fn settings(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let row: Option<(Value,)> = sqlx::query_as(
        "SELECT jsonb_build_object(
            'id', b.id,
            'anchor', b.anchor,
            'project', b.project_id,
            'workspace', b.workspace_id,
            'entity_identifier', b.entity_identifier,
            'entity_name', b.entity_name,
            'is_comments_enabled', b.is_comments_enabled,
            'is_reactions_enabled', b.is_reactions_enabled,
            'is_votes_enabled', b.is_votes_enabled,
            'is_activity_enabled', b.is_activity_enabled,
            'is_disabled', b.is_disabled,
            'intake', b.intake_id,
            'view_props', b.view_props,
            'created_at', b.created_at,
            'updated_at', b.updated_at,
            'project_details', jsonb_build_object(
                'id', p.id,
                'identifier', p.identifier,
                'name', p.name,
                'cover_image', p.cover_image,
                'icon_prop', p.icon_prop,
                'emoji', p.emoji,
                'description', p.description
            ),
            'workspace_detail', jsonb_build_object(
                'id', w.id,
                'name', w.name,
                'slug', w.slug,
                'logo', w.logo
            )
        )
        FROM deploy_boards b
        JOIN workspaces w ON w.id = b.workspace_id
        JOIN projects p ON p.id = b.project_id
        WHERE b.anchor = $1
          AND b.deleted_at IS NULL
          AND NOT b.is_disabled
          AND w.deleted_at IS NULL
          AND p.deleted_at IS NULL
          AND p.archived_at IS NULL
        LIMIT 1",
    )
    .bind(&anchor)
    .fetch_optional(&st.db)
    .await?;

    let (val,) = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(val))
}

/// GET /api/public/anchor/{anchor}/meta/
async fn meta(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let row: Option<(Value,)> = sqlx::query_as(
        "SELECT jsonb_build_object(
            'id', p.id,
            'identifier', p.identifier,
            'name', p.name,
            'cover_image', p.cover_image,
            'icon_prop', p.icon_prop,
            'emoji', p.emoji,
            'description', p.description
        )
        FROM projects p
        WHERE p.id = $1 AND p.workspace_id = $2
          AND p.deleted_at IS NULL AND p.archived_at IS NULL",
    )
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_optional(&st.db)
    .await?;

    let (val,) = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(val))
}

/// GET /api/public/anchor/{anchor}/project-members/ and /members/
async fn project_members(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let members: Vec<Value> = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', pm.id,
            'member', pm.member_id,
            'member__display_name', u.display_name,
            'member__avatar', u.avatar
        ) ORDER BY u.display_name), '[]'::jsonb)
        FROM project_members pm
        JOIN users u ON u.id = pm.member_id
        WHERE pm.project_id = $1 AND pm.workspace_id = $2
          AND pm.is_active = true AND pm.deleted_at IS NULL AND u.is_active = true",
    )
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(members.into_iter().next().unwrap_or_else(|| json!([]))))
}

/// GET /api/public/anchor/{anchor}/states/
async fn states(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', s.id,
            'name', s.name,
            'group', s.group,
            'color', s.color,
            'sequence', s.sequence
        ) ORDER BY s.sequence), '[]'::jsonb)
        FROM states s
        WHERE s.project_id = $1 AND s.workspace_id = $2
          AND s.deleted_at IS NULL AND s.group != 'triage'",
    )
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(rows.into_iter().next().unwrap_or_else(|| json!([]))))
}

/// GET /api/public/anchor/{anchor}/labels/
async fn labels(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', l.id,
            'name', l.name,
            'color', l.color,
            'parent', l.parent_id
        ) ORDER BY l.name), '[]'::jsonb)
        FROM labels l
        WHERE l.project_id = $1 AND l.workspace_id = $2
          AND l.deleted_at IS NULL",
    )
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(rows.into_iter().next().unwrap_or_else(|| json!([]))))
}

/// GET /api/public/anchor/{anchor}/cycles/
async fn cycles(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', c.id,
            'name', c.name
        ) ORDER BY c.created_at DESC), '[]'::jsonb)
        FROM cycles c
        WHERE c.project_id = $1 AND c.workspace_id = $2
          AND c.deleted_at IS NULL AND c.archived_at IS NULL",
    )
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(rows.into_iter().next().unwrap_or_else(|| json!([]))))
}

/// GET /api/public/anchor/{anchor}/modules/
async fn modules(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', m.id,
            'name', m.name
        ) ORDER BY m.created_at DESC), '[]'::jsonb)
        FROM modules m
        WHERE m.project_id = $1 AND m.workspace_id = $2
          AND m.deleted_at IS NULL AND m.archived_at IS NULL",
    )
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(rows.into_iter().next().unwrap_or_else(|| json!([]))))
}

/// GET /api/public/anchor/{anchor}/issues/
async fn issues_list(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let base = Base::new(
        format!("i.project_id = $1 AND i.workspace_id = $2 AND {ACTIVE}"),
        vec![Arg::Uuid(b.project_id), Arg::Uuid(b.workspace_id)],
    );
    workitems::list(&st.db, base, &q, Scope::Project(b.project_id)).await
}

/// GET /api/public/workspaces/{slug}/projects/{project_id}/anchor/
async fn workspace_project_anchor(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let row: Option<(Value,)> = sqlx::query_as(
        "SELECT jsonb_build_object(
            'id', b.id,
            'anchor', b.anchor,
            'project', b.project_id,
            'workspace', b.workspace_id,
            'entity_identifier', b.entity_identifier,
            'entity_name', b.entity_name,
            'is_comments_enabled', b.is_comments_enabled,
            'is_reactions_enabled', b.is_reactions_enabled,
            'is_votes_enabled', b.is_votes_enabled,
            'is_activity_enabled', b.is_activity_enabled,
            'is_disabled', b.is_disabled,
            'intake', b.intake_id,
            'view_props', b.view_props,
            'created_at', b.created_at,
            'updated_at', b.updated_at
        )
        FROM deploy_boards b
        JOIN workspaces w ON w.id = b.workspace_id
        WHERE w.slug = $1 AND b.project_id = $2
          AND b.entity_identifier = $2 AND b.entity_name = 'project'
          AND b.deleted_at IS NULL
          AND NOT b.is_disabled
        LIMIT 1",
    )
    .bind(&slug)
    .bind(project_id)
    .fetch_optional(&st.db)
    .await?;

    let (val,) = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(val))
}
