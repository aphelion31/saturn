//! Shared work-item (issue) list engine: Plane's `issue_on_results` row shape, legacy query filters,
//! `order_by`, `group_by` / `sub_group_by` and cursor pagination (`per_page`, `cursor=<limit>:<page>:<is_prev>`).
//!
//! Used by every endpoint that lists work items: project issues, cycle issues, module issues, views,
//! workspace views, archived and draft lists. Callers pass a base WHERE clause on alias `i` (issues)
//! with their own bound params; this module appends filters with further bound params.
//!
//! ```ignore
//! let base = Base::new("i.project_id = $1 AND i.workspace_id = $2", vec![Arg::Uuid(project_id), Arg::Uuid(ws)]);
//! workitems::list(&st.db, base, &q, Scope::Project(project_id), ws).await
//! ```
use std::collections::HashMap;

use axum::Json;
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::*;

/// One row of a work-item list (`issue_on_results` fields plus the m2m id arrays).
pub const ITEM_JSON: &str = "jsonb_build_object(
    'id', i.id, 'name', i.name, 'state_id', i.state_id, 'sort_order', i.sort_order, 'completed_at', i.completed_at,
    'estimate_point', i.estimate_point_id, 'priority', i.priority, 'start_date', i.start_date,
    'target_date', i.target_date, 'sequence_id', i.sequence_id, 'project_id', i.project_id,
    'parent_id', i.parent_id, 'type_id', i.type_id,
    'cycle_id', (SELECT ci.cycle_id FROM cycle_issues ci WHERE ci.issue_id = i.id AND ci.deleted_at IS NULL LIMIT 1),
    'sub_issues_count', (SELECT count(*) FROM issues c WHERE c.parent_id = i.id AND c.deleted_at IS NULL
                         AND c.archived_at IS NULL AND NOT c.is_draft),
    'created_at', i.created_at, 'updated_at', i.updated_at, 'created_by', i.created_by_id, 'updated_by', i.updated_by_id,
    'attachment_count', (SELECT count(*) FROM file_assets fa WHERE fa.issue_id = i.id AND fa.entity_type = 'ISSUE_ATTACHMENT'
                         AND fa.deleted_at IS NULL),
    'link_count', (SELECT count(*) FROM issue_links il WHERE il.issue_id = i.id AND il.deleted_at IS NULL),
    'is_draft', i.is_draft, 'archived_at', i.archived_at,
    'state__group', (SELECT s.group FROM states s WHERE s.id = i.state_id),
    'assignee_ids', COALESCE((SELECT jsonb_agg(a.assignee_id) FROM issue_assignees a JOIN users u ON u.id = a.assignee_id
                              WHERE a.issue_id = i.id AND a.deleted_at IS NULL AND u.is_active), '[]'::jsonb),
    'label_ids', COALESCE((SELECT jsonb_agg(l.label_id) FROM issue_labels l WHERE l.issue_id = i.id AND l.deleted_at IS NULL),
                          '[]'::jsonb),
    'module_ids', COALESCE((SELECT jsonb_agg(m.module_id) FROM module_issues m WHERE m.issue_id = i.id AND m.deleted_at IS NULL),
                           '[]'::jsonb))";

/// Plane's `Issue.issue_objects` manager: not deleted, not archived, not draft, not triage, project not archived.
pub const ACTIVE: &str = "i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft
    AND NOT EXISTS (SELECT 1 FROM states ts WHERE ts.id = i.state_id AND ts.group = 'triage')
    AND EXISTS (SELECT 1 FROM projects pp WHERE pp.id = i.project_id AND pp.archived_at IS NULL AND pp.deleted_at IS NULL)";

/// Bound parameter value.
#[derive(Clone, Debug)]
pub enum Arg {
    Uuid(Uuid),
    Uuids(Vec<Uuid>),
    Text(String),
    Texts(Vec<String>),
    Bool(bool),
}

/// A WHERE clause on alias `i` with its params, numbered from `$1`.
#[derive(Clone, Debug)]
pub struct Base {
    pub sql: String,
    pub args: Vec<Arg>,
}

