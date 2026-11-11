//! User dashboard: `/api/users/me/activities/`, the per-workspace activity and completed graphs, and
//! the workspace dashboard. Port of Plane's `app/urls/user.py`, `app/views/user/base.py`
//! (UserActivityEndpoint), `app/views/workspace/user.py` (graphs) and `app/views/workspace/base.py`
//! (UserWorkspaceDashboardEndpoint).
//!
//! Every value reaches SQL as a bound parameter. Issue queries use the work-item visibility rule
//! (active project member; guests see own items unless the project enables guest_view_all_features).
use std::collections::HashMap;

use uuid::Uuid;

use crate::common::*;
use crate::workitems::{ACTIVE, Arg, Base, Scope, list};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde_json::{Value, json};
use sqlx::PgPool;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/users/me/activities/", get(my_activities))
        .route("/api/users/me/workspaces/{slug}/activity-graph/", get(activity_graph))
        .route(
            "/api/users/me/workspaces/{slug}/issues-completed-graph/",
            get(completed_graph),
        )
        .route("/api/users/me/workspaces/{slug}/dashboard/", get(dashboard))
        .route("/api/workspaces/{slug}/user-issues/{user_id}/", get(user_issues))
        .route("/api/workspaces/{slug}/entity-search/", get(entity_search))
}

/// Plane `_visible_issues_q`: caller is an active member of the issue's project, and guests only
/// see issues they created unless `guest_view_all_features`. Binds `$1` = caller.
const VISIBLE: &str =
    "EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = i.project_id AND pm.member_id = $1
    AND pm.is_active AND pm.deleted_at IS NULL
    AND (pm.role > 5 OR EXISTS (SELECT 1 FROM projects p WHERE p.id = i.project_id AND p.guest_view_all_features)
         OR i.created_by_id = $1))";

/// Issue is assigned to `$2` (the caller).
const ASSIGNED: &str = "EXISTS (SELECT 1 FROM issue_assignees a WHERE a.issue_id = i.id AND a.assignee_id = $2
    AND a.deleted_at IS NULL)";

/// Plane's `state__group` not in completed/cancelled. A NULL state counts as pending, as in Django.
const PENDING: &str = "(s.group IS NULL OR s.group NOT IN ('completed', 'cancelled'))";

/// Caller's issues in the workspace (`$3`), visible and not archived/draft/triage.
fn scope() -> String {
    format!("i.workspace_id = $3 AND {ACTIVE} AND {VISIBLE} AND {ASSIGNED}")
}

/// IssueActivitySerializer shape. `actor_detail` is UserLite; project/workspace/issue details are the
/// lite serializers' fields.
// ponytail: issue_detail carries a subset of IssueFlatSerializer fields. Add the rest if the UI reads them.
fn activity_json() -> String {
    format!(
        "jsonb_build_object(
        'id', a.id, 'verb', a.verb, 'field', a.field, 'old_value', a.old_value, 'new_value', a.new_value,
        'comment', a.comment, 'attachments', a.attachments, 'epoch', a.epoch,
        'old_identifier', a.old_identifier, 'new_identifier', a.new_identifier,
        'created_at', a.created_at, 'updated_at', a.updated_at, 'created_by', a.created_by_id,
        'updated_by', a.updated_by_id, 'actor', a.actor_id, 'issue', a.issue_id,
        'issue_comment', a.issue_comment_id, 'project', a.project_id, 'workspace', a.workspace_id,
        'actor_detail', (SELECT {} FROM users u WHERE u.id = a.actor_id),
        'project_detail', (SELECT jsonb_build_object('id', p.id, 'identifier', p.identifier, 'name', p.name,
            'cover_image', p.cover_image, 'logo_props', p.logo_props, 'description', p.description)
            FROM projects p WHERE p.id = a.project_id),
        'workspace_detail', (SELECT jsonb_build_object('id', w.id, 'name', w.name, 'slug', w.slug)
            FROM workspaces w WHERE w.id = a.workspace_id),
        'issue_detail', (SELECT jsonb_build_object('id', i.id, 'name', i.name, 'priority', i.priority,
            'start_date', i.start_date, 'target_date', i.target_date, 'sequence_id', i.sequence_id,
            'project_id', i.project_id) FROM issues i WHERE i.id = a.issue_id)
        )",
        crate::users::USER_LITE_JSON
    )
}

