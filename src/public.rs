//! Public boards API: Plane's /api/public/ routes for public project spaces.
//! Authenticated or anonymous reads guarded by deploy_boards anchor.
//! Mutations guarded by sign-in and deploy board feature flags.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get},
};
use serde::Deserialize;
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
        // Issue detail
        .route("/api/public/anchor/{anchor}/issues/{issue_id}/", get(issue_detail))
        // Comments CRUD
        .route(
            "/api/public/anchor/{anchor}/issues/{issue_id}/comments/",
            get(comments_list).post(comment_create),
        )
        .route(
            "/api/public/anchor/{anchor}/issues/{issue_id}/comments/{pk}/",
            get(comment_retrieve).patch(comment_update).delete(comment_destroy),
        )
        // Issue reactions
        .route(
            "/api/public/anchor/{anchor}/issues/{issue_id}/reactions/",
            get(issue_reactions_list).post(issue_reaction_create),
        )
        .route(
            "/api/public/anchor/{anchor}/issues/{issue_id}/reactions/{reaction_code}/",
            delete(issue_reaction_destroy),
        )
        // Comment reactions
        .route(
            "/api/public/anchor/{anchor}/comments/{comment_id}/reactions/",
            get(comment_reactions_list).post(comment_reaction_create),
        )
        .route(
            "/api/public/anchor/{anchor}/comments/{comment_id}/reactions/{reaction_code}/",
            delete(comment_reaction_destroy),
        )
        // Issue votes
        .route(
            "/api/public/anchor/{anchor}/issues/{issue_id}/votes/",
            get(issue_votes_list).post(issue_vote_create).delete(issue_vote_destroy),
        )
        // Intake issues list & create
        .route(
            "/api/public/anchor/{anchor}/intakes/{intake_id}/intake-issues/",
            get(intake_issues_list).post(intake_issue_create),
        )
        .route(
            "/api/public/anchor/{anchor}/intakes/{intake_id}/inbox-issues/",
            get(intake_issues_list).post(intake_issue_create),
        )
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

/// Check that an issue exists in board scope (not deleted, archived, or draft; not triage state).
async fn issue_in_board_scope(db: &sqlx::PgPool, b: &BoardCtx, issue_id: Uuid) -> Result<bool, ApiError> {
    let exists: (bool,) = sqlx::query_as(&format!(
        "SELECT EXISTS (
            SELECT 1 FROM issues i
            WHERE i.id = $1 AND i.project_id = $2 AND i.workspace_id = $3
              AND {ACTIVE}
        )"
    ))
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(db)
    .await?;
    Ok(exists.0)
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
    let members: Value = sqlx::query_scalar(
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

    Ok(Json(members))
}

