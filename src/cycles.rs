//! Cycles: CRUD, date check, favorites, archive, cycle issues, transfer, user properties, progress.
//! Port of Plane's `app/urls/cycle.py` and `app/views/cycle/*`.
//!
//! Bind convention for the cycle SELECTs: `$1` project_id, `$2` workspace_id, `$3` user_id.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::NaiveDate;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/cycles/",
            get(list).post(create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/cycles/date-check/",
            post(date_check),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/cycles/{pk}/",
            get(retrieve).patch(update).put(update).delete(destroy),
        )
}

/// Project timezone (`$1` is the project id).
const TZ: &str = "(SELECT p.timezone FROM projects p WHERE p.id = $1)";

/// Start of the `YYYY-MM-DD` day held in bind `p` (NULL stays NULL), in the project timezone.
/// Today's start is `now()`, like Plane's `convert_to_utc`.
fn start_sql(p: &str) -> String {
    format!(
        "(CASE WHEN {p}::text IS NULL THEN NULL
          WHEN {p}::date = (now() AT TIME ZONE {TZ})::date THEN now()
          ELSE ((({p}::date)::timestamp + interval '1 second') AT TIME ZONE {TZ}) END)"
    )
}

/// End of the `YYYY-MM-DD` day held in bind `p`: 23:59 in the project timezone.
fn end_sql(p: &str) -> String {
    format!(
        "(CASE WHEN {p}::text IS NULL THEN NULL
          ELSE ((({p}::date)::timestamp + interval '23 hours 59 minutes') AT TIME ZONE {TZ}) END)"
    )
}

/// Plane's favorite flag for the current user in this project.
const IS_FAVORITE: &str = "EXISTS (SELECT 1 FROM user_favorites f WHERE f.entity_type = 'cycle'
    AND f.entity_identifier = c.id AND f.project_id = $1 AND f.user_id = $3 AND f.deleted_at IS NULL)";

/// Issue counts per state group and the assignee ids. Filters match Plane: not archived, not draft, not deleted.
const CYCLE_FROM: &str = "FROM cycles c
    LEFT JOIN LATERAL (
        SELECT count(*) AS total,
            count(*) FILTER (WHERE s.group = 'backlog') AS backlog,
            count(*) FILTER (WHERE s.group = 'unstarted') AS unstarted,
            count(*) FILTER (WHERE s.group = 'started') AS started,
            count(*) FILTER (WHERE s.group = 'completed') AS completed,
            count(*) FILTER (WHERE s.group = 'cancelled') AS cancelled
        FROM cycle_issues ci
        JOIN issues i ON i.id = ci.issue_id AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft
        LEFT JOIN states s ON s.id = i.state_id
        WHERE ci.cycle_id = c.id AND ci.deleted_at IS NULL
    ) k ON true
    LEFT JOIN LATERAL (
        SELECT COALESCE(jsonb_agg(DISTINCT a.assignee_id), '[]'::jsonb) AS ids
        FROM cycle_issues ci
        JOIN issues i ON i.id = ci.issue_id AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft
        JOIN issue_assignees a ON a.issue_id = i.id AND a.deleted_at IS NULL
        WHERE ci.cycle_id = c.id AND ci.deleted_at IS NULL
    ) asg ON true";