/// Plane `_paginate` offset cursor: `value:offset:is_prev`, where offset is the page index.
fn page_of(cursor: Option<&String>) -> i64 {
    cursor
        .and_then(|c| c.split(':').nth(1))
        .and_then(|p| p.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0)
}

/// `GET /api/users/me/activities/`: the caller's own issue activities, all workspaces, paginated.
async fn my_activities(
    State(st): State<AppState>,
    user: CurrentUser,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let per_page = q
        .get("per_page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1000)
        .clamp(1, 1000);
    let page = page_of(q.get("cursor"));
    let order = match q.get("order_by").map(String::as_str).unwrap_or("-created_at") {
        "created_at" => "a.created_at ASC",
        "updated_at" => "a.updated_at ASC",
        "-updated_at" => "a.updated_at DESC",
        _ => "a.created_at DESC",
    };
    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM issue_activities a WHERE a.actor_id = $1 AND a.deleted_at IS NULL")
            .bind(user.id)
            .fetch_one(&st.db)
            .await?;
    let mut rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {} FROM issue_activities a WHERE a.actor_id = $1 AND a.deleted_at IS NULL
         ORDER BY {order} LIMIT $2 OFFSET $3",
        activity_json()
    ))
    .bind(user.id)
    .bind(per_page + 1)
    .bind(page * per_page)
    .fetch_all(&st.db)
    .await?;
    let has_next = rows.len() as i64 > per_page;
    rows.truncate(per_page as usize);
    Ok(Json(json!({
        "next_cursor": format!("{per_page}:{}:0", page + 1),
        "prev_cursor": format!("{per_page}:{}:1", (page - 1).max(0)),
        "next_page_results": has_next,
        "prev_page_results": page > 0,
        "count": rows.len(),
        "total_pages": (total + per_page - 1) / per_page,
        "total_results": total,
        "extra_stats": null,
        "results": rows,
    })))
}

/// Per-day activity counts for the caller in the workspace, over the last `months` months.
async fn activity_days(
    db: &PgPool,
    user: &CurrentUser,
    ws: &WorkspaceCtx,
    months: i32,
) -> Result<Vec<Value>, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT jsonb_build_object('created_date', d, 'activity_count', c) FROM (
            SELECT a.created_at::date AS d, count(*) AS c FROM issue_activities a
            WHERE a.actor_id = $1 AND a.workspace_id = $2 AND a.deleted_at IS NULL
              AND a.created_at::date >= (CURRENT_DATE - make_interval(months => $3))::date
            GROUP BY 1 ORDER BY 1) x",
    )
    .bind(user.id)
    .bind(ws.workspace_id)
    .bind(months)
    .fetch_all(db)
    .await?)
}

/// `GET .../activity-graph/`: six months of daily activity counts.
async fn activity_graph(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    Ok(Json(Value::Array(activity_days(&st.db, &user, &ws, 6).await?)))
}

fn parse_month(v: Option<&String>) -> Result<i32, ApiError> {
    match v {
        None => Ok(1),
        Some(m) => m
            .parse::<i32>()
            .ok()
            .filter(|m| (1..=12).contains(m))
            .ok_or_else(|| ApiError::bad("Invalid month")),
    }
}

/// `GET .../issues-completed-graph/?month=`: completed issues per week bucket (ISO week mod 4).
async fn completed_graph(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let month = parse_month(q.get("month"))?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT jsonb_build_object('week', w, 'completed_count', c) FROM (
                SELECT (extract(week FROM i.completed_at)::int % 4) AS w, count(*) AS c
                FROM issues i WHERE {} AND i.completed_at IS NOT NULL
                  AND extract(month FROM i.completed_at)::int = $4
                GROUP BY 1 ORDER BY 1) x",
            scope()
        ))
        .bind(user.id)
        .bind(user.id)
        .bind(ws.workspace_id)
        .bind(month),
        &st.db,
    )
    .await
}

