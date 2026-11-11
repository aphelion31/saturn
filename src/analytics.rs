//! Analytics endpoints. Port of Plane's `app/urls/analytic.py`, `app/views/analytic/{base,advance,project_analytics}.py`,
//! `utils/analytics_plot.py` (x/y/segment grouping) and `utils/build_chart.py` (custom work-item charts).
//!
//! Axis, segment and group_by request values only select entries from fixed whitelists.
//! Every request value reaches SQL as a bound parameter.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use chrono::{Datelike, NaiveDate, Utc};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::common::*;
use crate::workitems::{Arg, Base};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/workspaces/{slug}/advance-analytics/", get(advance_analytics))
        .route("/api/workspaces/{slug}/advance-analytics-stats/", get(advance_stats))
        .route("/api/workspaces/{slug}/advance-analytics-charts/", get(advance_charts))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/advance-analytics/",
            get(project_advance_analytics),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/advance-analytics-stats/",
            get(project_advance_stats),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/advance-analytics-charts/",
            get(project_advance_charts),
        )
        .route("/api/workspaces/{slug}/project-stats/", get(project_stats))
        .route("/api/workspaces/{slug}/analytics/", get(analytics))
        .route("/api/workspaces/{slug}/default-analytics/", get(default_analytics))
        .route(
            "/api/workspaces/{slug}/analytic-view/",
            get(view_list).post(view_create),
        )
        .route(
            "/api/workspaces/{slug}/analytic-view/{pk}/",
            get(view_retrieve).patch(view_update).delete(view_destroy),
        )
        .route(
            "/api/workspaces/{slug}/saved-analytic-view/{analytic_id}/",
            get(saved_view_run),
        )
}

type Q = HashMap<String, String>;

fn qv<'a>(q: &'a Q, k: &str) -> Option<&'a str> {
    q.get(k).map(String::as_str).filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------------------------
// Filters (Plane `utils/date_utils.get_analytics_filters`)
// ---------------------------------------------------------------------------------------------

/// Project ids from `project_ids=a,b`. Non-UUID entries are dropped.
fn project_ids_of(q: &Q) -> Option<Vec<Uuid>> {
    let v = qv(q, "project_ids")?;
    let ids: Vec<Uuid> = v.split(',').filter_map(|s| Uuid::parse_str(s.trim()).ok()).collect();
    if ids.is_empty() { None } else { Some(ids) }
}

#[derive(Clone, Copy)]
struct Period {
    start: NaiveDate,
    end: NaiveDate,
}

fn period_of(q: &Q) -> Option<Period> {
    let today = Utc::now().date_naive();
    let d = |n: u64| today - chrono::Days::new(n);
    match qv(q, "date_filter")? {
        "yesterday" => Some(Period { start: d(1), end: d(1) }),
        "last_7_days" => Some(Period {
            start: d(7),
            end: today,
        }),
        "last_30_days" => Some(Period {
            start: d(30),
            end: today,
        }),
        "last_3_months" => Some(Period {
            start: d(90),
            end: today,
        }),
        "custom" => {
            let s = NaiveDate::parse_from_str(qv(q, "start_date")?, "%Y-%m-%d").ok()?;
            let e = NaiveDate::parse_from_str(qv(q, "end_date")?, "%Y-%m-%d").ok()?;
            Some(Period { start: s, end: e })
        }
        _ => None,
    }
}

/// Base WHERE on issues alias `i` for the caller: workspace, the caller's active project memberships,
/// not deleted, not archived, not draft, state not triage, project not archived/deleted.
fn issue_base(ws: Uuid, user: Uuid, q: &Q, project: Option<Uuid>) -> Base {
    let mut b = Base::new(
        "i.workspace_id = $1 AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft
         AND NOT EXISTS (SELECT 1 FROM states ts WHERE ts.id = i.state_id AND ts.group = 'triage')
         AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = i.project_id AND pm.member_id = $2
                     AND pm.is_active AND pm.deleted_at IS NULL)
         AND EXISTS (SELECT 1 FROM projects pp WHERE pp.id = i.project_id AND pp.deleted_at IS NULL
                     AND pp.archived_at IS NULL)",
        vec![Arg::Uuid(ws), Arg::Uuid(user)],
    );
    if let Some(p) = project {
        b.and("i.project_id = {}", Arg::Uuid(p));
    }
    if let Some(ids) = project_ids_of(q) {
        b.and("i.project_id = ANY({})", Arg::Uuids(ids));
    }
    b
}

fn guest_scope(b: &mut Base, restricted: bool, user: Uuid) {
    if restricted {
        b.and("i.created_by_id = {}", Arg::Uuid(user));
    }
}

fn apply_period(b: &mut Base, q: &Q, col: &str) {
    if let Some(p) = period_of(q) {
        b.and(&format!("{col}::date >= {{}}::date"), Arg::Text(p.start.to_string()));
        b.and(&format!("{col}::date <= {{}}::date"), Arg::Text(p.end.to_string()));
    }
}

async fn count(db: &sqlx::PgPool, from: &str, b: &Base, extra: &str) -> Result<i64, ApiError> {
    let sql = format!("SELECT count(*)::bigint FROM {from} WHERE {} AND {extra}", b.sql);
    let mut q = sqlx::query_scalar::<_, i64>(&sql);
    for a in &b.args {
        q = match a {
            Arg::Uuid(v) => q.bind(*v),
            Arg::Uuids(v) => q.bind(v.clone()),
            Arg::Text(v) => q.bind(v.clone()),
            Arg::Texts(v) => q.bind(v.clone()),
            Arg::Bool(v) => q.bind(*v),
        };
    }
    Ok(q.fetch_one(db).await?)
}

async fn fetch_json_rows(db: &sqlx::PgPool, sql: &str, b: &Base) -> Result<Vec<Value>, ApiError> {
    let mut q = sqlx::query_scalar::<_, Value>(sql);
    for a in &b.args {
        q = match a {
            Arg::Uuid(v) => q.bind(*v),
            Arg::Uuids(v) => q.bind(v.clone()),
            Arg::Text(v) => q.bind(v.clone()),
            Arg::Texts(v) => q.bind(v.clone()),
            Arg::Bool(v) => q.bind(*v),
        };
    }
    Ok(q.fetch_all(db).await?)
}

fn counted(n: i64) -> Value {
    json!({ "count": n })
}

// ---------------------------------------------------------------------------------------------
// Advance analytics: overview & work-items
// ---------------------------------------------------------------------------------------------

async fn advance_analytics(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<Q>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let tab = qv(&q, "tab").unwrap_or("overview");
    match tab {
        "overview" => overview(&st.db, &ws, &user, &q).await,
        "work-items" => {
            let mut base = issue_base(ws.workspace_id, user.id, &q, None);
            apply_period(&mut base, &q, "i.created_at");
            Ok(Json(work_item_counts(&st.db, &base).await?))
        }
        _ => Err(ApiError::bad_body(json!({"message": "Invalid tab"}))),
    }
}

async fn project_advance_analytics(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<Q>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let ws = &p.ws;
    let restricted = p.restricted_guest();
    let mut base = issue_base(ws.workspace_id, user.id, &q, Some(project_id));
    if let Some(c) = qv(&q, "cycle_id").and_then(|c| Uuid::parse_str(c).ok()) {
        base.and(
            "EXISTS (SELECT 1 FROM cycle_issues ci WHERE ci.issue_id = i.id AND ci.cycle_id = {} AND ci.deleted_at IS NULL)",
            Arg::Uuid(c),
        );
    }
    if let Some(m) = qv(&q, "module_id").and_then(|m| Uuid::parse_str(m).ok()) {
        base.and(
            "EXISTS (SELECT 1 FROM module_issues mi WHERE mi.issue_id = i.id AND mi.module_id = {} AND mi.deleted_at IS NULL)",
            Arg::Uuid(m),
        );
    }
    guest_scope(&mut base, restricted, user.id);
    apply_period(&mut base, &q, "i.created_at");
    Ok(Json(work_item_counts(&st.db, &base).await?))
}

async fn work_item_counts(db: &sqlx::PgPool, base: &Base) -> Result<Value, ApiError> {
    let mut out = Map::new();
    let total = count(db, "issues i", base, "TRUE").await?;
    out.insert("total_work_items".into(), counted(total));
    for (key, group) in [
        ("started_work_items", "started"),
        ("backlog_work_items", "backlog"),
        ("un_started_work_items", "unstarted"),
        ("completed_work_items", "completed"),
    ] {
        let extra = format!("EXISTS (SELECT 1 FROM states s WHERE s.id = i.state_id AND s.group = '{group}')");
        out.insert(key.into(), counted(count(db, "issues i", base, &extra).await?));
    }
    Ok(Value::Object(out))
}

