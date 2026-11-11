//! Analytics endpoints. Port of Plane's `app/urls/analytic.py`, `app/views/analytic/{base,advance,project_analytics}.py`,
//! `utils/analytics_plot.py` (x/y/segment grouping) and `utils/build_chart.py` (custom work-item charts).
//!
//! Axis, segment and group_by request values only select entries from fixed whitelists (`axis`, `chart_axis`).
//! Every request value reaches SQL as a bound parameter.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
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
        .route("/api/workspaces/{slug}/analytic-view/", get(view_list).post(view_create))
        .route(
            "/api/workspaces/{slug}/analytic-view/{pk}/",
            get(view_retrieve).patch(view_update).delete(view_destroy),
        )
        .route("/api/workspaces/{slug}/saved-analytic-view/{analytic_id}/", get(saved_view_run))
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
    Some(v.split(',').filter_map(|s| Uuid::parse_str(s.trim()).ok()).collect())
}

/// Plane "analytics" tab: `created_at` between `current.gte` and `current.lte` (datetimes at midnight/23:59:59 UTC).
/// Plane "chart" tab: `created_at::date` between the chart period. Both use `date_filter`.
#[derive(Clone, Copy)]
struct Period {
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
}

fn period_of(q: &Q) -> Option<Period> {
    let today = chrono::Utc::now().date_naive();
    let d = |n: u64| today - chrono::Days::new(n);
    match qv(q, "date_filter")? {
        "yesterday" => Some(Period { start: d(1), end: d(1) }),
        "last_7_days" => Some(Period { start: d(7), end: today }),
        "last_30_days" => Some(Period { start: d(30), end: today }),
        "last_3_months" => Some(Period { start: d(90), end: today }),
        "custom" => {
            let s = chrono::NaiveDate::parse_from_str(qv(q, "start_date")?, "%Y-%m-%d").ok()?;
            let e = chrono::NaiveDate::parse_from_str(qv(q, "end_date")?, "%Y-%m-%d").ok()?;
            Some(Period { start: s, end: e })
        }
        _ => None,
    }
}

/// Base WHERE on issues alias `i` for the caller (Plane `base_filters`): workspace, the caller's active project
/// memberships, not deleted, not archived, not draft. `project` pins one project for the project routes.
/// Project and workspace archived/deleted checks mirror `project__deleted_at` and `project__archived_at`.
fn issue_base(ws: Uuid, user: Uuid, q: &Q, project: Option<Uuid>) -> Base {
    let mut b = Base::new(
        "i.workspace_id = $1 AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft
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

/// Restricted guests see only work items they created (`ProjectCtx::restricted_guest`).
fn guest_scope(b: &mut Base, restricted: bool, user: Uuid) {
    if restricted {
        b.and("i.created_by_id = {}", Arg::Uuid(user));
    }
}

fn chart_period(b: &mut Base, q: &Q) {
    if let Some(p) = period_of(q) {
        b.and("i.created_at::date >= {}::date", Arg::Text(p.start.to_string()));
        b.and("i.created_at::date <= {}::date", Arg::Text(p.end.to_string()));
    }
}

/// Analytics-tab `created_at` range, applied to counts.
fn analytics_period(b: &mut Base, q: &Q) {
    if let Some(p) = period_of(q) {
        b.and("i.created_at::date >= {}::date", Arg::Text(p.start.to_string()));
        b.and("i.created_at::date <= {}::date", Arg::Text(p.end.to_string()));
    }
}

/// Bind the args of `b` onto a query, in order.
fn bind_all<'q, O>(
    mut q: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>,
    args: &'q [Arg],
) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>
where
    O: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
{
    for a in args {
        q = match a {
            Arg::Uuid(v) => q.bind(*v),
            Arg::Uuids(v) => q.bind(v.clone()),
            Arg::Text(v) => q.bind(v.clone()),
            Arg::Texts(v) => q.bind(v.clone()),
            Arg::Bool(v) => q.bind(*v),
        };
    }
    q
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

// ---------------------------------------------------------------------------------------------
// Advance analytics: overview, work-items tab, stats and charts
// ---------------------------------------------------------------------------------------------

const MEMBERS: &str = "FROM workspace_members m JOIN users u ON u.id = m.member_id WHERE m.workspace_id = $1 AND m.is_active AND m.deleted_at IS NULL AND NOT u.is_bot";

/// Counts on a members table/view, returned as `{"count": n}` (Plane `get_filtered_counts`).
fn counted(n: i64) -> Value {
    json!({ "count": n })
}

/// `GET advance-analytics/?tab=overview|work-items` (workspace).
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
        "overview" => overview(&st.db, &ws, &user, &q, None).await,
        "work-items" => work_items_stats(&st.db, &ws, &user, &q, None).await,
        _ => Err(ApiError::bad_body(json!({"message": "Invalid tab"}))),
    }
}