/// `GET .../dashboard/?month=`: Plane's workspace dashboard for the caller.
async fn dashboard(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let month = parse_month(q.get("month"))?;
    let sc = scope();
    let db = &st.db;
    let (u, w) = (user.id, ws.workspace_id);

    let issue_activities = activity_days(db, &user, &ws, 3).await?;

    let completed_issues: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT jsonb_build_object('week_in_month', wk, 'completed_count', c) FROM (
            SELECT (((extract(day FROM i.completed_at)::int - 1) / 7) + 1) AS wk, count(*) AS c
            FROM issues i WHERE {sc} AND i.completed_at IS NOT NULL AND extract(month FROM i.completed_at)::int = $4
            GROUP BY 1 ORDER BY 1) x"
    ))
    .bind(u)
    .bind(u)
    .bind(w)
    .bind(month)
    .fetch_all(db)
    .await?;

    let assigned: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM issues i LEFT JOIN states s ON s.id = i.state_id WHERE {sc}"
    ))
    .bind(u)
    .bind(u)
    .bind(w)
    .fetch_one(db)
    .await?;

    let pending: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM issues i LEFT JOIN states s ON s.id = i.state_id WHERE {sc} AND {PENDING}"
    ))
    .bind(u)
    .bind(u)
    .bind(w)
    .fetch_one(db)
    .await?;

    let completed: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM issues i LEFT JOIN states s ON s.id = i.state_id WHERE {sc} AND s.group = 'completed'"
    ))
    .bind(u)
    .bind(u)
    .bind(w)
    .fetch_one(db)
    .await?;

    // Plane compares ISO week numbers only, not years.
    let due_week: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM issues i LEFT JOIN states s ON s.id = i.state_id
         WHERE {sc} AND extract(week FROM i.target_date) = extract(week FROM CURRENT_DATE)"
    ))
    .bind(u)
    .bind(u)
    .bind(w)
    .fetch_one(db)
    .await?;

    let state_distribution: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT jsonb_build_object('state_group', g, 'state_count', c) FROM (
            SELECT s.group AS g, count(*) AS c FROM issues i LEFT JOIN states s ON s.id = i.state_id
            WHERE {sc} GROUP BY 1 ORDER BY 1) x"
    ))
    .bind(u)
    .bind(u)
    .bind(w)
    .fetch_all(db)
    .await?;

    let overdue: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT jsonb_build_object('id', i.id, 'name', i.name, 'workspace__slug', wk.slug,
            'project_id', i.project_id, 'target_date', i.target_date)
         FROM issues i LEFT JOIN states s ON s.id = i.state_id JOIN workspaces wk ON wk.id = i.workspace_id
         WHERE {sc} AND {PENDING} AND i.target_date < now() AND i.completed_at IS NULL"
    ))
    .bind(u)
    .bind(u)
    .bind(w)
    .fetch_all(db)
    .await?;

    let upcoming: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT jsonb_build_object('id', i.id, 'name', i.name, 'workspace__slug', wk.slug,
            'project_id', i.project_id, 'start_date', i.start_date)
         FROM issues i LEFT JOIN states s ON s.id = i.state_id JOIN workspaces wk ON wk.id = i.workspace_id
         WHERE {sc} AND {PENDING} AND i.start_date >= now() AND i.completed_at IS NULL"
    ))
    .bind(u)
    .bind(u)
    .bind(w)
    .fetch_all(db)
    .await?;

    Ok(Json(json!({
        "issue_activities": issue_activities,
        "completed_issues": completed_issues,
        "assigned_issues_count": assigned,
        "pending_issues_count": pending,
        "completed_issues_count": completed,
        "issues_due_week_count": due_week,
        "state_distribution": state_distribution,
        "overdue_issues": overdue,
        "upcoming_issues": upcoming,
    })))
}

/// `GET /api/workspaces/{slug}/user-issues/{user_id}/`: work items the target is assigned to, created by,
/// or subscribed to, that the caller can see. Uses the shared list engine (filters, order, group, cursor).
// ponytail: user-activity export (CSV job) skipped, per scope.
async fn user_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, target)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let base = Base::new(
        format!(
            "i.workspace_id = $1 AND {ACTIVE}
             AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = i.project_id AND pm.member_id = $2
                 AND pm.is_active AND pm.deleted_at IS NULL
                 AND (pm.role > 5 OR EXISTS (SELECT 1 FROM projects p WHERE p.id = i.project_id AND p.guest_view_all_features)
                      OR i.created_by_id = $2))
             AND (i.created_by_id = $3
                 OR EXISTS (SELECT 1 FROM issue_assignees a WHERE a.issue_id = i.id AND a.assignee_id = $3 AND a.deleted_at IS NULL)
                 OR EXISTS (SELECT 1 FROM issue_subscribers sb WHERE sb.issue_id = i.id AND sb.subscriber_id = $3 AND sb.deleted_at IS NULL))"
        ),
        vec![Arg::Uuid(ws.workspace_id), Arg::Uuid(user.id), Arg::Uuid(target)],
    );
    list(
        &st.db,
        base,
        &q,
        Scope::Workspace {
            workspace_id: ws.workspace_id,
            user_id: user.id,
        },
    )
    .await
}