/// GET /api/public/anchor/{anchor}/states/
async fn states(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let rows: Value = sqlx::query_scalar(
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

    Ok(Json(rows))
}

/// GET /api/public/anchor/{anchor}/labels/
async fn labels(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let rows: Value = sqlx::query_scalar(
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

    Ok(Json(rows))
}

/// GET /api/public/anchor/{anchor}/cycles/
async fn cycles(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let rows: Value = sqlx::query_scalar(
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

    Ok(Json(rows))
}

/// GET /api/public/anchor/{anchor}/modules/
async fn modules(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path(anchor): Path<String>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let rows: Value = sqlx::query_scalar(
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

    Ok(Json(rows))
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

/// GET /api/public/anchor/{anchor}/issues/{issue_id}/
async fn issue_detail(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path((anchor, issue_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;

    let row: Option<(Value,)> = sqlx::query_as(&format!(
        "SELECT jsonb_build_object(
            'id', i.id,
            'name', i.name,
            'state_id', i.state_id,
            'sort_order', i.sort_order,
            'description_json', i.description_json,
            'description_html', i.description_html,
            'description_stripped', i.description_stripped,
            'description_binary', i.description_binary,
            'estimate_point', i.estimate_point_id,
            'priority', i.priority,
            'start_date', i.start_date,
            'target_date', i.target_date,
            'sequence_id', i.sequence_id,
            'project_id', i.project_id,
            'parent_id', i.parent_id,
            'created_by', i.created_by_id,
            'state__group', (SELECT s.group FROM states s WHERE s.id = i.state_id),
            'cycle_id', (SELECT ci.cycle_id FROM cycle_issues ci WHERE ci.issue_id = i.id AND ci.deleted_at IS NULL LIMIT 1),
            'assignee_ids', COALESCE((SELECT jsonb_agg(a.assignee_id) FROM issue_assignees a JOIN users u ON u.id = a.assignee_id
                                      WHERE a.issue_id = i.id AND a.deleted_at IS NULL AND u.is_active), '[]'::jsonb),
            'label_ids', COALESCE((SELECT jsonb_agg(l.label_id) FROM issue_labels l WHERE l.issue_id = i.id AND l.deleted_at IS NULL),
                                  '[]'::jsonb),
            'module_ids', COALESCE((SELECT jsonb_agg(m.module_id) FROM module_issues m JOIN modules mod ON mod.id = m.module_id
                                    WHERE m.issue_id = i.id AND m.deleted_at IS NULL AND mod.archived_at IS NULL),
                                   '[]'::jsonb),
            'vote_items', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'vote', v.vote,
                    'actor_details', jsonb_build_object(
                        'id', u.id,
                        'first_name', u.first_name,
                        'last_name', u.last_name,
                        'avatar', u.avatar,
                        'display_name', u.display_name
                    )
                ))
                FROM issue_votes v
                JOIN users u ON u.id = v.actor_id
                WHERE v.issue_id = i.id AND v.deleted_at IS NULL
            ), '[]'::jsonb),
            'reaction_items', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'reaction', r.reaction,
                    'actor_details', jsonb_build_object(
                        'id', u.id,
                        'first_name', u.first_name,
                        'last_name', u.last_name,
                        'avatar', u.avatar,
                        'display_name', u.display_name
                    )
                ))
                FROM issue_reactions r
                JOIN users u ON u.id = r.actor_id
                WHERE r.issue_id = i.id AND r.deleted_at IS NULL
            ), '[]'::jsonb)
        )
        FROM issues i
        WHERE i.id = $1 AND i.project_id = $2 AND i.workspace_id = $3
          AND {ACTIVE}
        LIMIT 1"
    ))
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_optional(&st.db)
    .await?;

    let (val,) = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(val))
}

/// GET /api/public/anchor/{anchor}/issues/{issue_id}/comments/
async fn comments_list(
    State(st): State<AppState>,
    user: MaybeUser,
    Path((anchor, issue_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_comments_enabled {
        return Ok(Json(json!([])));
    }
    let current_user_id = user.0.map(|u| u.id).unwrap_or(Uuid::nil());

    let rows: Value = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', c.id,
            'comment_html', c.comment_html,
            'comment_json', c.comment_json,
            'comment_stripped', c.comment_stripped,
            'attachments', c.attachments,
            'access', c.access,
            'created_at', c.created_at,
            'updated_at', c.updated_at,
            'created_by', c.created_by_id,
            'updated_by', c.updated_by_id,
            'actor', c.actor_id,
            'issue', c.issue_id,
            'project', c.project_id,
            'workspace', c.workspace_id,
            'actor_detail', jsonb_build_object(
                'id', u.id,
                'first_name', u.first_name,
                'last_name', u.last_name,
                'avatar', u.avatar,
                'display_name', u.display_name
            ),
            'is_member', EXISTS(
                SELECT 1 FROM project_members pm
                WHERE pm.project_id = c.project_id AND pm.workspace_id = c.workspace_id
                  AND pm.member_id = $4 AND pm.is_active = true
            ),
            'comment_reactions', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'id', cr.id,
                    'actor', cr.actor_id,
                    'comment', cr.comment_id,
                    'reaction', cr.reaction,
                    'display_name', ru.display_name,
                    'workspace', cr.workspace_id,
                    'project', cr.project_id,
                    'created_at', cr.created_at,
                    'updated_at', cr.updated_at
                ) ORDER BY cr.created_at)
                FROM comment_reactions cr
                JOIN users ru ON ru.id = cr.actor_id
                WHERE cr.comment_id = c.id AND cr.deleted_at IS NULL
            ), '[]'::jsonb)
        ) ORDER BY c.created_at), '[]'::jsonb)
        FROM issue_comments c
        LEFT JOIN users u ON u.id = c.actor_id
        WHERE c.issue_id = $1 AND c.project_id = $2 AND c.workspace_id = $3
          AND c.access = 'EXTERNAL' AND c.deleted_at IS NULL",
    )
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .bind(current_user_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(rows))
}