impl Base {
    pub fn new(sql: impl Into<String>, args: Vec<Arg>) -> Self {
        Self { sql: sql.into(), args }
    }
    /// Append `AND <sql>` where `sql` uses `{}` as the placeholder for one new param.
    pub fn and(&mut self, sql: &str, arg: Arg) {
        self.args.push(arg);
        let p = format!("${}", self.args.len());
        self.sql = format!("{} AND {}", self.sql, sql.replace("{}", &p));
    }
    pub fn and_raw(&mut self, sql: &str) {
        self.sql = format!("{} AND {}", self.sql, sql);
    }
}

/// Which projects group values (states, labels, members, ...) come from.
#[derive(Clone, Copy, Debug)]
pub enum Scope {
    Project(Uuid),
    /// All projects the user is an active member of in this workspace.
    Workspace {
        workspace_id: Uuid,
        user_id: Uuid,
    },
}

fn bind<'q>(
    mut q: sqlx::query::QueryScalar<'q, sqlx::Postgres, Value, sqlx::postgres::PgArguments>,
    args: &'q [Arg],
) -> sqlx::query::QueryScalar<'q, sqlx::Postgres, Value, sqlx::postgres::PgArguments> {
    for a in args {
        q = match a {
            Arg::Uuid(v) => q.bind(v),
            Arg::Uuids(v) => q.bind(v),
            Arg::Text(v) => q.bind(v),
            Arg::Texts(v) => q.bind(v),
            Arg::Bool(v) => q.bind(v),
        };
    }
    q
}

fn split(v: &str) -> Vec<String> {
    v.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "null")
        .map(String::from)
        .collect()
}

fn uuids(v: &str) -> Vec<Uuid> {
    split(v).iter().filter_map(|s| Uuid::parse_str(s).ok()).collect()
}

/// Apply Plane's legacy query-string filters (`issue_filters`). Unknown keys are ignored.
pub fn apply_filters(base: &mut Base, q: &HashMap<String, String>) {
    let get = |k: &str| q.get(k).map(String::as_str).filter(|v| !v.is_empty());
    if let Some(v) = get("state") {
        base.and("i.state_id = ANY({})", Arg::Uuids(uuids(v)));
    }
    if let Some(v) = get("state_group") {
        base.and(
            "EXISTS (SELECT 1 FROM states fs WHERE fs.id = i.state_id AND fs.group = ANY({}))",
            Arg::Texts(split(v)),
        );
    }
    if let Some(v) = get("priority") {
        base.and("i.priority = ANY({})", Arg::Texts(split(v)));
    }
    if let Some(v) = get("parent") {
        base.and("i.parent_id = ANY({})", Arg::Uuids(uuids(v)));
    }
    if let Some(v) = get("labels") {
        base.and(
            "EXISTS (SELECT 1 FROM issue_labels fl WHERE fl.issue_id = i.id AND fl.deleted_at IS NULL AND fl.label_id = ANY({}))",
            Arg::Uuids(uuids(v)),
        );
    }
    if let Some(v) = get("assignees") {
        base.and(
            "EXISTS (SELECT 1 FROM issue_assignees fa WHERE fa.issue_id = i.id AND fa.deleted_at IS NULL AND fa.assignee_id = ANY({}))",
            Arg::Uuids(uuids(v)),
        );
    }
    if let Some(v) = get("mentions") {
        base.and(
            "EXISTS (SELECT 1 FROM issue_mentions fm WHERE fm.issue_id = i.id AND fm.deleted_at IS NULL AND fm.mention_id = ANY({}))",
            Arg::Uuids(uuids(v)),
        );
    }
    if let Some(v) = get("created_by") {
        base.and("i.created_by_id = ANY({})", Arg::Uuids(uuids(v)));
    }
    if let Some(v) = get("subscriber") {
        base.and(
            "EXISTS (SELECT 1 FROM issue_subscribers fsb WHERE fsb.issue_id = i.id AND fsb.deleted_at IS NULL AND fsb.subscriber_id = ANY({}))",
            Arg::Uuids(uuids(v)),
        );
    }
    if let Some(v) = get("cycle") {
        base.and(
            "EXISTS (SELECT 1 FROM cycle_issues fc WHERE fc.issue_id = i.id AND fc.deleted_at IS NULL AND fc.cycle_id = ANY({}))",
            Arg::Uuids(uuids(v)),
        );
    }
    if let Some(v) = get("module") {
        base.and(
            "EXISTS (SELECT 1 FROM module_issues fmo WHERE fmo.issue_id = i.id AND fmo.deleted_at IS NULL AND fmo.module_id = ANY({}))",
            Arg::Uuids(uuids(v)),
        );
    }
    if let Some(v) = get("project") {
        base.and("i.project_id = ANY({})", Arg::Uuids(uuids(v)));
    }
    if let Some(v) = get("estimate_point") {
        base.and("i.estimate_point_id = ANY({})", Arg::Uuids(uuids(v)));
    }
    if let Some(v) = get("name") {
        base.and("i.name ILIKE '%' || {} || '%'", Arg::Text(v.to_string()));
    }
    if get("sub_issue") == Some("false") {
        base.and_raw("i.parent_id IS NULL");
    }
    for (key, col) in [
        ("start_date", "i.start_date"),
        ("target_date", "i.target_date"),
        ("completed_at", "i.completed_at::date"),
        ("created_at", "i.created_at::date"),
        ("updated_at", "i.updated_at::date"),
    ] {
        if let Some(v) = get(key) {
            // Plane format: "2024-01-01;after,2024-02-01;before"
            for part in split(v) {
                let (date, dir) = part.split_once(';').unwrap_or((part.as_str(), "after"));
                if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
                    continue;
                }
                let op = if dir == "before" { "<=" } else { ">=" };
                base.and(&format!("{col} {op} {{}}::date"), Arg::Text(date.to_string()));
            }
        }
    }
    if let Some(v) = get("updated_at__exact") {
        if chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").is_ok() {
            base.and("i.updated_at::date = {}::date", Arg::Text(v.to_string()));
        }
    }
    if let Some(v) = get("updated_at__gt") {
        base.and("i.updated_at > {}::timestamptz", Arg::Text(v.to_string()));
    }
}