async fn overview(db: &sqlx::PgPool, ws: &WorkspaceCtx, user: &CurrentUser, q: &Q) -> ApiResult {
    let mut out = Map::new();
    let ws_id = ws.workspace_id;
    let pids = project_ids_of(q);

    let (members_from, members_where, members_args): (&str, &str, Vec<Arg>) = match &pids {
        Some(ids) => (
            "project_members m JOIN users u ON u.id = m.member_id",
            "m.project_id = ANY($1) AND m.is_active AND m.deleted_at IS NULL AND NOT u.is_bot",
            vec![Arg::Uuids(ids.clone())],
        ),
        None => (
            "workspace_members m JOIN users u ON u.id = m.member_id",
            "m.workspace_id = $1 AND m.is_active AND m.deleted_at IS NULL AND NOT u.is_bot",
            vec![Arg::Uuid(ws_id)],
        ),
    };

    let role_count = |role: Option<i16>| {
        let mut b = Base::new(members_where, members_args.clone());
        if let Some(r) = role {
            b.and_raw(&format!("m.role = {r}"));
        }
        apply_period(&mut b, q, "m.created_at");
        async move { count(db, members_from, &b, "TRUE").await }
    };

    let total_users = role_count(None).await?;
    let total_admins = role_count(Some(ADMIN)).await?;
    let total_members = role_count(Some(MEMBER)).await?;
    let total_guests = role_count(Some(GUEST)).await?;

    let mut base = issue_base(ws_id, user.id, q, None);
    apply_period(&mut base, q, "i.created_at");
    let total_work_items = count(db, "issues i", &base, "TRUE").await?;

    let total_projects = {
        let mut p_b = Base::new(
            "p.workspace_id = $1 AND p.deleted_at IS NULL AND p.archived_at IS NULL
             AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = p.id AND pm.member_id = $2
                         AND pm.is_active AND pm.deleted_at IS NULL)",
            vec![Arg::Uuid(ws_id), Arg::Uuid(user.id)],
        );
        if let Some(ids) = &pids {
            p_b.and("p.id = ANY({})", Arg::Uuids(ids.clone()));
        }
        apply_period(&mut p_b, q, "p.created_at");
        count(db, "projects p", &p_b, "TRUE").await?
    };

    let total_cycles = {
        let mut c_b = cycles_base(ws_id, user.id, q);
        apply_period(&mut c_b, q, "c.created_at");
        count(db, "cycles c", &c_b, "TRUE").await?
    };

    let total_intake = count(
        db,
        "issues i",
        &base,
        "EXISTS (SELECT 1 FROM intake_issues ii WHERE ii.issue_id = i.id AND ii.deleted_at IS NULL AND ii.status IN (-2,-1,0,1,2))",
    )
    .await?;

    out.insert("total_users".into(), counted(total_users));
    out.insert("total_admins".into(), counted(total_admins));
    out.insert("total_members".into(), counted(total_members));
    out.insert("total_guests".into(), counted(total_guests));
    out.insert("total_projects".into(), counted(total_projects));
    out.insert("total_work_items".into(), counted(total_work_items));
    out.insert("total_cycles".into(), counted(total_cycles));
    out.insert("total_intake".into(), counted(total_intake));
    Ok(Json(Value::Object(out)))
}

fn cycles_base(ws: Uuid, user: Uuid, q: &Q) -> Base {
    let mut b = Base::new(
        "c.workspace_id = $1 AND c.deleted_at IS NULL AND c.archived_at IS NULL
         AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = c.project_id AND pm.member_id = $2
                     AND pm.is_active AND pm.deleted_at IS NULL)",
        vec![Arg::Uuid(ws), Arg::Uuid(user)],
    );
    if let Some(ids) = project_ids_of(q) {
        b.and("c.project_id = ANY({})", Arg::Uuids(ids));
    }
    b
}

fn modules_base(ws: Uuid, user: Uuid, q: &Q) -> Base {
    let mut b = Base::new(
        "m.workspace_id = $1 AND m.deleted_at IS NULL AND m.archived_at IS NULL
         AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = m.project_id AND pm.member_id = $2
                     AND pm.is_active AND pm.deleted_at IS NULL)",
        vec![Arg::Uuid(ws), Arg::Uuid(user)],
    );
    if let Some(ids) = project_ids_of(q) {
        b.and("m.project_id = ANY({})", Arg::Uuids(ids));
    }
    b
}

fn pages_base(ws: Uuid, user: Uuid, q: &Q) -> Base {
    let mut b = Base::new(
        "p.workspace_id = $1 AND p.deleted_at IS NULL
         AND EXISTS (SELECT 1 FROM pages pg WHERE pg.id = p.page_id AND pg.deleted_at IS NULL)
         AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = p.project_id AND pm.member_id = $2
                     AND pm.is_active AND pm.deleted_at IS NULL)
         AND EXISTS (SELECT 1 FROM projects pp WHERE pp.id = p.project_id AND pp.deleted_at IS NULL
                     AND pp.archived_at IS NULL)",
        vec![Arg::Uuid(ws), Arg::Uuid(user)],
    );
    if let Some(ids) = project_ids_of(q) {
        b.and("p.project_id = ANY({})", Arg::Uuids(ids));
    }
    b
}

fn views_base(ws: Uuid, user: Uuid, q: &Q) -> Base {
    let mut b = Base::new(
        "v.workspace_id = $1 AND v.deleted_at IS NULL
         AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = v.project_id AND pm.member_id = $2
                     AND pm.is_active AND pm.deleted_at IS NULL)",
        vec![Arg::Uuid(ws), Arg::Uuid(user)],
    );
    if let Some(ids) = project_ids_of(q) {
        b.and("v.project_id = ANY({})", Arg::Uuids(ids));
    }
    b
}

// ---------------------------------------------------------------------------------------------
// Advance analytics: stats
// ---------------------------------------------------------------------------------------------

async fn advance_stats(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<Q>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    match qv(&q, "type").unwrap_or("work-items") {
        "work-items" => {
            let mut base = issue_base(ws.workspace_id, user.id, &q, None);
            apply_period(&mut base, &q, "i.created_at");
            let sql = format!(
                "SELECT jsonb_build_object(
                    'project_id', i.project_id,
                    'project__name', p.name,
                    'cancelled_work_items', count(*) FILTER (WHERE s.group = 'cancelled')::bigint,
                    'completed_work_items', count(*) FILTER (WHERE s.group = 'completed')::bigint,
                    'backlog_work_items', count(*) FILTER (WHERE s.group = 'backlog')::bigint,
                    'un_started_work_items', count(*) FILTER (WHERE s.group = 'unstarted')::bigint,
                    'started_work_items', count(*) FILTER (WHERE s.group = 'started')::bigint
                 )
                 FROM issues i
                 JOIN projects p ON p.id = i.project_id
                 LEFT JOIN states s ON s.id = i.state_id
                 WHERE {}
                 GROUP BY i.project_id, p.name
                 ORDER BY i.project_id",
                base.sql
            );
            let rows = fetch_json_rows(&st.db, &sql, &base).await?;
            Ok(Json(Value::Array(rows)))
        }
        _ => Err(ApiError::bad_body(json!({"message": "Invalid type"}))),
    }
}

async fn project_advance_stats(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<Q>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let ws = &p.ws;
    let restricted = p.restricted_guest();
    match qv(&q, "type").unwrap_or("work-items") {
        "work-items" => {
            let mut base = issue_base(ws.workspace_id, user.id, &q, Some(project_id));
            if let Some(c) = qv(&q, "cycle_id").and_then(|c| Uuid::parse_str(c).ok()) {
                base.and(
                    "EXISTS (SELECT 1 FROM cycle_issues ci WHERE ci.issue_id = i.id AND ci.cycle_id = {} AND ci.deleted_at IS NULL)",
                    Arg::Uuid(c),
                );
            }
            if let Some(m) = qv(&q, "module_id").and_then(|m| Uuid::parse_str(m).ok()) {
                base.and(
                    "EXISTS (SELECT 1 FROM module_issues mi WHERE mi.issue_id = i.id AND mi.module_id = {} AND mi.deleted_at IS NULL)",
                    Arg::Uuid(m),
                );
            }
            guest_scope(&mut base, restricted, user.id);
            apply_period(&mut base, &q, "i.created_at");

            let sql = format!(
                "SELECT jsonb_build_object(
                    'display_name', u.display_name,
                    'assignee_id', u.id,
                    'avatar_url', CASE
                        WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id::text || '/'
                        ELSE NULLIF(u.avatar, '')
                    END,
                    'cancelled_work_items', count(DISTINCT i.id) FILTER (WHERE s.group = 'cancelled')::bigint,
                    'completed_work_items', count(DISTINCT i.id) FILTER (WHERE s.group = 'completed')::bigint,
                    'backlog_work_items', count(DISTINCT i.id) FILTER (WHERE s.group = 'backlog')::bigint,
                    'un_started_work_items', count(DISTINCT i.id) FILTER (WHERE s.group = 'unstarted')::bigint,
                    'started_work_items', count(DISTINCT i.id) FILTER (WHERE s.group = 'started')::bigint
                 )
                 FROM issues i
                 LEFT JOIN states s ON s.id = i.state_id
                 LEFT JOIN issue_assignees ia ON ia.issue_id = i.id AND ia.deleted_at IS NULL
                 LEFT JOIN users u ON u.id = ia.assignee_id
                 WHERE {}
                 GROUP BY u.id, u.display_name, u.avatar_asset_id, u.avatar
                 ORDER BY u.display_name NULLS LAST",
                base.sql
            );
            let rows = fetch_json_rows(&st.db, &sql, &base).await?;
            Ok(Json(Value::Array(rows)))
        }
        _ => Err(ApiError::bad_body(json!({"message": "Invalid type"}))),
    }
}