#[derive(Deserialize)]
struct CommentIn {
    comment_html: Option<String>,
    comment_json: Option<Value>,
}

/// POST /api/public/anchor/{anchor}/issues/{issue_id}/comments/
async fn comment_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((anchor, issue_id)): Path<(String, Uuid)>,
    Json(body): Json<CommentIn>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_comments_enabled {
        return Err(ApiError::bad("Comments are not enabled for this project"));
    }
    if !issue_in_board_scope(&st.db, &b, issue_id).await? {
        return Err(ApiError::not_found());
    }

    let id = Uuid::new_v4();
    let raw_html = body.comment_html.unwrap_or_else(|| "<p></p>".to_string());
    let comment_html = sanitize_html(&raw_html)?;
    let comment_json = body.comment_json.unwrap_or_else(|| json!({}));
    let comment_stripped = strip_tags(&comment_html);

    let row: (Value,) = sqlx::query_as(
        "WITH ins AS (
            INSERT INTO issue_comments (
                id, created_at, updated_at, comment_stripped, attachments,
                created_by_id, updated_by_id, issue_id, project_id, workspace_id,
                actor_id, comment_html, comment_json, access
            )
            VALUES ($1, now(), now(), $2, '{}', $3, $3, $4, $5, $6, $3, $7, $8, 'EXTERNAL')
            RETURNING *
        )
        SELECT jsonb_build_object(
            'id', ins.id,
            'comment_html', ins.comment_html,
            'comment_json', ins.comment_json,
            'comment_stripped', ins.comment_stripped,
            'attachments', ins.attachments,
            'access', ins.access,
            'created_at', ins.created_at,
            'updated_at', ins.updated_at,
            'created_by', ins.created_by_id,
            'updated_by', ins.updated_by_id,
            'actor', ins.actor_id,
            'issue', ins.issue_id,
            'project', ins.project_id,
            'workspace', ins.workspace_id,
            'actor_detail', jsonb_build_object(
                'id', u.id,
                'first_name', u.first_name,
                'last_name', u.last_name,
                'avatar', u.avatar,
                'display_name', u.display_name
            ),
            'comment_reactions', '[]'::jsonb
        )
        FROM ins
        JOIN users u ON u.id = ins.actor_id",
    )
    .bind(id)
    .bind(&comment_stripped)
    .bind(user.id)
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .bind(&comment_html)
    .bind(&comment_json)
    .fetch_one(&st.db)
    .await?;

    Ok((StatusCode::CREATED, Json(row.0)))
}

/// GET /api/public/anchor/{anchor}/issues/{issue_id}/comments/{pk}/
async fn comment_retrieve(
    State(st): State<AppState>,
    user: MaybeUser,
    Path((anchor, issue_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    let current_user_id = user.0.map(|u| u.id).unwrap_or(Uuid::nil());

    let row: Option<(Value,)> = sqlx::query_as(
        "SELECT jsonb_build_object(
            'id', c.id,
            'comment_html', c.comment_html,
            'comment_json', c.comment_json,
            'comment_stripped', c.comment_stripped,
            'attachments', c.attachments,
            'access', c.access,
            'created_at', c.created_at,
            'updated_at', c.updated_at,
            'created_by', c.created_by_id,
            'updated_by', c.updated_by_id,
            'actor', c.actor_id,
            'issue', c.issue_id,
            'project', c.project_id,
            'workspace', c.workspace_id,
            'actor_detail', jsonb_build_object(
                'id', u.id,
                'first_name', u.first_name,
                'last_name', u.last_name,
                'avatar', u.avatar,
                'display_name', u.display_name
            ),
            'is_member', EXISTS(
                SELECT 1 FROM project_members pm
                WHERE pm.project_id = c.project_id AND pm.workspace_id = c.workspace_id
                  AND pm.member_id = $4 AND pm.is_active = true
            ),
            'comment_reactions', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'id', cr.id,
                    'actor', cr.actor_id,
                    'comment', cr.comment_id,
                    'reaction', cr.reaction,
                    'display_name', ru.display_name,
                    'workspace', cr.workspace_id,
                    'project', cr.project_id,
                    'created_at', cr.created_at,
                    'updated_at', cr.updated_at
                ) ORDER BY cr.created_at)
                FROM comment_reactions cr
                JOIN users ru ON ru.id = cr.actor_id
                WHERE cr.comment_id = c.id AND cr.deleted_at IS NULL
            ), '[]'::jsonb)
        )
        FROM issue_comments c
        LEFT JOIN users u ON u.id = c.actor_id
        WHERE c.id = $1 AND c.issue_id = $2 AND c.project_id = $3
          AND c.access = 'EXTERNAL' AND c.deleted_at IS NULL
        LIMIT 1",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(b.project_id)
    .bind(current_user_id)
    .fetch_optional(&st.db)
    .await?;

    let (val,) = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(val))
}