/// `ORDER BY` for Plane's `order_by` param (whitelisted). Returns (sort key SQL, descending).
fn order_key(order_by: &str) -> (&'static str, bool) {
    let desc = order_by.starts_with('-');
    let key = match order_by.trim_start_matches('-') {
        "updated_at" => "i.updated_at",
        "sequence_id" => "i.sequence_id",
        "sort_order" => "i.sort_order",
        "target_date" => "i.target_date",
        "start_date" => "i.start_date",
        "completed_at" => "i.completed_at",
        "archived_at" => "i.archived_at",
        "priority" => "array_position(ARRAY['urgent','high','medium','low','none']::text[], i.priority::text)",
        "state__name" => "(SELECT s.name FROM states s WHERE s.id = i.state_id)",
        "state__group" => {
            "array_position(ARRAY['backlog','unstarted','started','completed','cancelled']::text[],
                           (SELECT s.group FROM states s WHERE s.id = i.state_id)::text)"
        }
        "assignees__first_name" => {
            "(SELECT min(u.first_name) FROM issue_assignees a JOIN users u ON u.id = a.assignee_id
                                     WHERE a.issue_id = i.id AND a.deleted_at IS NULL)"
        }
        "labels__name" => {
            "(SELECT min(l.name) FROM issue_labels il JOIN labels l ON l.id = il.label_id
                            WHERE il.issue_id = i.id AND il.deleted_at IS NULL)"
        }
        "issue_module__module__name" => {
            "(SELECT min(m.name) FROM module_issues mi JOIN modules m ON m.id = mi.module_id
                                          WHERE mi.issue_id = i.id AND mi.deleted_at IS NULL)"
        }
        _ => return ("i.created_at", order_by != "created_at"),
    };
    // Plane's `order_issue_queryset`: "priority" lists by priority index descending ("-priority" ascending),
    // and state group always lists backlog first.
    let desc = match order_by.trim_start_matches('-') {
        "priority" => !desc,
        "state__group" => false,
        _ => desc,
    };
    (key, desc)
}