// ---------------------------------------------------------------------------------------------
// Advance analytics: charts
// ---------------------------------------------------------------------------------------------

async fn advance_charts(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<Q>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let chart_type = qv(&q, "type").unwrap_or("projects");
    match chart_type {
        "projects" => project_chart(&st.db, &ws, &user, &q).await,
        "work-items" => {
            let mut base = issue_base(ws.workspace_id, user.id, &q, None);
            apply_period(&mut base, &q, "i.created_at");
            let ws_created: chrono::DateTime<Utc> =
                sqlx::query_scalar("SELECT created_at FROM workspaces WHERE id = $1")
                    .bind(ws.workspace_id)
                    .fetch_one(&st.db)
                    .await?;
            monthly_completion_chart(&st.db, &base, ws_created.date_naive(), &q).await
        }
        "custom-work-items" => {
            let mut base = issue_base(ws.workspace_id, user.id, &q, None);
            apply_period(&mut base, &q, "i.created_at");
            let x_axis = qv(&q, "x_axis").unwrap_or("PRIORITY");
            let group_by = qv(&q, "group_by");
            build_custom_chart(&st.db, &base, x_axis, group_by).await
        }
        _ => Err(ApiError::bad_body(json!({"message": "Invalid type"}))),
    }
}

async fn project_advance_charts(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<Q>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let ws = &p.ws;
    let restricted = p.restricted_guest();
    let mut base = issue_base(ws.workspace_id, user.id, &q, Some(project_id));
    let cycle_id = qv(&q, "cycle_id").and_then(|c| Uuid::parse_str(c).ok());
    let module_id = qv(&q, "module_id").and_then(|m| Uuid::parse_str(m).ok());
    if let Some(c) = cycle_id {
        base.and(
            "EXISTS (SELECT 1 FROM cycle_issues ci WHERE ci.issue_id = i.id AND ci.cycle_id = {} AND ci.deleted_at IS NULL)",
            Arg::Uuid(c),
        );
    }
    if let Some(m) = module_id {
        base.and(
            "EXISTS (SELECT 1 FROM module_issues mi WHERE mi.issue_id = i.id AND mi.module_id = {} AND mi.deleted_at IS NULL)",
            Arg::Uuid(m),
        );
    }
    guest_scope(&mut base, restricted, user.id);
    apply_period(&mut base, &q, "i.created_at");

    let chart_type = qv(&q, "type").unwrap_or("work-items");
    match chart_type {
        "work-items" => {
            if let Some(c) = cycle_id {
                let dates: Option<(Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>)> =
                    sqlx::query_as("SELECT start_date, end_date FROM cycles WHERE id = $1 AND deleted_at IS NULL")
                        .bind(c)
                        .fetch_optional(&st.db)
                        .await?;
                let (Some(s), Some(e)) = dates.unwrap_or((None, None)) else {
                    return Ok(Json(json!({"data": [], "schema": {}})));
                };
                daily_completion_chart(&st.db, &base, s.date_naive(), e.date_naive()).await
            } else if let Some(m) = module_id {
                let dates: Option<(Option<NaiveDate>, Option<NaiveDate>)> =
                    sqlx::query_as("SELECT start_date, target_date FROM modules WHERE id = $1 AND deleted_at IS NULL")
                        .bind(m)
                        .fetch_optional(&st.db)
                        .await?;
                let (Some(s), Some(e)) = dates.unwrap_or((None, None)) else {
                    return Ok(Json(json!({"data": [], "schema": {}})));
                };
                daily_completion_chart(&st.db, &base, s, e).await
            } else {
                let proj_created: chrono::DateTime<Utc> =
                    sqlx::query_scalar("SELECT created_at FROM projects WHERE id = $1")
                        .bind(project_id)
                        .fetch_one(&st.db)
                        .await?;
                monthly_completion_chart(&st.db, &base, proj_created.date_naive(), &q).await
            }
        }
        "custom-work-items" => {
            let x_axis = qv(&q, "x_axis").unwrap_or("PRIORITY");
            let group_by = qv(&q, "group_by");
            build_custom_chart(&st.db, &base, x_axis, group_by).await
        }
        _ => Err(ApiError::bad_body(json!({"message": "Invalid type"}))),
    }
}

async fn project_chart(db: &sqlx::PgPool, ws: &WorkspaceCtx, user: &CurrentUser, q: &Q) -> ApiResult {
    let ws_id = ws.workspace_id;
    let mut base = issue_base(ws_id, user.id, q, None);
    apply_period(&mut base, q, "i.created_at");
    let total_work_items = count(db, "issues i", &base, "TRUE").await?;

    let mut cyc = cycles_base(ws_id, user.id, q);
    apply_period(&mut cyc, q, "c.created_at");
    let total_cycles = count(db, "cycles c", &cyc, "TRUE").await?;

    let mut mod_b = modules_base(ws_id, user.id, q);
    apply_period(&mut mod_b, q, "m.created_at");
    let total_modules = count(db, "modules m", &mod_b, "TRUE").await?;

    let total_intake = count(
        db,
        "issues i",
        &base,
        "EXISTS (SELECT 1 FROM intake_issues ii WHERE ii.issue_id = i.id AND ii.deleted_at IS NULL)",
    )
    .await?;

    let total_members = {
        let mut m_b = Base::new(
            "m.workspace_id = $1 AND m.is_active AND m.deleted_at IS NULL",
            vec![Arg::Uuid(ws_id)],
        );
        apply_period(&mut m_b, q, "m.created_at");
        count(db, "workspace_members m", &m_b, "TRUE").await?
    };

    let mut pag = pages_base(ws_id, user.id, q);
    apply_period(&mut pag, q, "p.created_at");
    let total_pages = count(db, "project_pages p", &pag, "TRUE").await?;

    let mut viw = views_base(ws_id, user.id, q);
    apply_period(&mut viw, q, "v.created_at");
    let total_views = count(db, "issue_views v", &viw, "TRUE").await?;

    let items = vec![
        json!({"key": "work_items", "name": "Work Items", "count": total_work_items}),
        json!({"key": "cycles", "name": "Cycles", "count": total_cycles}),
        json!({"key": "modules", "name": "Modules", "count": total_modules}),
        json!({"key": "intake", "name": "Intake", "count": total_intake}),
        json!({"key": "members", "name": "Members", "count": total_members}),
        json!({"key": "pages", "name": "Pages", "count": total_pages}),
        json!({"key": "views", "name": "Views", "count": total_views}),
    ];
    Ok(Json(Value::Array(items)))
}