/// CycleSerializer fields plus list extras, and `status` (CURRENT / UPCOMING / COMPLETED / DRAFT).
/// ponytail: timestamps come back as stored (UTC); Plane converts them to the project timezone.
fn cycle_select(where_sql: &str) -> String {
    format!(
        "SELECT jsonb_build_object(
            'id', c.id, 'workspace_id', c.workspace_id, 'project_id', c.project_id, 'name', c.name,
            'description', c.description, 'start_date', c.start_date, 'end_date', c.end_date,
            'owned_by_id', c.owned_by_id, 'created_by', c.created_by_id, 'archived_at', c.archived_at,
            'view_props', c.view_props, 'sort_order', c.sort_order, 'external_source', c.external_source,
            'external_id', c.external_id, 'progress_snapshot', c.progress_snapshot, 'logo_props', c.logo_props,
            'version', c.version, 'is_favorite', {IS_FAVORITE},
            'total_issues', COALESCE(k.total, 0), 'backlog_issues', COALESCE(k.backlog, 0),
            'unstarted_issues', COALESCE(k.unstarted, 0), 'started_issues', COALESCE(k.started, 0),
            'completed_issues', COALESCE(k.completed, 0), 'cancelled_issues', COALESCE(k.cancelled, 0),
            'assignee_ids', asg.ids,
            'status', CASE
                WHEN c.start_date <= now() AND c.end_date >= now() THEN 'CURRENT'
                WHEN c.start_date > now() THEN 'UPCOMING'
                WHEN c.end_date < now() THEN 'COMPLETED'
                ELSE 'DRAFT' END)
         {CYCLE_FROM}
         WHERE c.project_id = $1 AND c.workspace_id = $2 AND c.deleted_at IS NULL AND {where_sql}"
    )
}

fn cycle_not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, json!({"error": "Cycle not found"}))
}

const DATE_ERR: &str = "Date has wrong format. Use one of these formats instead: YYYY-MM-DD.";

/// Read a `YYYY-MM-DD` day from a body field. A datetime keeps only its date part. Missing or null is None.
fn parse_day(b: &Value, k: &str) -> Result<Option<String>, ApiError> {
    let v = match b.get(k) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(v)) if v.is_empty() => return Ok(None),
        Some(Value::String(v)) => v,
        Some(_) => return Err(ApiError::bad_body(json!({ (k): [DATE_ERR] }))),
    };
    let day = v.get(..10).unwrap_or(v);
    NaiveDate::parse_from_str(day, "%Y-%m-%d").map_err(|_| ApiError::bad_body(json!({ (k): [DATE_ERR] })))?;
    Ok(Some(day.to_string()))
}

/// `start <= end` for two `YYYY-MM-DD` days (lexical order is date order).
fn check_order(start: &Option<String>, end: &Option<String>) -> Result<(), ApiError> {
    if let (Some(s), Some(e)) = (start, end)
        && s > e
    {
        return Err(ApiError::bad_body(
            json!({"non_field_errors": ["Start date cannot exceed end date"]}),
        ));
    }
    Ok(())
}

/// Validated `name`: required when `Some` in the body, trimmed, 1..=255 chars.
fn name_of(b: &Value) -> Result<Option<String>, ApiError> {
    let required = || ApiError::bad_body(json!({"name": ["This field is required."]}));
    match b.get("name") {
        None => Ok(None),
        Some(Value::String(v)) => {
            let n = v.trim();
            if n.is_empty() {
                Err(required())
            } else if n.chars().count() > 255 {
                Err(ApiError::bad_body(
                    json!({"name": ["Ensure this field has no more than 255 characters."]}),
                ))
            } else {
                Ok(Some(n.to_string()))
            }
        }
        Some(_) => Err(required()),
    }
}

/// Fetch one cycle by id in this project. 404 when missing, deleted or archived is false.
async fn fetch_cycle(db: &sqlx::PgPool, ctx: &ProjectCtx, user: &CurrentUser, pk: Uuid) -> ApiResult {
    sqlx::query_scalar(&format!("{} AND c.id = $4", cycle_select("TRUE")))
        .bind(ctx.project_id)
        .bind(ctx.workspace_id())
        .bind(user.id)
        .bind(pk)
        .fetch_optional(db)
        .await?
        .map(Json)
        .ok_or_else(cycle_not_found)
}