fn order_sql(order_by: &str) -> String {
    let (key, desc) = order_key(order_by);
    if key == "i.created_at" {
        return format!("i.created_at {}", if desc { "DESC" } else { "ASC" });
    }
    format!(
        "{key} {} NULLS LAST, i.created_at DESC",
        if desc { "DESC" } else { "ASC" }
    )
}

/// SQL producing the group key(s) of one issue as a text set (m2m fields can put an issue in many groups).
/// `None` groups use the string "None", as Plane does.
fn group_expr(field: &str) -> Option<&'static str> {
    Some(match field {
        "state_id" => "ARRAY[i.state_id::text]",
        "state__group" => "ARRAY[(SELECT s.group FROM states s WHERE s.id = i.state_id)::text]",
        "priority" => "ARRAY[i.priority::text]",
        "project_id" => "ARRAY[i.project_id::text]",
        "created_by" => "ARRAY[i.created_by_id::text]",
        "target_date" => "ARRAY[COALESCE(i.target_date::text, 'None')]",
        "start_date" => "ARRAY[COALESCE(i.start_date::text, 'None')]",
        "cycle_id" => {
            "ARRAY[COALESCE((SELECT ci.cycle_id::text FROM cycle_issues ci WHERE ci.issue_id = i.id AND ci.deleted_at IS NULL LIMIT 1), 'None')]"
        }
        "labels__id" => {
            "COALESCE(NULLIF(ARRAY(SELECT l.label_id::text FROM issue_labels l WHERE l.issue_id = i.id AND l.deleted_at IS NULL), '{}'), ARRAY['None'])"
        }
        "assignees__id" => {
            "COALESCE(NULLIF(ARRAY(SELECT a.assignee_id::text FROM issue_assignees a WHERE a.issue_id = i.id AND a.deleted_at IS NULL), '{}'), ARRAY['None'])"
        }
        "issue_module__module_id" => {
            "COALESCE(NULLIF(ARRAY(SELECT m.module_id::text FROM module_issues m WHERE m.issue_id = i.id AND m.deleted_at IS NULL), '{}'), ARRAY['None'])"
        }
        _ => return None,
    })
}

/// All possible group keys, so empty groups still appear (`issue_group_values`).
async fn group_values(db: &PgPool, field: &str, scope: Scope, base: &Base) -> Result<Vec<String>, ApiError> {
    let (projects_sql, a, b) = match scope {
        Scope::Project(p) => ("SELECT $1::uuid", p, Uuid::nil()),
        Scope::Workspace { workspace_id, user_id } => (
            "SELECT project_id FROM project_members WHERE workspace_id = $1 AND member_id = $2 AND is_active AND deleted_at IS NULL",
            workspace_id,
            user_id,
        ),
    };
    let fixed = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let sql = match field {
        "priority" => return Ok(fixed(&["urgent", "high", "medium", "low", "none"])),
        "state__group" => return Ok(fixed(&["backlog", "unstarted", "started", "completed", "cancelled"])),
        "state_id" => format!("SELECT id::text FROM states WHERE project_id IN ({projects_sql}) AND NOT is_triage AND deleted_at IS NULL"),
        "labels__id" => format!("SELECT id::text FROM labels WHERE project_id IN ({projects_sql}) AND deleted_at IS NULL UNION ALL SELECT 'None'"),
        "issue_module__module_id" => format!("SELECT id::text FROM modules WHERE project_id IN ({projects_sql}) AND deleted_at IS NULL UNION ALL SELECT 'None'"),
        "cycle_id" => format!("SELECT id::text FROM cycles WHERE project_id IN ({projects_sql}) AND deleted_at IS NULL UNION ALL SELECT 'None'"),
        "project_id" => format!("SELECT id::text FROM projects WHERE id IN ({projects_sql}) AND deleted_at IS NULL"),
        "assignees__id" => match scope {
            Scope::Project(_) => "SELECT member_id::text FROM project_members WHERE project_id = $1 AND is_active AND deleted_at IS NULL
                                  UNION ALL SELECT 'None'".to_string(),
            Scope::Workspace { .. } => "SELECT member_id::text FROM workspace_members WHERE workspace_id = $1 AND is_active AND deleted_at IS NULL
                                        AND $2::uuid IS NOT NULL UNION ALL SELECT 'None'".to_string(),
        },
        // Value-derived groups: distinct values among the filtered issues.
        f => {
            let expr = group_expr(f).ok_or_else(|| ApiError::bad("Invalid group_by"))?;
            let sql = format!("SELECT DISTINCT to_jsonb(g) FROM issues i, unnest({expr}) g WHERE {}", base.sql);
            let rows: Vec<Value> = bind(sqlx::query_scalar(&sql), &base.args).fetch_all(db).await?;
            return Ok(rows.into_iter().filter_map(|v| v.as_str().map(String::from)).collect());
        }
    };
    let q = sqlx::query_scalar::<_, String>(&sql).bind(a);
    let q = if matches!(scope, Scope::Workspace { .. }) {
        q.bind(b)
    } else {
        q
    };
    Ok(q.fetch_all(db).await?)
}