async fn monthly_completion_chart(db: &sqlx::PgPool, base: &Base, start_date: NaiveDate, q: &Q) -> ApiResult {
    let mut scoped = base.clone();
    let effective_start = if let Some(p) = period_of(q) {
        scoped.and("i.created_at::date >= {}::date", Arg::Text(p.start.to_string()));
        scoped.and("i.created_at::date <= {}::date", Arg::Text(p.end.to_string()));
        p.start
    } else {
        start_date
    };

    let sql = format!(
        "SELECT to_char(date_trunc('month', i.created_at), 'YYYY-MM-01') AS month_str,
                count(*)::bigint AS created_count,
                count(*) FILTER (WHERE s.group = 'completed')::bigint AS completed_count
         FROM issues i
         LEFT JOIN states s ON s.id = i.state_id
         WHERE {}
         GROUP BY date_trunc('month', i.created_at)
         ORDER BY date_trunc('month', i.created_at)",
        scoped.sql
    );
    let mut qq = sqlx::query_as::<_, (String, i64, i64)>(&sql);
    for a in &scoped.args {
        qq = match a {
            Arg::Uuid(v) => qq.bind(*v),
            Arg::Uuids(v) => qq.bind(v.clone()),
            Arg::Text(v) => qq.bind(v.clone()),
            Arg::Texts(v) => qq.bind(v.clone()),
            Arg::Bool(v) => qq.bind(*v),
        };
    }
    let rows = qq.fetch_all(db).await?;
    let mut stats_map: HashMap<String, (i64, i64)> = HashMap::new();
    for (m, created_c, completed_c) in rows {
        stats_map.insert(m, (created_c, completed_c));
    }

    let end_date = Utc::now().date_naive();
    let last_month = NaiveDate::from_ymd_opt(end_date.year(), end_date.month(), 1).unwrap_or(end_date);
    let mut cur =
        NaiveDate::from_ymd_opt(effective_start.year(), effective_start.month(), 1).unwrap_or(effective_start);

    let mut data = Vec::new();
    while cur <= last_month {
        let date_str = cur.format("%Y-%m-01").to_string();
        let (created_c, completed_c) = stats_map.get(&date_str).copied().unwrap_or((0, 0));
        data.push(json!({
            "key": date_str,
            "name": date_str,
            "count": created_c,
            "completed_issues": completed_c,
            "created_issues": created_c,
        }));
        let (y, m) = if cur.month() == 12 {
            (cur.year() + 1, 1)
        } else {
            (cur.year(), cur.month() + 1)
        };
        cur = match NaiveDate::from_ymd_opt(y, m, 1) {
            Some(d) => d,
            None => break,
        };
    }

    Ok(Json(json!({
        "data": data,
        "schema": {
            "completed_issues": "completed_issues",
            "created_issues": "created_issues"
        }
    })))
}

async fn daily_completion_chart(db: &sqlx::PgPool, base: &Base, start: NaiveDate, end: NaiveDate) -> ApiResult {
    let sql = format!(
        "SELECT to_char(i.created_at::date, 'YYYY-MM-DD') AS day_str,
                count(*)::bigint AS created_count,
                count(*) FILTER (WHERE s.group = 'completed')::bigint AS completed_count
         FROM issues i
         LEFT JOIN states s ON s.id = i.state_id
         WHERE {}
         GROUP BY i.created_at::date
         ORDER BY i.created_at::date",
        base.sql
    );
    let mut qq = sqlx::query_as::<_, (String, i64, i64)>(&sql);
    for a in &base.args {
        qq = match a {
            Arg::Uuid(v) => qq.bind(*v),
            Arg::Uuids(v) => qq.bind(v.clone()),
            Arg::Text(v) => qq.bind(v.clone()),
            Arg::Texts(v) => qq.bind(v.clone()),
            Arg::Bool(v) => qq.bind(*v),
        };
    }
    let rows = qq.fetch_all(db).await?;
    let mut stats_map: HashMap<String, (i64, i64)> = HashMap::new();
    for (d, created_c, completed_c) in rows {
        stats_map.insert(d, (created_c, completed_c));
    }

    let mut data = Vec::new();
    let mut cur = start;
    while cur <= end {
        let date_str = cur.format("%Y-%m-%d").to_string();
        let (created_c, completed_c) = stats_map.get(&date_str).copied().unwrap_or((0, 0));
        data.push(json!({
            "key": date_str,
            "name": date_str,
            "count": created_c + completed_c,
            "completed_issues": completed_c,
            "created_issues": created_c,
        }));
        cur = match cur.checked_add_days(chrono::Days::new(1)) {
            Some(d) => d,
            None => break,
        };
    }

    Ok(Json(json!({
        "data": data,
        "schema": {
            "completed_issues": "completed_issues",
            "created_issues": "created_issues"
        }
    })))
}

// ---------------------------------------------------------------------------------------------
// Custom work-item charts (x_axis & group_by fixed whitelists)
// ---------------------------------------------------------------------------------------------

struct AxisDef {
    id_expr: &'static str,
    name_expr: &'static str,
    join_sql: &'static str,
}

fn get_axis_def(axis: &str, prefix: &str) -> Option<AxisDef> {
    match (axis, prefix) {
        ("STATES", "x") => Some(AxisDef {
            id_expr: "x_s.id::text",
            name_expr: "x_s.name",
            join_sql: "LEFT JOIN states x_s ON x_s.id = i.state_id",
        }),
        ("STATES", "g") => Some(AxisDef {
            id_expr: "g_s.id::text",
            name_expr: "g_s.name",
            join_sql: "LEFT JOIN states g_s ON g_s.id = i.state_id",
        }),
        ("STATE_GROUPS", "x") => Some(AxisDef {
            id_expr: "x_sg.group",
            name_expr: "x_sg.group",
            join_sql: "LEFT JOIN states x_sg ON x_sg.id = i.state_id",
        }),
        ("STATE_GROUPS", "g") => Some(AxisDef {
            id_expr: "g_sg.group",
            name_expr: "g_sg.group",
            join_sql: "LEFT JOIN states g_sg ON g_sg.id = i.state_id",
        }),
        ("LABELS", "x") => Some(AxisDef {
            id_expr: "x_l.id::text",
            name_expr: "x_l.name",
            join_sql: "JOIN issue_labels x_il ON x_il.issue_id = i.id AND x_il.deleted_at IS NULL JOIN labels x_l ON x_l.id = x_il.label_id",
        }),
        ("LABELS", "g") => Some(AxisDef {
            id_expr: "g_l.id::text",
            name_expr: "g_l.name",
            join_sql: "JOIN issue_labels g_il ON g_il.issue_id = i.id AND g_il.deleted_at IS NULL JOIN labels g_l ON g_l.id = g_il.label_id",
        }),
        ("ASSIGNEES", "x") => Some(AxisDef {
            id_expr: "x_u.id::text",
            name_expr: "x_u.display_name",
            join_sql: "JOIN issue_assignees x_ia ON x_ia.issue_id = i.id AND x_ia.deleted_at IS NULL JOIN users x_u ON x_u.id = x_ia.assignee_id",
        }),
        ("ASSIGNEES", "g") => Some(AxisDef {
            id_expr: "g_u.id::text",
            name_expr: "g_u.display_name",
            join_sql: "JOIN issue_assignees g_ia ON g_ia.issue_id = i.id AND g_ia.deleted_at IS NULL JOIN users g_u ON g_u.id = g_ia.assignee_id",
        }),
        ("ESTIMATE_POINTS", "x") => Some(AxisDef {
            id_expr: "x_ep.key::text",
            name_expr: "x_ep.value",
            join_sql: "JOIN estimate_points x_ep ON x_ep.id = i.estimate_point_id",
        }),
        ("ESTIMATE_POINTS", "g") => Some(AxisDef {
            id_expr: "g_ep.key::text",
            name_expr: "g_ep.value",
            join_sql: "JOIN estimate_points g_ep ON g_ep.id = i.estimate_point_id",
        }),
        ("CYCLES", "x") => Some(AxisDef {
            id_expr: "x_c.id::text",
            name_expr: "x_c.name",
            join_sql: "JOIN cycle_issues x_ci ON x_ci.issue_id = i.id AND x_ci.deleted_at IS NULL JOIN cycles x_c ON x_c.id = x_ci.cycle_id",
        }),
        ("CYCLES", "g") => Some(AxisDef {
            id_expr: "g_c.id::text",
            name_expr: "g_c.name",
            join_sql: "JOIN cycle_issues g_ci ON g_ci.issue_id = i.id AND g_ci.deleted_at IS NULL JOIN cycles g_c ON g_c.id = g_ci.cycle_id",
        }),
        ("MODULES", "x") => Some(AxisDef {
            id_expr: "x_m.id::text",
            name_expr: "x_m.name",
            join_sql: "JOIN module_issues x_mi ON x_mi.issue_id = i.id AND x_mi.deleted_at IS NULL JOIN modules x_m ON x_m.id = x_mi.module_id",
        }),
        ("MODULES", "g") => Some(AxisDef {
            id_expr: "g_m.id::text",
            name_expr: "g_m.name",
            join_sql: "JOIN module_issues g_mi ON g_mi.issue_id = i.id AND g_mi.deleted_at IS NULL JOIN modules g_m ON g_m.id = g_mi.module_id",
        }),
        ("PRIORITY", _) => Some(AxisDef {
            id_expr: "i.priority",
            name_expr: "i.priority",
            join_sql: "",
        }),
        ("START_DATE", _) => Some(AxisDef {
            id_expr: "i.start_date::text",
            name_expr: "i.start_date::text",
            join_sql: "",
        }),
        ("TARGET_DATE", _) => Some(AxisDef {
            id_expr: "i.target_date::text",
            name_expr: "i.target_date::text",
            join_sql: "",
        }),
        ("CREATED_AT", _) => Some(AxisDef {
            id_expr: "to_char(i.created_at::date, 'YYYY-MM-DD')",
            name_expr: "to_char(i.created_at::date, 'YYYY-MM-DD')",
            join_sql: "",
        }),
        ("COMPLETED_AT", _) => Some(AxisDef {
            id_expr: "to_char(i.completed_at::date, 'YYYY-MM-DD')",
            name_expr: "to_char(i.completed_at::date, 'YYYY-MM-DD')",
            join_sql: "",
        }),
        ("CREATED_BY", "x") => Some(AxisDef {
            id_expr: "x_cr.id::text",
            name_expr: "x_cr.display_name",
            join_sql: "LEFT JOIN users x_cr ON x_cr.id = i.created_by_id",
        }),
        ("CREATED_BY", "g") => Some(AxisDef {
            id_expr: "g_cr.id::text",
            name_expr: "g_cr.display_name",
            join_sql: "LEFT JOIN users g_cr ON g_cr.id = i.created_by_id",
        }),
        _ => None,
    }
}