/// PATCH /api/public/anchor/{anchor}/issues/{issue_id}/comments/{pk}/
async fn comment_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((anchor, issue_id, pk)): Path<(String, Uuid, Uuid)>,
    Json(body): Json<CommentIn>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_comments_enabled {
        return Err(ApiError::bad("Comments are not enabled for this project"));
    }

    let existing: Option<(Option<String>, Option<Value>)> = sqlx::query_as(
        "SELECT comment_html, comment_json FROM issue_comments
         WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $4
           AND access = 'EXTERNAL' AND actor_id = $5 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .bind(user.id)
    .fetch_optional(&st.db)
    .await?;

    let (old_html, old_json) = existing.ok_or_else(ApiError::not_found)?;
    let raw_html = body.comment_html.or(old_html).unwrap_or_else(|| "<p></p>".to_string());
    let new_html = sanitize_html(&raw_html)?;
    let new_json = body.comment_json.or(old_json).unwrap_or_else(|| json!({}));
    let new_stripped = strip_tags(&new_html);

    let row: (Value,) = sqlx::query_as(
        "WITH upd AS (
            UPDATE issue_comments
            SET comment_html = $1, comment_json = $2, comment_stripped = $3,
                updated_at = now(), updated_by_id = $4
            WHERE id = $5 AND issue_id = $6 AND project_id = $7 AND workspace_id = $8
              AND access = 'EXTERNAL' AND actor_id = $4 AND deleted_at IS NULL
            RETURNING *
        )
        SELECT jsonb_build_object(
            'id', upd.id,
            'comment_html', upd.comment_html,
            'comment_json', upd.comment_json,
            'comment_stripped', upd.comment_stripped,
            'attachments', upd.attachments,
            'access', upd.access,
            'created_at', upd.created_at,
            'updated_at', upd.updated_at,
            'created_by', upd.created_by_id,
            'updated_by', upd.updated_by_id,
            'actor', upd.actor_id,
            'issue', upd.issue_id,
            'project', upd.project_id,
            'workspace', upd.workspace_id,
            'actor_detail', jsonb_build_object(
                'id', u.id,
                'first_name', u.first_name,
                'last_name', u.last_name,
                'avatar', u.avatar,
                'display_name', u.display_name
            )
        )
        FROM upd
        JOIN users u ON u.id = upd.actor_id",
    )
    .bind(&new_html)
    .bind(&new_json)
    .bind(&new_stripped)
    .bind(user.id)
    .bind(pk)
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(row.0))
}