fn parse_cursor(c: Option<&str>) -> (i64, bool) {
    // "<limit>:<page>:<is_prev>"
    let Some(c) = c else { return (0, false) };
    let parts: Vec<&str> = c.split(':').collect();
    if parts.len() != 3 {
        return (0, false);
    }
    (parts[1].parse().unwrap_or(0).max(0), parts[2] == "1")
}

/// Paginated, optionally grouped work-item list with Plane's response envelope.
pub async fn list(db: &PgPool, mut base: Base, q: &HashMap<String, String>, scope: Scope) -> ApiResult {
    apply_filters(&mut base, q);
    let per_page: i64 = q
        .get("per_page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000)
        .clamp(1, 1000);
    let (page, _) = parse_cursor(q.get("cursor").map(String::as_str));
    let order = order_sql(q.get("order_by").map(String::as_str).unwrap_or("-created_at"));
    let group_by = q
        .get("group_by")
        .map(String::as_str)
        .filter(|g| !g.is_empty() && *g != "null");
    let sub_group_by = q
        .get("sub_group_by")
        .map(String::as_str)
        .filter(|g| !g.is_empty() && *g != "null");
    let offset = page * per_page;

    let total: Value = bind(
        sqlx::query_scalar(&format!("SELECT to_jsonb(count(*)) FROM issues i WHERE {}", base.sql)),
        &base.args,
    )
    .fetch_one(db)
    .await?;
    let total = total.as_i64().unwrap_or(0);
    let envelope = |results: Value, has_next: bool, count: i64, total_pages: i64| {
        json!({
            "grouped_by": group_by, "sub_grouped_by": sub_group_by,
            "total_count": total,
            "next_cursor": format!("{per_page}:{}:0", page + 1),
            "prev_cursor": format!("{per_page}:{}:1", page - 1),
            "next_page_results": has_next, "prev_page_results": page > 0,
            "count": count, "total_pages": total_pages, "total_results": total,
            "extra_stats": null, "results": results,
        })
    };

    let Some(group_by) = group_by else {
        let rows: Vec<Value> = bind(
            sqlx::query_scalar(&format!(
                "SELECT {ITEM_JSON} FROM issues i WHERE {} ORDER BY {order} LIMIT {} OFFSET {offset}",
                base.sql,
                per_page + 1
            )),
            &base.args,
        )
        .fetch_all(db)
        .await?;
        let has_next = rows.len() as i64 > per_page;
        let rows: Vec<Value> = rows.into_iter().take(per_page as usize).collect();
        let n = rows.len() as i64;
        return Ok(Json(envelope(
            Value::Array(rows),
            has_next,
            n,
            (total + per_page - 1) / per_page,
        )));
    };
    if sub_group_by == Some(group_by) {
        return Err(ApiError::bad("Group by and sub group by cannot have same parameters"));
    }
    let gexpr = group_expr(group_by).ok_or_else(|| ApiError::bad("Invalid group_by"))?;
    let sexpr = match sub_group_by {
        Some(sg) => group_expr(sg).ok_or_else(|| ApiError::bad("Invalid sub_group_by"))?,
        None => "ARRAY['']",
    };
    // One row per (issue, group, sub group), numbered within the group, paged per group like Plane.
    let rows: Vec<Value> = bind(
        sqlx::query_scalar(&format!(
            "WITH x AS (
               SELECT i.*, g AS grp, sg AS sgrp, row_number() OVER (PARTITION BY g, sg ORDER BY {order}) AS rn,
                      count(*) OVER (PARTITION BY g) AS gtotal, count(*) OVER (PARTITION BY g, sg) AS sgtotal
               FROM issues i, unnest({gexpr}) g, unnest({sexpr}) sg WHERE {})
             SELECT jsonb_build_object('g', i.grp, 'sg', i.sgrp, 'gt', i.gtotal, 'sgt', i.sgtotal, 'more', i.rn > {stop}, 'item',
                                       CASE WHEN i.rn <= {stop} THEN {ITEM_JSON} END)
             FROM x i WHERE i.rn > {offset} AND i.rn <= {stop} + 1 ORDER BY i.rn",
            base.sql,
            stop = offset + per_page
        )),
        &base.args,
    )
    .fetch_all(db)
    .await?;
    let mut groups: Map<String, Value> = Map::new();
    let mk = |total: i64| json!({"results": if sub_group_by.is_some() { json!({}) } else { json!([]) }, "total_results": total});
    for g in group_values(db, group_by, scope, &base).await? {
        groups.insert(g, mk(0));
    }
    let sub_values = match sub_group_by {
        Some(sg) => group_values(db, sg, scope, &base).await?,
        None => vec![],
    };
    let mut has_next = false;
    let mut count = 0i64;
    for r in rows {
        let g = r["g"].as_str().unwrap_or("None").to_string();
        let entry = groups.entry(g).or_insert_with(|| mk(0));
        entry["total_results"] = r["gt"].clone();
        if r["more"].as_bool() == Some(true) {
            has_next = true;
            continue;
        }
        count += 1;
        match sub_group_by {
            None => entry["results"].as_array_mut().unwrap().push(r["item"].clone()),
            Some(_) => {
                let sg = r["sg"].as_str().unwrap_or("None").to_string();
                let subs = entry["results"].as_object_mut().unwrap();
                let s = subs
                    .entry(sg)
                    .or_insert_with(|| json!({"results": [], "total_results": 0}));
                s["total_results"] = r["sgt"].clone();
                s["results"].as_array_mut().unwrap().push(r["item"].clone());
            }
        }
    }
    if sub_group_by.is_some() {
        for (_, entry) in groups.iter_mut() {
            let subs = entry["results"].as_object_mut().unwrap();
            for sv in &sub_values {
                subs.entry(sv.clone())
                    .or_insert_with(|| json!({"results": [], "total_results": 0}));
            }
        }
    }
    Ok(Json(envelope(
        Value::Object(groups),
        has_next,
        count,
        (total + per_page - 1) / per_page,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_bind_params() {
        let mut b = Base::new("i.project_id = $1", vec![Arg::Uuid(Uuid::nil())]);
        let q: HashMap<String, String> = [
            ("priority".into(), "high,urgent".into()),
            ("start_date".into(), "2024-01-01;after,bad;before".into()),
        ]
        .into();
        apply_filters(&mut b, &q);
        assert_eq!(b.args.len(), 3);
        assert!(b.sql.contains("i.priority = ANY($2)"));
        assert!(b.sql.contains("i.start_date >= $3::date"));
    }

    #[test]
    fn cursor_and_order() {
        assert_eq!(parse_cursor(Some("100:2:0")), (2, false));
        assert_eq!(parse_cursor(Some("junk")), (0, false));
        assert_eq!(order_sql("-created_at"), "i.created_at DESC");
        assert!(order_sql("; drop table x").starts_with("i.created_at"));
    }
}