async fn build_custom_chart(db: &sqlx::PgPool, base: &Base, x_axis: &str, group_by: Option<&str>) -> ApiResult {
    let x_def = get_axis_def(x_axis, "x").ok_or_else(|| ApiError::bad(format!("Invalid x_axis field: {x_axis}")))?;

    if let Some(g) = group_by {
        let g_def = get_axis_def(g, "g").ok_or_else(|| ApiError::bad(format!("Invalid group_by field: {g}")))?;

        let sql = format!(
            "SELECT {} AS key, {} AS display_name, {} AS group_key, {} AS group_name, count(DISTINCT i.id)::bigint AS count
             FROM issues i
             {}
             {}
             WHERE {}
             GROUP BY key, display_name, group_key, group_name
             ORDER BY count DESC",
            x_def.id_expr, x_def.name_expr, g_def.id_expr, g_def.name_expr,
            x_def.join_sql, g_def.join_sql, base.sql
        );
        let mut qq = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>, Option<String>, i64)>(&sql);
        for a in &base.args {
            qq = match a {
                Arg::Uuid(v) => qq.bind(*v),
                Arg::Uuids(v) => qq.bind(v.clone()),
                Arg::Text(v) => qq.bind(v.clone()),
                Arg::Texts(v) => qq.bind(v.clone()),
                Arg::Bool(v) => qq.bind(*v),
            };
        }
        let rows = qq.fetch_all(db).await?;

        let mut data_map: Vec<(String, String, i64, HashMap<String, i64>)> = Vec::new();
        let mut key_indexes: HashMap<String, usize> = HashMap::new();
        let mut schema = Map::new();

        for (k_opt, name_opt, gk_opt, gname_opt, c) in rows {
            let key = k_opt.unwrap_or_else(|| "none".to_string());
            let name = name_opt.unwrap_or_else(|| "None".to_string());
            let gk = gk_opt.unwrap_or_else(|| "none".to_string());
            let gname = gname_opt.unwrap_or_else(|| "None".to_string());

            schema.entry(gk.clone()).or_insert_with(|| Value::String(gname));

            if let Some(&idx) = key_indexes.get(&key) {
                data_map[idx].2 += c;
                *data_map[idx].3.entry(gk).or_insert(0) += c;
            } else {
                let idx = data_map.len();
                let mut groups = HashMap::new();
                groups.insert(gk, c);
                data_map.push((key.clone(), name, c, groups));
                key_indexes.insert(key, idx);
            }
        }

        let mut data = Vec::new();
        for (k, name, count_val, groups) in data_map {
            let mut obj = Map::new();
            obj.insert("key".into(), Value::String(k));
            obj.insert("name".into(), Value::String(name));
            obj.insert("count".into(), json!(count_val));
            for (gk, val) in groups {
                obj.insert(gk, json!(val));
            }
            data.push(Value::Object(obj));
        }

        Ok(Json(json!({ "data": data, "schema": schema })))
    } else {
        let sql = format!(
            "SELECT {} AS key, {} AS display_name, count(DISTINCT i.id)::bigint AS count
             FROM issues i
             {}
             WHERE {}
             GROUP BY key, display_name
             ORDER BY key NULLS LAST",
            x_def.id_expr, x_def.name_expr, x_def.join_sql, base.sql
        );
        let mut qq = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(&sql);
        for a in &base.args {
            qq = match a {
                Arg::Uuid(v) => qq.bind(*v),
                Arg::Uuids(v) => qq.bind(v.clone()),
                Arg::Text(v) => qq.bind(v.clone()),
                Arg::Texts(v) => qq.bind(v.clone()),
                Arg::Bool(v) => qq.bind(*v),
            };
        }
        let rows = qq.fetch_all(db).await?;
        let data: Vec<Value> = rows
            .into_iter()
            .map(|(k, name, c)| {
                json!({
                    "key": k.unwrap_or_else(|| "None".to_string()),
                    "name": name.unwrap_or_else(|| "None".to_string()),
                    "count": c
                })
            })
            .collect();
        Ok(Json(json!({ "data": data, "schema": {} })))
    }
}

// ---------------------------------------------------------------------------------------------
// Project stats
// ---------------------------------------------------------------------------------------------

async fn project_stats(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<Q>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let valid_fields = [
        "total_issues",
        "completed_issues",
        "total_members",
        "total_cycles",
        "total_modules",
    ];
    let requested: Vec<&str> = match qv(&q, "fields") {
        Some(f) => {
            let req: Vec<&str> = f
                .split(',')
                .map(str::trim)
                .filter(|x| valid_fields.contains(x))
                .collect();
            if req.is_empty() { valid_fields.to_vec() } else { req }
        }
        None => valid_fields.to_vec(),
    };

    let pids = project_ids_of(&q);

    let mut where_clause = String::from("p.workspace_id = $1 AND p.deleted_at IS NULL");
    if ws.role == GUEST {
        where_clause.push_str(" AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = p.id AND pm.member_id = $2 AND pm.is_active AND pm.deleted_at IS NULL)");
    } else if ws.role == MEMBER {
        where_clause.push_str(" AND (p.network = 2 OR EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = p.id AND pm.member_id = $2 AND pm.is_active AND pm.deleted_at IS NULL))");
    }

    let mut build_args = vec!["'id'", "p.id"];
    for f in &requested {
        match *f {
            "total_issues" => {
                build_args.push("'total_issues'");
                build_args.push("(SELECT count(*)::bigint FROM issues i WHERE i.project_id = p.id AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft AND NOT EXISTS (SELECT 1 FROM states ts WHERE ts.id = i.state_id AND ts.group = 'triage') AND p.archived_at IS NULL)");
            }
            "completed_issues" => {
                build_args.push("'completed_issues'");
                build_args.push("(SELECT count(*)::bigint FROM issues i JOIN states s ON s.id = i.state_id WHERE i.project_id = p.id AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft AND s.group IN ('completed', 'cancelled') AND p.archived_at IS NULL)");
            }
            "total_cycles" => {
                build_args.push("'total_cycles'");
                build_args.push("(SELECT count(*)::bigint FROM cycles c WHERE c.project_id = p.id AND c.deleted_at IS NULL AND c.archived_at IS NULL)");
            }
            "total_modules" => {
                build_args.push("'total_modules'");
                build_args.push("(SELECT count(*)::bigint FROM modules m WHERE m.project_id = p.id AND m.deleted_at IS NULL AND m.archived_at IS NULL)");
            }
            "total_members" => {
                build_args.push("'total_members'");
                build_args.push("(SELECT count(*)::bigint FROM project_members pm JOIN users u ON u.id = pm.member_id WHERE pm.project_id = p.id AND pm.is_active AND pm.deleted_at IS NULL AND NOT u.is_bot)");
            }
            _ => {}
        }
    }

    let json_expr = format!("jsonb_build_object({})", build_args.join(", "));
    let (sql, rows) = if let Some(ids) = pids {
        let sql =
            format!("SELECT {json_expr} FROM projects p WHERE {where_clause} AND p.id = ANY($3) ORDER BY p.created_at");
        let rows: Vec<Value> = sqlx::query_scalar(&sql)
            .bind(ws.workspace_id)
            .bind(user.id)
            .bind(ids)
            .fetch_all(&st.db)
            .await?;
        (sql, rows)
    } else {
        let sql = format!("SELECT {json_expr} FROM projects p WHERE {where_clause} ORDER BY p.created_at");
        let rows: Vec<Value> = sqlx::query_scalar(&sql)
            .bind(ws.workspace_id)
            .bind(user.id)
            .fetch_all(&st.db)
            .await?;
        (sql, rows)
    };
    let _ = sql;
    Ok(Json(Value::Array(rows)))
}

// ---------------------------------------------------------------------------------------------
// Legacy analytics & saved views
// ---------------------------------------------------------------------------------------------