/// DELETE /api/public/anchor/{anchor}/issues/{issue_id}/comments/{pk}/
async fn comment_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((anchor, issue_id, pk)): Path<(String, Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_comments_enabled {
        return Err(ApiError::bad("Comments are not enabled for this project"));
    }

    let res = sqlx::query(
        "UPDATE issue_comments
         SET deleted_at = now()
         WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $4
           AND access = 'EXTERNAL' AND actor_id = $5 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .bind(user.id)
    .execute(&st.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/public/anchor/{anchor}/issues/{issue_id}/reactions/
async fn issue_reactions_list(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path((anchor, issue_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_reactions_enabled {
        return Ok(Json(json!([])));
    }

    let rows: Value = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', r.id,
            'actor', r.actor_id,
            'issue', r.issue_id,
            'reaction', r.reaction,
            'project', r.project_id,
            'workspace', r.workspace_id,
            'created_at', r.created_at,
            'updated_at', r.updated_at,
            'actor_detail', jsonb_build_object(
                'id', u.id,
                'first_name', u.first_name,
                'last_name', u.last_name,
                'avatar', u.avatar,
                'display_name', u.display_name
            )
        ) ORDER BY r.created_at DESC), '[]'::jsonb)
        FROM issue_reactions r
        JOIN users u ON u.id = r.actor_id
        WHERE r.issue_id = $1 AND r.project_id = $2 AND r.workspace_id = $3
          AND r.deleted_at IS NULL",
    )
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(rows))
}

#[derive(Deserialize)]
struct ReactionIn {
    reaction: String,
}

/// POST /api/public/anchor/{anchor}/issues/{issue_id}/reactions/
async fn issue_reaction_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((anchor, issue_id)): Path<(String, Uuid)>,
    Json(body): Json<ReactionIn>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_reactions_enabled {
        return Err(ApiError::bad("Reactions are not enabled for this project board"));
    }
    if !issue_in_board_scope(&st.db, &b, issue_id).await? {
        return Err(ApiError::not_found());
    }

    let id = Uuid::new_v4();
    let row: (Value,) = sqlx::query_as(
        "WITH ins AS (
            INSERT INTO issue_reactions (
                id, created_at, updated_at, reaction, actor_id, created_by_id,
                issue_id, project_id, updated_by_id, workspace_id
            )
            VALUES ($1, now(), now(), $2, $3, $3, $4, $5, $3, $6)
            RETURNING *
        )
        SELECT jsonb_build_object(
            'id', ins.id,
            'actor', ins.actor_id,
            'issue', ins.issue_id,
            'reaction', ins.reaction,
            'project', ins.project_id,
            'workspace', ins.workspace_id,
            'created_at', ins.created_at,
            'updated_at', ins.updated_at,
            'actor_detail', jsonb_build_object(
                'id', u.id,
                'first_name', u.first_name,
                'last_name', u.last_name,
                'avatar', u.avatar,
                'display_name', u.display_name
            )
        )
        FROM ins
        JOIN users u ON u.id = ins.actor_id",
    )
    .bind(id)
    .bind(&body.reaction)
    .bind(user.id)
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok((StatusCode::CREATED, Json(row.0)))
}

/// DELETE /api/public/anchor/{anchor}/issues/{issue_id}/reactions/{reaction_code}/
async fn issue_reaction_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((anchor, issue_id, reaction_code)): Path<(String, Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_reactions_enabled {
        return Err(ApiError::bad("Reactions are not enabled for this project board"));
    }

    let res = sqlx::query(
        "UPDATE issue_reactions
         SET deleted_at = now()
         WHERE issue_id = $1 AND reaction = $2 AND actor_id = $3
           AND project_id = $4 AND workspace_id = $5 AND deleted_at IS NULL",
    )
    .bind(issue_id)
    .bind(&reaction_code)
    .bind(user.id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .execute(&st.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/public/anchor/{anchor}/comments/{comment_id}/reactions/
async fn comment_reactions_list(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path((anchor, comment_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_reactions_enabled {
        return Ok(Json(json!([])));
    }

    let rows: Value = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', cr.id,
            'actor', cr.actor_id,
            'comment', cr.comment_id,
            'reaction', cr.reaction,
            'display_name', u.display_name,
            'project', cr.project_id,
            'workspace', cr.workspace_id,
            'created_at', cr.created_at,
            'updated_at', cr.updated_at
        ) ORDER BY cr.created_at DESC), '[]'::jsonb)
        FROM comment_reactions cr
        JOIN users u ON u.id = cr.actor_id
        JOIN issue_comments c ON c.id = cr.comment_id
        WHERE cr.comment_id = $1 AND cr.project_id = $2 AND cr.workspace_id = $3
          AND c.access = 'EXTERNAL' AND cr.deleted_at IS NULL",
    )
    .bind(comment_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(rows))
}

/// POST /api/public/anchor/{anchor}/comments/{comment_id}/reactions/
async fn comment_reaction_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((anchor, comment_id)): Path<(String, Uuid)>,
    Json(body): Json<ReactionIn>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_reactions_enabled {
        return Err(ApiError::bad("Reactions are not enabled for this board"));
    }

    // Verify comment is external and bound to board project
    let comment_ok: (bool,) = sqlx::query_as(
        "SELECT EXISTS (
            SELECT 1 FROM issue_comments
            WHERE id = $1 AND project_id = $2 AND workspace_id = $3
              AND access = 'EXTERNAL' AND deleted_at IS NULL
        )",
    )
    .bind(comment_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    if !comment_ok.0 {
        return Err(ApiError::not_found());
    }

    let id = Uuid::new_v4();
    let row: (Value,) = sqlx::query_as(
        "WITH ins AS (
            INSERT INTO comment_reactions (
                id, created_at, updated_at, reaction, actor_id, created_by_id,
                comment_id, project_id, updated_by_id, workspace_id
            )
            VALUES ($1, now(), now(), $2, $3, $3, $4, $5, $3, $6)
            RETURNING *
        )
        SELECT jsonb_build_object(
            'id', ins.id,
            'actor', ins.actor_id,
            'comment', ins.comment_id,
            'reaction', ins.reaction,
            'display_name', u.display_name,
            'workspace', ins.workspace_id,
            'project', ins.project_id,
            'created_at', ins.created_at,
            'updated_at', ins.updated_at
        )
        FROM ins
        JOIN users u ON u.id = ins.actor_id",
    )
    .bind(id)
    .bind(&body.reaction)
    .bind(user.id)
    .bind(comment_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok((StatusCode::CREATED, Json(row.0)))
}