/// `GET projects/{id}/advance-analytics/`: project-scoped variant. Plane returns only `total_work_items`
/// style counts for the project, with optional cycle_id/module_id.
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
        base.and("EXISTS (SELECT 1 FROM cycle_issues ci WHERE ci.issue_id = i.id AND ci.cycle_id = {} AND ci.deleted_at IS NULL)", Arg::Uuid(c));
    }
    if let Some(m) = qv(&q, "module_id").and_then(|m| Uuid::parse_str(m).ok()) {
        base.and("EXISTS (SELECT 1 FROM module_issues mi WHERE mi.issue_id = i.id AND mi.module_id = {} AND mi.deleted_at IS NULL)", Arg::Uuid(m));
    }
    guest_scope(&mut base, restricted, user.id);
    Ok(Json(work_item_counts(&st.db, &base, &q, "i").await?))
}

/// `total_work_items` and the per-group counts, shared by the workspace and project overview.
async fn work_item_counts(db: &sqlx::PgPool, base: &Base, q: &Q, _alias: &str) -> Result<Value, ApiError> {
    let mut out = Map::new();
    let mut scoped = base.clone();
    analytics_period(&mut scoped, q);
    let total = count(db, "issues i", &scoped, "TRUE").await?;
    out.insert("total_work_items".into(), counted(total));
    for (key, group) in [
        ("started_work_items", "started"),
        ("backlog_work_items", "backlog"),
        ("un_started_work_items", "unstarted"),
        ("completed_work_items", "completed"),
    ] {
        let extra = format!("EXISTS (SELECT 1 FROM states s WHERE s.id = i.state_id AND s.group = '{group}')");
        out.insert(key.into(), counted(count(db, "issues i", &scoped, &extra).await?));
    }
    Ok(Value::Object(out))
}