/// Avatar URL for a user alias `u`, as Plane's `member__avatar_url` annotation.
const AVATAR_URL: &str =
    "CASE WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id || '/'
    ELSE NULLIF(u.avatar, '') END";

#[derive(serde::Deserialize)]
struct EntityQ {
    query: Option<String>,
    query_type: Option<String>,
    count: Option<i64>,
    project_id: Option<Uuid>,
}

/// ILIKE pattern with `%`, `_` and `\` escaped, so the user's text matches literally.
fn like_pattern(text: &str) -> String {
    let esc = text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{esc}%")
}

/// `GET /api/workspaces/{slug}/entity-search/`: mention, project, issue, cycle, module and page lookups
/// inside one project (Plane `SearchEndpoint`). Membership comes from `require_project`. Restricted
/// guests see only the issues they created, as in the list endpoints.
// ponytail: Plane answers 200 with data for any workspace member in user_mention. Here a non-member of
// the project gets 403. Keeps the stricter tenant rule from AGENTS.md.
async fn entity_search(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<EntityQ>,
) -> ApiResult {
    let Some(project_id) = q.project_id else {
        // Plane only fills results when a project is given.
        require_workspace(&st.db, &user, &slug).await?;
        return Ok(Json(json!({})));
    };
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    let ws_id = p.workspace_id();
    let count = q.count.unwrap_or(5).clamp(1, 100);
    let text = q.query.filter(|s| !s.is_empty());
    let pat = text.as_deref().map(like_pattern);
    // Sequence ids named in the query, e.g. "12" or "PRJ 12" (whole integers only, as Plane).
    let seqs: Vec<i32> = text
        .as_deref()
        .unwrap_or("")
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|d| d.parse::<i32>().ok())
        .collect();
    let restricted = p.restricted_guest();

    let mut out = serde_json::Map::new();
    for t in q
        .query_type
        .as_deref()
        .unwrap_or("user_mention")
        .split(',')
        .map(str::trim)
    {
        let rows: Vec<Value> = match t {
            "user_mention" => {
                sqlx::query_scalar(&format!(
                "SELECT jsonb_build_object('member__avatar_url', {AVATAR_URL}, 'member__display_name', u.display_name,
                    'member__id', u.id)
                 FROM project_members pm JOIN users u ON u.id = pm.member_id
                 WHERE pm.project_id = $1 AND pm.workspace_id = $2 AND pm.is_active AND pm.deleted_at IS NULL
                   AND NOT u.is_bot
                   AND ($3::text IS NULL OR u.first_name ILIKE $3 OR u.last_name ILIKE $3 OR u.display_name ILIKE $3)
                 ORDER BY pm.created_at DESC LIMIT $4"
            ))
                .bind(project_id)
                .bind(ws_id)
                .bind(pat.as_deref())
                .bind(count)
                .fetch_all(&st.db)
                .await?
            }
            "project" => {
                sqlx::query_scalar(
                    "SELECT jsonb_build_object('name', p.name, 'id', p.id, 'identifier', p.identifier,
                    'logo_props', p.logo_props, 'workspace__slug', w.slug)
                 FROM projects p JOIN workspaces w ON w.id = p.workspace_id
                 WHERE p.workspace_id = $1 AND p.deleted_at IS NULL
                   AND (EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = p.id AND pm.member_id = $2
                                AND pm.is_active AND pm.deleted_at IS NULL) OR p.network = 2)
                   AND ($3::text IS NULL OR p.name ILIKE $3 OR p.identifier ILIKE $3)
                 ORDER BY p.created_at DESC LIMIT $4",
                )
                .bind(ws_id)
                .bind(user.id)
                .bind(pat.as_deref())
                .bind(count)
                .fetch_all(&st.db)
                .await?
            }
            "issue" => {
                sqlx::query_scalar(&format!(
                    "SELECT jsonb_build_object('name', i.name, 'id', i.id, 'sequence_id', i.sequence_id,
                    'project__identifier', pr.identifier, 'project_id', i.project_id, 'priority', i.priority,
                    'state_id', i.state_id, 'type_id', i.type_id)
                 FROM issues i JOIN projects pr ON pr.id = i.project_id
                 WHERE i.workspace_id = $1 AND i.project_id = $2 AND {ACTIVE}
                   AND (NOT $4::bool OR i.created_by_id = $5)
                   AND ($3::text IS NULL OR i.name ILIKE $3 OR pr.identifier ILIKE $3 OR i.sequence_id = ANY($6))
                 ORDER BY i.created_at DESC LIMIT $7"
                ))
                .bind(ws_id)
                .bind(project_id)
                .bind(pat.as_deref())
                .bind(restricted)
                .bind(user.id)
                .bind(&seqs)
                .bind(count)
                .fetch_all(&st.db)
                .await?
            }
            "cycle" => {
                sqlx::query_scalar(
                    "SELECT jsonb_build_object('name', c.name, 'id', c.id, 'project_id', c.project_id,
                    'project__identifier', pr.identifier, 'workspace__slug', w.slug,
                    'status', CASE
                        WHEN c.start_date <= now() AND c.end_date >= now() THEN 'CURRENT'
                        WHEN c.start_date > now() THEN 'UPCOMING'
                        WHEN c.end_date < now() THEN 'COMPLETED'
                        ELSE 'DRAFT' END)
                 FROM cycles c JOIN projects pr ON pr.id = c.project_id JOIN workspaces w ON w.id = c.workspace_id
                 WHERE c.workspace_id = $1 AND c.project_id = $2 AND c.deleted_at IS NULL
                   AND ($3::text IS NULL OR c.name ILIKE $3)
                 ORDER BY c.created_at DESC LIMIT $4",
                )
                .bind(ws_id)
                .bind(project_id)
                .bind(pat.as_deref())
                .bind(count)
                .fetch_all(&st.db)
                .await?
            }
            "module" => {
                sqlx::query_scalar(
                    "SELECT jsonb_build_object('name', m.name, 'id', m.id, 'project_id', m.project_id,
                    'project__identifier', pr.identifier, 'status', m.status, 'workspace__slug', w.slug)
                 FROM modules m JOIN projects pr ON pr.id = m.project_id JOIN workspaces w ON w.id = m.workspace_id
                 WHERE m.workspace_id = $1 AND m.project_id = $2 AND m.deleted_at IS NULL
                   AND ($3::text IS NULL OR m.name ILIKE $3)
                 ORDER BY m.created_at DESC LIMIT $4",
                )
                .bind(ws_id)
                .bind(project_id)
                .bind(pat.as_deref())
                .bind(count)
                .fetch_all(&st.db)
                .await?
            }
            "page" => {
                sqlx::query_scalar(
                    "SELECT jsonb_build_object('name', pg.name, 'id', pg.id, 'logo_props', pg.logo_props,
                    'projects__id', pp.project_id, 'workspace__slug', w.slug)
                 FROM pages pg JOIN project_pages pp ON pp.page_id = pg.id AND pp.deleted_at IS NULL
                 JOIN workspaces w ON w.id = pg.workspace_id
                 WHERE pg.workspace_id = $1 AND pp.project_id = $2 AND pg.deleted_at IS NULL
                   AND pg.access = 0
                   AND (NOT $4::bool OR pg.owned_by_id = $5)
                   AND ($3::text IS NULL OR pg.name ILIKE $3)
                 ORDER BY pg.created_at DESC LIMIT $6",
                )
                .bind(ws_id)
                .bind(project_id)
                .bind(pat.as_deref())
                .bind(restricted)
                .bind(user.id)
                .bind(count)
                .fetch_all(&st.db)
                .await?
            }
            _ => continue,
        };
        out.insert(t.to_string(), Value::Array(rows));
    }
    Ok(Json(Value::Object(out)))
}