const ANALYTIC_VIEW_JSON: &str = "jsonb_build_object(
    'id', v.id,
    'name', v.name,
    'description', v.description,
    'query', v.query,
    'query_dict', v.query_dict,
    'workspace', v.workspace_id,
    'workspace_id', v.workspace_id,
    'created_by', v.created_by_id,
    'updated_by', v.updated_by_id,
    'created_at', v.created_at,
    'updated_at', v.updated_at
)";

const VIEW_FIELDS: &[(&str, &str)] = &[("name", "text"), ("description", "text"), ("query_dict", "jsonb")];

const VALID_ANALYTICS_FIELDS: &[&str] = &[
    "state_id",
    "state__group",
    "labels__id",
    "assignees__id",
    "estimate_point__value",
    "issue_cycle__cycle_id",
    "issue_module__module_id",
    "priority",
    "start_date",
    "target_date",
    "created_at",
    "completed_at",
];

const VALID_YAXIS: &[&str] = &["issue_count", "estimate"];

struct PlotField {
    expr: &'static str,
    join_sql: &'static str,
}

fn get_plot_field(name: &str, alias: &str) -> Option<PlotField> {
    match (name, alias) {
        ("state_id", _) => Some(PlotField {
            expr: "i.state_id::text",
            join_sql: "",
        }),
        ("state__group", "x") => Some(PlotField {
            expr: "s_x.group",
            join_sql: "LEFT JOIN states s_x ON s_x.id = i.state_id",
        }),
        ("state__group", "seg") => Some(PlotField {
            expr: "s_seg.group",
            join_sql: "LEFT JOIN states s_seg ON s_seg.id = i.state_id",
        }),
        ("labels__id", "x") => Some(PlotField {
            expr: "l_x.id::text",
            join_sql: "JOIN issue_labels il_x ON il_x.issue_id = i.id AND il_x.deleted_at IS NULL JOIN labels l_x ON l_x.id = il_x.label_id",
        }),
        ("labels__id", "seg") => Some(PlotField {
            expr: "l_seg.id::text",
            join_sql: "JOIN issue_labels il_seg ON il_seg.issue_id = i.id AND il_seg.deleted_at IS NULL JOIN labels l_seg ON l_seg.id = il_seg.label_id",
        }),
        ("assignees__id", "x") => Some(PlotField {
            expr: "u_x.id::text",
            join_sql: "JOIN issue_assignees ia_x ON ia_x.issue_id = i.id AND ia_x.deleted_at IS NULL JOIN users u_x ON u_x.id = ia_x.assignee_id",
        }),
        ("assignees__id", "seg") => Some(PlotField {
            expr: "u_seg.id::text",
            join_sql: "JOIN issue_assignees ia_seg ON ia_seg.issue_id = i.id AND ia_seg.deleted_at IS NULL JOIN users u_seg ON u_seg.id = ia_seg.assignee_id",
        }),
        ("estimate_point__value", "x") => Some(PlotField {
            expr: "ep_x.value",
            join_sql: "JOIN estimate_points ep_x ON ep_x.id = i.estimate_point_id",
        }),
        ("estimate_point__value", "seg") => Some(PlotField {
            expr: "ep_seg.value",
            join_sql: "JOIN estimate_points ep_seg ON ep_seg.id = i.estimate_point_id",
        }),
        ("issue_cycle__cycle_id", "x") => Some(PlotField {
            expr: "ci_x.cycle_id::text",
            join_sql: "JOIN cycle_issues ci_x ON ci_x.issue_id = i.id AND ci_x.deleted_at IS NULL",
        }),
        ("issue_cycle__cycle_id", "seg") => Some(PlotField {
            expr: "ci_seg.cycle_id::text",
            join_sql: "JOIN cycle_issues ci_seg ON ci_seg.issue_id = i.id AND ci_seg.deleted_at IS NULL",
        }),
        ("issue_module__module_id", "x") => Some(PlotField {
            expr: "mi_x.module_id::text",
            join_sql: "JOIN module_issues mi_x ON mi_x.issue_id = i.id AND mi_x.deleted_at IS NULL",
        }),
        ("issue_module__module_id", "seg") => Some(PlotField {
            expr: "mi_seg.module_id::text",
            join_sql: "JOIN module_issues mi_seg ON mi_seg.issue_id = i.id AND mi_seg.deleted_at IS NULL",
        }),
        ("priority", _) => Some(PlotField {
            expr: "i.priority",
            join_sql: "",
        }),
        ("start_date", _) => Some(PlotField {
            expr: "to_char(i.start_date, 'YYYY-FMMM')",
            join_sql: "",
        }),
        ("target_date", _) => Some(PlotField {
            expr: "to_char(i.target_date, 'YYYY-FMMM')",
            join_sql: "",
        }),
        ("created_at", _) => Some(PlotField {
            expr: "to_char(i.created_at, 'YYYY-FMMM')",
            join_sql: "",
        }),
        ("completed_at", _) => Some(PlotField {
            expr: "to_char(i.completed_at, 'YYYY-FMMM')",
            join_sql: "",
        }),
        _ => None,
    }
}

async fn run_graph_plot(
    db: &sqlx::PgPool,
    base: &Base,
    x_axis: &str,
    y_axis: &str,
    segment: Option<&str>,
) -> ApiResult<Value> {
    if !VALID_ANALYTICS_FIELDS.contains(&x_axis) || !VALID_YAXIS.contains(&y_axis) {
        return Err(ApiError::bad_body(json!({
            "error": "x-axis and y-axis dimensions are required and the values should be valid"
        })));
    }
    if let Some(seg) = segment {
        if !VALID_ANALYTICS_FIELDS.contains(&seg) || seg == x_axis {
            return Err(ApiError::bad_body(json!({
                "error": "Both segment and x axis cannot be same and segment should be valid"
            })));
        }
    }

    let x_def = get_plot_field(x_axis, "x").ok_or_else(|| ApiError::bad("Invalid x_axis dimension"))?;

    let mut grouped_data: HashMap<String, Vec<Value>> = HashMap::new();

    if let Some(seg) = segment {
        let seg_def = get_plot_field(seg, "seg").ok_or_else(|| ApiError::bad("Invalid segment dimension"))?;

        if y_axis == "issue_count" {
            let sql = format!(
                "SELECT {} AS dimension, {} AS segment, count(*)::bigint AS count
                 FROM issues i
                 {}
                 {}
                 WHERE {} AND {} IS NOT NULL
                 GROUP BY dimension, segment
                 ORDER BY dimension",
                x_def.expr, seg_def.expr, x_def.join_sql, seg_def.join_sql, base.sql, x_def.expr
            );
            let mut qq = sqlx::query_as::<_, (String, Option<String>, i64)>(&sql);
            for a in &base.args {
                qq = match a {
                    Arg::Uuid(v) => qq.bind(*v),
                    Arg::Uuids(v) => qq.bind(v.clone()),
                    Arg::Text(v) => qq.bind(v.clone()),
                    Arg::Texts(v) => qq.bind(v.clone()),
                    Arg::Bool(v) => qq.bind(*v),
                };
            }
            let rows = qq.fetch_all(db).await?;
            for (dim, s, c) in rows {
                grouped_data.entry(dim.clone()).or_default().push(json!({
                    "dimension": dim,
                    "segment": s,
                    "count": c
                }));
            }
        } else {
            let sql = format!(
                "SELECT {} AS dimension, {} AS segment,
                        COALESCE(sum(CASE WHEN ep_est.value ~ '^-?[0-9]+(\\.[0-9]+)?$' THEN ep_est.value::float8 ELSE 0 END), 0)::float8 AS estimate
                 FROM issues i
                 {}
                 {}
                 LEFT JOIN estimate_points ep_est ON ep_est.id = i.estimate_point_id
                 WHERE {} AND {} IS NOT NULL
                 GROUP BY dimension, segment
                 ORDER BY dimension",
                x_def.expr, seg_def.expr, x_def.join_sql, seg_def.join_sql, base.sql, x_def.expr
            );
            let mut qq = sqlx::query_as::<_, (String, Option<String>, f64)>(&sql);
            for a in &base.args {
                qq = match a {
                    Arg::Uuid(v) => qq.bind(*v),
                    Arg::Uuids(v) => qq.bind(v.clone()),
                    Arg::Text(v) => qq.bind(v.clone()),
                    Arg::Texts(v) => qq.bind(v.clone()),
                    Arg::Bool(v) => qq.bind(*v),
                };
            }
            let rows = qq.fetch_all(db).await?;
            for (dim, s, est) in rows {
                grouped_data.entry(dim.clone()).or_default().push(json!({
                    "dimension": dim,
                    "segment": s,
                    "estimate": est
                }));
            }
        }
    } else if y_axis == "issue_count" {
        let sql = format!(
            "SELECT {} AS dimension, count(*)::bigint AS count
             FROM issues i
             {}
             WHERE {} AND {} IS NOT NULL
             GROUP BY dimension
             ORDER BY dimension",
            x_def.expr, x_def.join_sql, base.sql, x_def.expr
        );
        let mut qq = sqlx::query_as::<_, (String, i64)>(&sql);
        for a in &base.args {
            qq = match a {
                Arg::Uuid(v) => qq.bind(*v),
                Arg::Uuids(v) => qq.bind(v.clone()),
                Arg::Text(v) => qq.bind(v.clone()),
                Arg::Texts(v) => qq.bind(v.clone()),
                Arg::Bool(v) => qq.bind(*v),
            };
        }
        let rows = qq.fetch_all(db).await?;
        for (dim, c) in rows {
            grouped_data.entry(dim.clone()).or_default().push(json!({
                "dimension": dim,
                "count": c
            }));
        }
    } else {
        let sql = format!(
            "SELECT {} AS dimension,
                    COALESCE(sum(CASE WHEN ep_est.value ~ '^-?[0-9]+(\\.[0-9]+)?$' THEN ep_est.value::float8 ELSE 0 END), 0)::float8 AS estimate
             FROM issues i
             {}
             LEFT JOIN estimate_points ep_est ON ep_est.id = i.estimate_point_id
             WHERE {} AND {} IS NOT NULL
             GROUP BY dimension
             ORDER BY dimension",
            x_def.expr, x_def.join_sql, base.sql, x_def.expr
        );
        let mut qq = sqlx::query_as::<_, (String, f64)>(&sql);
        for a in &base.args {
            qq = match a {
                Arg::Uuid(v) => qq.bind(*v),
                Arg::Uuids(v) => qq.bind(v.clone()),
                Arg::Text(v) => qq.bind(v.clone()),
                Arg::Texts(v) => qq.bind(v.clone()),
                Arg::Bool(v) => qq.bind(*v),
            };
        }
        let rows = qq.fetch_all(db).await?;
        for (dim, est) in rows {
            grouped_data.entry(dim.clone()).or_default().push(json!({
                "dimension": dim,
                "estimate": est
            }));
        }
    }

    let mut out = Map::new();
    if x_axis == "priority" {
        for p in ["low", "medium", "high", "urgent", "none"] {
            if let Some(items) = grouped_data.remove(p) {
                out.insert(p.to_string(), Value::Array(items));
            }
        }
    } else {
        let mut keys: Vec<String> = grouped_data.keys().cloned().collect();
        keys.sort_by(|a, b| (a == "none", a).cmp(&(b == "none", b)));
        for k in keys {
            if let Some(items) = grouped_data.remove(&k) {
                out.insert(k, Value::Array(items));
            }
        }
    }

    Ok(Value::Object(out))
}