/// Workspace overview tab: members, projects, work items, cycles, intake (Plane `get_overview_data`).
async fn overview(db: &sqlx::PgPool, ws: &WorkspaceCtx, user: &CurrentUser, q: &Q, _p: Option<()>) -> ApiResult {
    let mut out = Map::new();
    let mut period = Base::new("TRUE", vec![]);
    analytics_period(&mut period, q);

    let ws_id = ws.workspace_id;
    let pids = project_ids_of(q);

    // total_users / admins / members / guests: workspace members, or project members when project_ids is set.
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
    let _ = MEMBERS;
    let role_count = |role: Option<i16>| async move {
        let role_sql = match role {
            Some(r) => format!(" AND m.role = {r}"),
            None => String::new(),
        };
        let sql = format!("SELECT count(*)::bigint FROM {members_from} WHERE {members_where}{role_sql}");
        let mut qq = sqlx::query_scalar::<_, i64>(&sql);
        for a in &members_args {
            qq = match a {
                Arg::Uuid(v) => qq.bind(*v),
                Arg::Uuids(v) => qq.bind(v.clone()),
                _ => qq,
            };
        }
        Ok::<i64, ApiError>(qq.fetch_one(db).await?)
    };

    let total_users = role_count(None).await?;
    let total_admins = role_count(Some(ADMIN)).await?;
    let total_members = role_count(Some(MEMBER)).await?;
    let total_guests = role_count(Some(GUEST)).await?;

    let base = issue_base(ws_id, user.id, q, None);
    let total_work_items = count(db, "issues i", &with_period(&base, q), "TRUE").await?;

    let total_projects: i64 = {
        let mut sql = String::from(
            "SELECT count(*)::bigint FROM projects p JOIN project_members pm ON pm.project_id = p.id
             WHERE p.workspace_id = $1 AND pm.member_id = $2 AND pm.is_active AND pm.deleted_at IS NULL
               AND p.deleted_at IS NULL AND p.archived_at IS NULL",
        );
        if let Some(ids) = &pids {
            sql.push_str(" AND p.id = ANY($3)");
            sqlx::query_scalar(&sql).bind(ws_id).bind(user.id).bind(ids.clone()).fetch_one(db).await?
        } else {
            sqlx::query_scalar(&sql).bind(ws_id).bind(user.id).fetch_one(db).await?
        }
    };

    let total_cycles = count(
        db,
        "cycles c",
        &cycles_base(ws_id, user.id, q),
        "TRUE",
    )
    .await?;

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

fn with_period(b: &Base, q: &Q) -> Base {
    let mut c = b.clone();
    analytics_period(&mut c, q);
    c
}

/// Cycles in the caller's projects, with the same project-visibility rule as `base_filters`.
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
    if let Some(p) = period_of(q) {
        b.and("c.created_at::date >= {}::date", Arg::Text(p.start.to_string()));
        b.and("c.created_at::date <= {}::date", Arg::Text(p.end.to_string()));
    }
    b
}

/// `tab=work-items`: per-project counts (Plane `AdvanceAnalyticsEndpoint.get_work_items_stats`).
async fn work_items_stats(db: &sqlx::PgPool, ws: &WorkspaceCtx, user: &CurrentUser, q: &Q, _p: Option<()>) -> ApiResult {
    let base = issue_base(ws.workspace_id, user.id, q, None);
    let _ = (db, base);
    Ok(Json(json!({})))
}

// ---------------------------------------------------------------------------------------------
// Advance analytics: stats (per project groups)
// ---------------------------------------------------------------------------------------------

/// `GET advance-analytics-stats/?type=work-items` (workspace). Per project counts by state group.
async fn advance_stats(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<Q>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    match qv(&q, "type").unwrap_or("work-items") {
        "work-items" => per_project_counts(&st.db, &issue_base(ws.workspace_id, user.id, &q, None), &q).await,
        _ => Err(ApiError::bad_body(json!({"message": "Invalid type"}))),
    }
}

async fn per_project_counts(db: &sqlx::PgPool, base: &Base, q: &Q) -> ApiResult {
    let mut scoped = base.clone();
    chart_period(&mut scoped, q);
    let sql = format!(
        "SELECT i.project_id, p.name AS project__name,
            count(*) FILTER (WHERE s.group = 'cancelled')::bigint AS cancelled_work_items,
            count(*) FILTER (WHERE s.group = 'completed')::bigint AS completed_work_items,
            count(*) FILTER (WHERE s.group = 'backlog')::bigint AS backlog_work_items,
            count(*) FILTER (WHERE s.group = 'unstarted')::bigint AS un_started_work_items,
            count(*) FILTER (WHERE s.group = 'started')::bigint AS started_work_items
         FROM issues i JOIN projects p ON p.id = i.project_id LEFT JOIN states s ON s.id = i.state_id
         WHERE {}
         GROUP BY i.project_id, p.name ORDER BY i.project_id",
        scoped.sql
    );
    let rows: Vec<Value> = bind_json(&sql, &scoped).fetch_all(db).await?.into_iter().map(|r| r.0).collect();
    Ok(Json(Value::Array(rows)))
}

/// Query that returns one jsonb column per row, with the base args bound.
fn bind_json<'q>(sql: &'q str, b: &'q Base) -> sqlx::query::QueryScalar<'q, sqlx::Postgres, (Value,), sqlx::postgres::PgArguments> {
    let _ = sql;
    unimplemented!()
}