/// DELETE /api/public/anchor/{anchor}/comments/{comment_id}/reactions/{reaction_code}/
async fn comment_reaction_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((anchor, comment_id, reaction_code)): Path<(String, Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_reactions_enabled {
        return Err(ApiError::bad("Reactions are not enabled for this board"));
    }

    let res = sqlx::query(
        "UPDATE comment_reactions cr
         SET deleted_at = now()
         FROM issue_comments c
         WHERE cr.comment_id = c.id AND cr.comment_id = $1 AND cr.reaction = $2 AND cr.actor_id = $3
           AND cr.project_id = $4 AND cr.workspace_id = $5 AND c.access = 'EXTERNAL'
           AND cr.deleted_at IS NULL",
    )
    .bind(comment_id)
    .bind(&reaction_code)
    .bind(user.id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .execute(&st.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/public/anchor/{anchor}/issues/{issue_id}/votes/
async fn issue_votes_list(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path((anchor, issue_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_votes_enabled {
        return Ok(Json(json!([])));
    }

    let rows: Value = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'issue', v.issue_id,
            'vote', v.vote,
            'workspace', v.workspace_id,
            'project', v.project_id,
            'actor', v.actor_id,
            'actor_detail', jsonb_build_object(
                'id', u.id,
                'first_name', u.first_name,
                'last_name', u.last_name,
                'avatar', u.avatar,
                'display_name', u.display_name
            )
        ) ORDER BY v.created_at DESC), '[]'::jsonb)
        FROM issue_votes v
        JOIN users u ON u.id = v.actor_id
        WHERE v.issue_id = $1 AND v.project_id = $2 AND v.workspace_id = $3
          AND v.deleted_at IS NULL",
    )
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(rows))
}

#[derive(Deserialize)]
struct VoteIn {
    vote: Option<i32>,
}