async fn list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let order = format!("ORDER BY {IS_FAVORITE} DESC, c.created_at DESC");
    let base = "c.archived_at IS NULL";
    if q.get("cycle_view").map(String::as_str) == Some("current") {
        // Plane returns the current cycles, or all cycles when none is current.
        let current: Vec<Value> = sqlx::query_scalar(&format!(
            "{} AND {base} AND c.start_date <= now() AND c.end_date >= now() {order}",
            cycle_select("TRUE")
        ))
        .bind(project_id)
        .bind(p.workspace_id())
        .bind(user.id)
        .fetch_all(&st.db)
        .await?;
        if !current.is_empty() {
            return Ok(Json(Value::Array(current)));
        }
    }
    json_rows(
        sqlx::query_scalar(&format!("{} AND {base} {order}", cycle_select("TRUE")))
            .bind(project_id)
            .bind(p.workspace_id())
            .bind(user.id),
        &st.db,
    )
    .await
}

async fn retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    fetch_cycle(&st.db, &p, &user, pk).await
}

async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let name = name_of(&b)?.ok_or_else(|| ApiError::bad_body(json!({"name": ["This field is required."]})))?;
    let start = parse_day(&b, "start_date")?;
    let end = parse_day(&b, "end_date")?;
    // Plane: a cycle has either both dates or none (draft).
    if start.is_some() != end.is_some() {
        return Err(ApiError::bad(
            "Both start date and end date are either required or are to be null",
        ));
    }
    check_order(&start, &end)?;
    let id: Uuid = sqlx::query_scalar(&format!(
        "INSERT INTO cycles (id, name, description, start_date, end_date, owned_by_id, created_by_id, updated_by_id,
            project_id, workspace_id, timezone, view_props, logo_props, external_source, external_id, sort_order)
         VALUES (gen_random_uuid(), $4, COALESCE($5, ''), {start}, {end}, $3, $3, $3, $1, $2,
            (SELECT p.timezone FROM projects p WHERE p.id = $1),
            COALESCE($6::jsonb, '{{}}'::jsonb), COALESCE($7::jsonb, '{{}}'::jsonb), $8, $9,
            COALESCE($10::float8, (SELECT max(sort_order) + 10000 FROM cycles WHERE project_id = $1), 65535))
         RETURNING id",
        start = start_sql("$11"),
        end = end_sql("$12"),
    ))
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .bind(name)
    .bind(s(&b, "description"))
    .bind(b.get("view_props").filter(|v| !v.is_null()))
    .bind(b.get("logo_props").filter(|v| !v.is_null()))
    .bind(s(&b, "external_source"))
    .bind(s(&b, "external_id"))
    .bind(b.get("sort_order").and_then(Value::as_f64))
    .bind(start)
    .bind(end)
    .fetch_one(&st.db)
    .await?;
    Ok((StatusCode::CREATED, fetch_cycle(&st.db, &p, &user, id).await?))
}

async fn update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let cur: (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
        bool,
    ) = sqlx::query_as(
        "SELECT c.archived_at, c.end_date, c.archived_at IS NULL FROM cycles c
         WHERE c.id = $1 AND c.project_id = $2 AND c.workspace_id = $3 AND c.deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(cycle_not_found)?;
    if cur.0.is_some() {
        return Err(ApiError::bad("Archived cycle cannot be updated"));
    }
    // A completed cycle only accepts sort_order changes, other fields are dropped like Plane does.
    let completed = cur.1.is_some_and(|e| e < chrono::Utc::now());
    let only_sort = completed && b.get("sort_order").is_some();
    if completed && !only_sort {
        return Err(ApiError::bad(
            "The Cycle has already been completed so it cannot be edited",
        ));
    }
    let b = if only_sort {
        json!({"sort_order": b["sort_order"]})
    } else {
        b
    };
    let name = name_of(&b)?;
    let has_start = b.get("start_date").is_some();
    let has_end = b.get("end_date").is_some();
    let start = parse_day(&b, "start_date")?;
    let end = parse_day(&b, "end_date")?;
    // Plane checks the order only when both dates are in the request.
    if has_start && has_end {
        check_order(&start, &end)?;
    }
    let v: Option<Uuid> = sqlx::query_scalar(&format!(
        "UPDATE cycles c SET
            name = COALESCE($4, c.name),
            description = COALESCE($5, c.description),
            start_date = CASE WHEN $6 THEN {start} ELSE c.start_date END,
            end_date = CASE WHEN $8 THEN {end} ELSE c.end_date END,
            view_props = COALESCE($10::jsonb, c.view_props),
            logo_props = COALESCE($11::jsonb, c.logo_props),
            external_source = COALESCE($12, c.external_source),
            external_id = COALESCE($13, c.external_id),
            sort_order = COALESCE($14::float8, c.sort_order),
            updated_by_id = $3, updated_at = now()
         WHERE c.id = $15 AND c.project_id = $1 AND c.workspace_id = $2
           AND c.deleted_at IS NULL AND c.archived_at IS NULL
         RETURNING c.id",
        start = start_sql("$7"),
        end = end_sql("$9"),
    ))
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .bind(name)
    .bind(s(&b, "description"))
    .bind(has_start)
    .bind(start)
    .bind(has_end)
    .bind(end)
    .bind(b.get("view_props").filter(|v| !v.is_null()))
    .bind(b.get("logo_props").filter(|v| !v.is_null()))
    .bind(s(&b, "external_source"))
    .bind(s(&b, "external_id"))
    .bind(b.get("sort_order").and_then(Value::as_f64))
    .bind(pk)
    .fetch_optional(&st.db)
    .await?;
    v.ok_or_else(cycle_not_found)?;
    fetch_cycle(&st.db, &p, &user, pk).await
}