async fn analytics(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<Q>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;

    let x_axis = qv(&q, "x_axis").unwrap_or_default();
    let y_axis = qv(&q, "y_axis").unwrap_or_default();
    let segment = qv(&q, "segment");

    let mut base = issue_base(ws.workspace_id, user.id, &q, None);
    crate::workitems::apply_filters(&mut base, &q);

    let distribution = run_graph_plot(&st.db, &base, x_axis, y_axis, segment).await?;
    let total = count(&st.db, "issues i", &base, "TRUE").await?;

    let state_details = if x_axis == "state_id" || segment == Some("state_id") {
        let sql = format!(
            "SELECT jsonb_build_object('state_id', s.id, 'state__name', s.name, 'state__color', s.color)
             FROM issues i
             JOIN states s ON s.id = i.state_id
             WHERE {}
             GROUP BY s.id, s.name, s.color
             ORDER BY s.id",
            base.sql
        );
        Value::Array(fetch_json_rows(&st.db, &sql, &base).await?)
    } else {
        json!({})
    };

    let label_details = if x_axis == "labels__id" || segment == Some("labels__id") {
        let sql = format!(
            "SELECT jsonb_build_object('labels__id', l.id, 'labels__color', l.color, 'labels__name', l.name)
             FROM issues i
             JOIN issue_labels il ON il.issue_id = i.id AND il.deleted_at IS NULL
             JOIN labels l ON l.id = il.label_id
             WHERE {}
             GROUP BY l.id, l.color, l.name
             ORDER BY l.id",
            base.sql
        );
        Value::Array(fetch_json_rows(&st.db, &sql, &base).await?)
    } else {
        json!({})
    };

    let assignee_details = if x_axis == "assignees__id" || segment == Some("assignees__id") {
        let sql = format!(
            "SELECT jsonb_build_object(
                'assignees__id', u.id,
                'assignees__display_name', u.display_name,
                'assignees__first_name', u.first_name,
                'assignees__last_name', u.last_name,
                'assignees__avatar_url', CASE
                    WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id::text || '/'
                    ELSE NULLIF(u.avatar, '')
                END
             )
             FROM issues i
             JOIN issue_assignees ia ON ia.issue_id = i.id AND ia.deleted_at IS NULL
             JOIN users u ON u.id = ia.assignee_id
             WHERE {} AND (u.avatar != '' OR u.avatar_asset_id IS NOT NULL)
             GROUP BY u.id, u.display_name, u.first_name, u.last_name, u.avatar_asset_id, u.avatar
             ORDER BY u.id",
            base.sql
        );
        Value::Array(fetch_json_rows(&st.db, &sql, &base).await?)
    } else {
        json!({})
    };

    let cycle_details = if x_axis == "issue_cycle__cycle_id" || segment == Some("issue_cycle__cycle_id") {
        let sql = format!(
            "SELECT jsonb_build_object('issue_cycle__cycle_id', c.id, 'issue_cycle__cycle__name', c.name)
             FROM issues i
             JOIN cycle_issues ci ON ci.issue_id = i.id AND ci.deleted_at IS NULL
             JOIN cycles c ON c.id = ci.cycle_id
             WHERE {}
             GROUP BY c.id, c.name
             ORDER BY c.id",
            base.sql
        );
        Value::Array(fetch_json_rows(&st.db, &sql, &base).await?)
    } else {
        json!({})
    };

    let module_details = if x_axis == "issue_module__module_id" || segment == Some("issue_module__module_id") {
        let sql = format!(
            "SELECT jsonb_build_object('issue_module__module_id', m.id, 'issue_module__module__name', m.name)
             FROM issues i
             JOIN module_issues mi ON mi.issue_id = i.id AND mi.deleted_at IS NULL
             JOIN modules m ON m.id = mi.module_id
             WHERE {}
             GROUP BY m.id, m.name
             ORDER BY m.id",
            base.sql
        );
        Value::Array(fetch_json_rows(&st.db, &sql, &base).await?)
    } else {
        json!({})
    };

    Ok(Json(json!({
        "total": total,
        "distribution": distribution,
        "extras": {
            "state_details": state_details,
            "assignee_details": assignee_details,
            "label_details": label_details,
            "cycle_details": cycle_details,
            "module_details": module_details
        }
    })))
}