/// POST /api/public/anchor/{anchor}/issues/{issue_id}/votes/
async fn issue_vote_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((anchor, issue_id)): Path<(String, Uuid)>,
    Json(body): Json<VoteIn>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let b = require_anchor(&st.db, &anchor).await?;
    if !b.is_votes_enabled {
        return Err(ApiError::bad("Votes are not enabled for this project board"));
    }
    if !issue_in_board_scope(&st.db, &b, issue_id).await? {
        return Err(ApiError::not_found());
    }

    let vote_val = body.vote.unwrap_or(1);
    let id = Uuid::new_v4();

    let row: (Value,) = sqlx::query_as(
        "WITH ins AS (
            INSERT INTO issue_votes (
                id, created_at, updated_at, vote, actor_id, created_by_id,
                issue_id, project_id, updated_by_id, workspace_id
            )
            VALUES ($1, now(), now(), $2, $3, $3, $4, $5, $3, $6)
            ON CONFLICT (issue_id, actor_id) DO UPDATE
            SET vote = EXCLUDED.vote, updated_at = now(), deleted_at = NULL
            RETURNING *
        )
        SELECT jsonb_build_object(
            'issue', ins.issue_id,
            'vote', ins.vote,
            'workspace', ins.workspace_id,
            'project', ins.project_id,
            'actor', ins.actor_id,
            'actor_detail', jsonb_build_object(
                'id', u.id,
                'first_name', u.first_name,
                'last_name', u.last_name,
                'avatar', u.avatar,
                'display_name', u.display_name
            )
        )
        FROM ins
        JOIN users u ON u.id = ins.actor_id",
    )
    .bind(id)
    .bind(vote_val)
    .bind(user.id)
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok((StatusCode::CREATED, Json(row.0)))
}