async fn destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    // allow_permission([ADMIN], creator=True): workspace or project admin, or the cycle creator.
    let creator: Option<Option<Uuid>> = sqlx::query_scalar(
        "SELECT c.created_by_id FROM cycles c WHERE c.id = $1 AND c.project_id = $2 AND c.workspace_id = $3
           AND c.deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    let creator = creator.ok_or_else(cycle_not_found)?;
    if !(p.is_admin() || creator == Some(user.id)) {
        return Err(ApiError::forbidden());
    }
    let mut tx = st.db.begin().await?;
    sqlx::query("UPDATE cycles SET deleted_at = now(), updated_by_id = $2 WHERE id = $1 AND deleted_at IS NULL")
        .bind(pk)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    // Soft delete the bridge rows so the issues are no longer in the cycle.
    sqlx::query("UPDATE cycle_issues SET deleted_at = now() WHERE cycle_id = $1 AND deleted_at IS NULL")
        .bind(pk)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE user_favorites SET deleted_at = now() WHERE entity_type = 'cycle' AND entity_identifier = $1
           AND project_id = $2 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    no_content()
}

/// POST cycles/date-check/: 200 with `{"status": true}`, or `{"error": ..., "status": false}` on overlap.
async fn date_check(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let (Some(start), Some(end)) = (parse_day(&b, "start_date")?, parse_day(&b, "end_date")?) else {
        return Err(ApiError::bad("Start date and end date both are required"));
    };
    let cycle_id = uuid_of(&b, "cycle_id");
    // Bounds use the same day-to-UTC rule as create/update, then overlap is checked on the stored columns.
    let overlap: bool = sqlx::query_scalar(&format!(
        "WITH r AS (SELECT {s} AS a, {e} AS z)
         SELECT EXISTS (SELECT 1 FROM cycles c, r
           WHERE c.project_id = $1 AND c.workspace_id = $2 AND c.deleted_at IS NULL
             AND c.id IS DISTINCT FROM $5::uuid
             AND ((c.start_date <= r.a AND c.end_date >= r.a)
               OR (c.start_date <= r.z AND c.end_date >= r.z)
               OR (c.start_date >= r.a AND c.end_date <= r.z)))",
        s = start_sql("$3::text"),
        e = end_sql("$4::text"),
    ))
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(start)
    .bind(end)
    .bind(cycle_id)
    .fetch_one(&st.db)
    .await?;
    if overlap {
        return Ok(Json(json!({
            "error": "You have a cycle already on the given dates, if you want to create a draft cycle you can do that by removing dates",
            "status": false,
        })));
    }
    Ok(Json(json!({"status": true})))
}