async fn default_analytics(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<Q>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let mut base = issue_base(ws.workspace_id, user.id, &q, None);
    crate::workitems::apply_filters(&mut base, &q);

    let total_issues = count(&st.db, "issues i", &base, "TRUE").await?;

    let sql_classified = format!(
        "SELECT jsonb_build_object('state_group', s.group, 'state_count', count(*)::bigint)
         FROM issues i
         LEFT JOIN states s ON s.id = i.state_id
         WHERE {}
         GROUP BY s.group
         ORDER BY s.group NULLS LAST",
        base.sql
    );
    let total_issues_classified = fetch_json_rows(&st.db, &sql_classified, &base).await?;

    let open_extra =
        "EXISTS (SELECT 1 FROM states s WHERE s.id = i.state_id AND s.group IN ('backlog', 'unstarted', 'started'))";
    let open_issues = count(&st.db, "issues i", &base, open_extra).await?;

    let sql_open_classified = format!(
        "SELECT jsonb_build_object('state_group', s.group, 'state_count', count(*)::bigint)
         FROM issues i
         LEFT JOIN states s ON s.id = i.state_id
         WHERE {} AND s.group IN ('backlog', 'unstarted', 'started')
         GROUP BY s.group
         ORDER BY s.group NULLS LAST",
        base.sql
    );
    let open_issues_classified = fetch_json_rows(&st.db, &sql_open_classified, &base).await?;

    let current_year = Utc::now().year();
    let sql_month_wise = format!(
        "SELECT jsonb_build_object('month', EXTRACT(month FROM i.completed_at)::integer, 'count', count(*)::bigint)
         FROM issues i
         WHERE {} AND EXTRACT(year FROM i.completed_at) = {current_year}
         GROUP BY EXTRACT(month FROM i.completed_at)
         ORDER BY EXTRACT(month FROM i.completed_at)",
        base.sql
    );
    let issue_completed_month_wise = fetch_json_rows(&st.db, &sql_month_wise, &base).await?;

    let sql_created_user = format!(
        "SELECT jsonb_build_object(
            'created_by__first_name', u.first_name,
            'created_by__last_name', u.last_name,
            'created_by__display_name', u.display_name,
            'created_by__id', u.id,
            'created_by__avatar_url', CASE
                WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id::text || '/'
                ELSE NULLIF(u.avatar, '')
            END,
            'count', count(*)::bigint
         )
         FROM issues i
         JOIN users u ON u.id = i.created_by_id
         WHERE {} AND i.created_by_id IS NOT NULL
         GROUP BY u.id, u.first_name, u.last_name, u.display_name, u.avatar_asset_id, u.avatar
         ORDER BY count(*) DESC
         LIMIT 5",
        base.sql
    );
    let most_issue_created_user = fetch_json_rows(&st.db, &sql_created_user, &base).await?;

    let sql_closed_user = format!(
        "SELECT jsonb_build_object(
            'assignees__first_name', u.first_name,
            'assignees__last_name', u.last_name,
            'assignees__display_name', u.display_name,
            'assignees__id', u.id,
            'assignees__avatar_url', CASE
                WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id::text || '/'
                ELSE NULLIF(u.avatar, '')
            END,
            'count', count(DISTINCT i.id)::bigint
         )
         FROM issues i
         JOIN issue_assignees ia ON ia.issue_id = i.id AND ia.deleted_at IS NULL
         JOIN users u ON u.id = ia.assignee_id
         WHERE {} AND i.completed_at IS NOT NULL
         GROUP BY u.id, u.first_name, u.last_name, u.display_name, u.avatar_asset_id, u.avatar
         ORDER BY count(DISTINCT i.id) DESC
         LIMIT 5",
        base.sql
    );
    let most_issue_closed_user = fetch_json_rows(&st.db, &sql_closed_user, &base).await?;

    let sql_pending_user = format!(
        "SELECT jsonb_build_object(
            'assignees__first_name', u.first_name,
            'assignees__last_name', u.last_name,
            'assignees__display_name', u.display_name,
            'assignees__id', u.id,
            'assignees__avatar_url', CASE
                WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id::text || '/'
                ELSE NULLIF(u.avatar, '')
            END,
            'count', count(DISTINCT i.id)::bigint
         )
         FROM issues i
         JOIN issue_assignees ia ON ia.issue_id = i.id AND ia.deleted_at IS NULL
         JOIN users u ON u.id = ia.assignee_id
         WHERE {} AND i.completed_at IS NULL
         GROUP BY u.id, u.first_name, u.last_name, u.display_name, u.avatar_asset_id, u.avatar
         ORDER BY count(DISTINCT i.id) DESC",
        base.sql
    );
    let pending_issue_user = fetch_json_rows(&st.db, &sql_pending_user, &base).await?;

    let sql_open_est = format!(
        "SELECT sum(i.point)::float8 FROM issues i LEFT JOIN states s ON s.id = i.state_id WHERE {} AND s.group IN ('backlog', 'unstarted', 'started')",
        base.sql
    );
    let mut qq_open = sqlx::query_scalar::<_, Option<f64>>(&sql_open_est);
    for a in &base.args {
        qq_open = match a {
            Arg::Uuid(v) => qq_open.bind(*v),
            Arg::Uuids(v) => qq_open.bind(v.clone()),
            Arg::Text(v) => qq_open.bind(v.clone()),
            Arg::Texts(v) => qq_open.bind(v.clone()),
            Arg::Bool(v) => qq_open.bind(*v),
        };
    }
    let open_estimate_sum: Option<f64> = qq_open.fetch_one(&st.db).await?;

    let sql_tot_est = format!("SELECT sum(i.point)::float8 FROM issues i WHERE {}", base.sql);
    let mut qq_tot = sqlx::query_scalar::<_, Option<f64>>(&sql_tot_est);
    for a in &base.args {
        qq_tot = match a {
            Arg::Uuid(v) => qq_tot.bind(*v),
            Arg::Uuids(v) => qq_tot.bind(v.clone()),
            Arg::Text(v) => qq_tot.bind(v.clone()),
            Arg::Texts(v) => qq_tot.bind(v.clone()),
            Arg::Bool(v) => qq_tot.bind(*v),
        };
    }
    let total_estimate_sum: Option<f64> = qq_tot.fetch_one(&st.db).await?;

    Ok(Json(json!({
        "total_issues": total_issues,
        "total_issues_classified": total_issues_classified,
        "open_issues": open_issues,
        "open_issues_classified": open_issues_classified,
        "issue_completed_month_wise": issue_completed_month_wise,
        "most_issue_created_user": most_issue_created_user,
        "most_issue_closed_user": most_issue_closed_user,
        "pending_issue_user": pending_issue_user,
        "open_estimate_sum": open_estimate_sum,
        "total_estimate_sum": total_estimate_sum,
    })))
}

async fn view_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {ANALYTIC_VIEW_JSON} FROM analytic_views v WHERE v.workspace_id = $1 AND v.deleted_at IS NULL ORDER BY v.created_at DESC"
        ))
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

async fn view_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;

    let name = s(&b, "name")
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| ApiError::bad_body(json!({"name": ["This field is required."]})))?;
    let description = s(&b, "description").unwrap_or("");
    let query_dict = b.get("query_dict").cloned().unwrap_or_else(|| json!({}));

    let row: Value = sqlx::query_scalar(&format!(
        "INSERT INTO analytic_views AS v (id, name, description, query, query_dict, workspace_id, created_by_id, updated_by_id, created_at, updated_at)
         VALUES (gen_random_uuid(), $1, $2, '{{}}'::jsonb, $3, $4, $5, $5, now(), now())
         RETURNING {ANALYTIC_VIEW_JSON}"
    ))
    .bind(name)
    .bind(description)
    .bind(query_dict)
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;

    created(row)
}

async fn view_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {ANALYTIC_VIEW_JSON} FROM analytic_views v WHERE v.id = $1 AND v.workspace_id = $2 AND v.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

async fn view_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;

    let mut sets = crate::users::patch_sets(&b, VIEW_FIELDS, 3);
    if sets.is_empty() {
        return view_retrieve(State(st), user, Path((slug, pk))).await;
    }
    sets.push("updated_at = now()".into());
    sets.push("updated_by_id = $2".into());

    let sql = format!(
        "UPDATE analytic_views v SET {} WHERE v.id = $1 AND v.workspace_id = $4 AND v.deleted_at IS NULL RETURNING {ANALYTIC_VIEW_JSON}",
        sets.join(", ")
    );
    json_row(
        sqlx::query_scalar(&sql)
            .bind(pk)
            .bind(user.id)
            .bind(&b)
            .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

async fn view_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;

    let res = sqlx::query(
        "UPDATE analytic_views SET deleted_at = now() WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(ws.workspace_id)
    .execute(&st.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn saved_view_run(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, analytic_id)): Path<(String, Uuid)>,
    Query(q): Query<Q>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;

    let row: Option<(Value,)> = sqlx::query_as(
        "SELECT v.query_dict FROM analytic_views v WHERE v.id = $1 AND v.workspace_id = $2 AND v.deleted_at IS NULL",
    )
    .bind(analytic_id)
    .bind(ws.workspace_id)
    .fetch_optional(&st.db)
    .await?;

    let (query_dict,) = row.ok_or_else(ApiError::not_found)?;

    let x_axis_str = query_dict
        .get("x_axis")
        .and_then(Value::as_str)
        .or_else(|| qv(&q, "x_axis"))
        .unwrap_or("priority");
    let y_axis_str = query_dict
        .get("y_axis")
        .and_then(Value::as_str)
        .or_else(|| qv(&q, "y_axis"))
        .unwrap_or("issue_count");
    let segment = qv(&q, "segment").or_else(|| query_dict.get("segment").and_then(Value::as_str));

    let mut base = issue_base(ws.workspace_id, user.id, &q, None);
    crate::workitems::apply_filters(&mut base, &q);

    if let Value::Object(dict_map) = &query_dict {
        let mut string_map = HashMap::new();
        for (k, v) in dict_map {
            if let Some(s) = v.as_str() {
                string_map.insert(k.clone(), s.to_string());
            }
        }
        crate::workitems::apply_filters(&mut base, &string_map);
    }

    let distribution = run_graph_plot(&st.db, &base, x_axis_str, y_axis_str, segment).await?;
    let total = count(&st.db, "issues i", &base, "TRUE").await?;

    Ok(Json(json!({
        "total": total,
        "distribution": distribution
    })))
}