/// DELETE /api/public/anchor/{anchor}/issues/{issue_id}/votes/
async fn issue_vote_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((anchor, issue_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let b = require_anchor(&st.db, &anchor).await?;

    let res = sqlx::query(
        "UPDATE issue_votes
         SET deleted_at = now()
         WHERE issue_id = $1 AND actor_id = $2
           AND project_id = $3 AND workspace_id = $4 AND deleted_at IS NULL",
    )
    .bind(issue_id)
    .bind(user.id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .execute(&st.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct IntakeIssueCreateIn {
    issue: IntakeIssuePayload,
}

#[derive(Deserialize)]
struct IntakeIssuePayload {
    name: String,
    description_html: Option<String>,
    description_json: Option<Value>,
    priority: Option<String>,
}

/// GET /api/public/anchor/{anchor}/intakes/{intake_id}/intake-issues/
async fn intake_issues_list(
    State(st): State<AppState>,
    _user: MaybeUser,
    Path((anchor, intake_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    if b.intake_id != Some(intake_id) {
        return Err(ApiError::bad("Intake is not enabled for this Project Board"));
    }

    let rows: Value = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', i.id,
            'name', i.name,
            'priority', i.priority,
            'sequence_id', i.sequence_id,
            'project_id', i.project_id,
            'created_at', i.created_at,
            'bridge_id', ii.id,
            'issue_intake', jsonb_build_array(jsonb_build_object(
                'id', ii.id,
                'status', ii.status,
                'duplicate_to', ii.duplicate_to_id,
                'snoozed_till', ii.snoozed_till,
                'source', ii.source
            )),
            'state_detail', (SELECT jsonb_build_object('id', s.id, 'name', s.name, 'color', s.color, 'group', s.group)
                             FROM states s WHERE s.id = i.state_id),
            'project_detail', (SELECT jsonb_build_object('id', p.id, 'name', p.name, 'identifier', p.identifier)
                               FROM projects p WHERE p.id = i.project_id)
        ) ORDER BY ii.created_at DESC), '[]'::jsonb)
        FROM intake_issues ii
        JOIN issues i ON i.id = ii.issue_id
        WHERE ii.intake_id = $1 AND ii.project_id = $2 AND ii.workspace_id = $3
          AND ii.deleted_at IS NULL AND i.deleted_at IS NULL",
    )
    .bind(intake_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(rows))
}

/// POST /api/public/anchor/{anchor}/intakes/{intake_id}/intake-issues/
async fn intake_issue_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((anchor, intake_id)): Path<(String, Uuid)>,
    Json(body): Json<IntakeIssueCreateIn>,
) -> ApiResult {
    let b = require_anchor(&st.db, &anchor).await?;
    if b.intake_id.is_none() {
        return Err(ApiError::bad("Intake is not enabled for this Project Board"));
    }
    if b.intake_id != Some(intake_id) {
        return Err(ApiError::bad("Intake does not belong to this Project Board"));
    }
    if body.issue.name.trim().is_empty() {
        return Err(ApiError::bad("Name is required"));
    }
    if body.issue.name.chars().count() > 255 {
        return Err(ApiError::bad("Name must be at most 255 characters"));
    }
    let priority = body.issue.priority.clone().unwrap_or_else(|| "low".to_string());
    if !["low", "medium", "high", "urgent", "none"].contains(&priority.as_str()) {
        return Err(ApiError::bad("Invalid priority"));
    }

    let mut tx = st.db.begin().await?;

    // Triage state
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1::text))")
        .bind(b.project_id)
        .execute(&mut *tx)
        .await?;

    let triage: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM states WHERE project_id = $1 AND \"group\" = 'triage' AND deleted_at IS NULL LIMIT 1",
    )
    .bind(b.project_id)
    .fetch_optional(&mut *tx)
    .await?;

    let triage = match triage {
        Some(t) => t,
        None => {
            sqlx::query_scalar(
                "INSERT INTO states (id, name, color, \"group\", \"default\", sequence, project_id, workspace_id,
                     created_by_id, updated_by_id, created_at, updated_at)
                 VALUES (gen_random_uuid(), 'Triage', '#4E5355', 'triage', false, 65000, $1, $2, $3, $3, now(), now())
                 RETURNING id",
            )
            .bind(b.project_id)
            .bind(b.workspace_id)
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?
        }
    };

    let seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(max(sequence), 0) + 1 FROM issue_sequences WHERE project_id = $1",
    )
    .bind(b.project_id)
    .fetch_one(&mut *tx)
    .await?;

    let issue_id = Uuid::new_v4();
    let raw_desc_html = body.issue.description_html.unwrap_or_else(|| "<p></p>".to_string());
    let desc_html = sanitize_html(&raw_desc_html)?;
    let desc_json = body.issue.description_json.unwrap_or_else(|| json!({}));
    let stripped = strip_tags(&desc_html);

    sqlx::query(
        "INSERT INTO issues (id, name, description_html, description_stripped, description_json, priority,
             sequence_id, project_id, workspace_id, state_id, sort_order, is_draft, created_by_id, updated_by_id,
             created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 65535, false, $11, $11, now(), now())",
    )
    .bind(issue_id)
    .bind(&body.issue.name)
    .bind(&desc_html)
    .bind(&stripped)
    .bind(&desc_json)
    .bind(&priority)
    .bind(seq as i32)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .bind(triage)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO issue_sequences (id, sequence, deleted, issue_id, project_id, workspace_id,
             created_by_id, updated_by_id, created_at, updated_at)
         VALUES (gen_random_uuid(), $1, false, $2, $3, $4, $5, $5, now(), now())",
    )
    .bind(seq)
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;

    let ii_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO intake_issues (id, status, source, intake_id, issue_id, project_id, workspace_id,
             created_by_id, updated_by_id, created_at, updated_at, extra)
         VALUES ($1, -2, 'IN_APP', $2, $3, $4, $5, $6, $6, now(), now(), '{}'::jsonb)",
    )
    .bind(ii_id)
    .bind(intake_id)
    .bind(issue_id)
    .bind(b.project_id)
    .bind(b.workspace_id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let out: (Value,) = sqlx::query_as(
        "SELECT jsonb_build_object(
            'id', i.id,
            'name', i.name,
            'priority', i.priority,
            'sequence_id', i.sequence_id,
            'project_id', i.project_id,
            'created_at', i.created_at,
            'bridge_id', ii.id,
            'issue_intake', jsonb_build_array(jsonb_build_object(
                'id', ii.id,
                'status', ii.status,
                'duplicate_to', ii.duplicate_to_id,
                'snoozed_till', ii.snoozed_till,
                'source', ii.source
            )),
            'state_detail', (SELECT jsonb_build_object('id', s.id, 'name', s.name, 'color', s.color, 'group', s.group)
                             FROM states s WHERE s.id = i.state_id),
            'project_detail', (SELECT jsonb_build_object('id', p.id, 'name', p.name, 'identifier', p.identifier)
                               FROM projects p WHERE p.id = i.project_id)
        )
        FROM intake_issues ii
        JOIN issues i ON i.id = ii.issue_id
        WHERE ii.id = $1",
    )
    .bind(ii_id)
    .fetch_one(&st.db)
    .await?;

    // Plane answers 200 here, not 201
    Ok(Json(out.0))
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
