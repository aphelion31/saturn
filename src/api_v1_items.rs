//! Public API v1 work items (`X-Api-Key`). Ports Plane's `api/urls/work_item.py` views. Both `/issues/`
//! and `/work-items/` prefixes map to the same handlers. Step 1: work-item CRUD.
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
use crate::workitems::{self, ACTIVE, Arg, Base, Scope};

type ProjectPath = Path<(String, Uuid)>;
type ItemPath = Path<(String, Uuid, Uuid)>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/",
            get(list).post(create),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/",
            get(list).post(create),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{pk}/",
            get(retrieve).patch(update).delete(destroy),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{pk}/",
            get(retrieve).patch(update).delete(destroy),
        )
}

/// Plane v1 IssueSerializer fields: model columns as plain ids, minus description_json/stripped.
/// `assignees` and `labels` are uuid lists. Not restricted by `ACTIVE`, callers add their own filter.
const ITEM_V1: &str = "jsonb_build_object(
    'id', i.id, 'name', i.name, 'description_html', i.description_html, 'priority', i.priority,
    'start_date', i.start_date, 'target_date', i.target_date, 'sequence_id', i.sequence_id,
    'sort_order', i.sort_order, 'completed_at', i.completed_at, 'archived_at', i.archived_at,
    'is_draft', i.is_draft, 'external_id', i.external_id, 'external_source', i.external_source,
    'point', i.point, 'estimate_point', i.estimate_point_id, 'state', i.state_id, 'parent', i.parent_id,
    'type', i.type_id, 'project', i.project_id, 'workspace', i.workspace_id,
    'created_by', i.created_by_id, 'updated_by', i.updated_by_id,
    'created_at', i.created_at, 'updated_at', i.updated_at,
    'assignees', COALESCE((SELECT jsonb_agg(a.assignee_id) FROM issue_assignees a JOIN users u ON u.id = a.assignee_id
                           WHERE a.issue_id = i.id AND a.deleted_at IS NULL AND u.is_active), '[]'::jsonb),
    'labels', COALESCE((SELECT jsonb_agg(l.label_id) FROM issue_labels l
                        WHERE l.issue_id = i.id AND l.deleted_at IS NULL), '[]'::jsonb))";

/// Restricted guests (`restricted_guest()`) only see work items they created.
fn scoped(p: &ProjectCtx, base: &mut Base) {
    if p.restricted_guest() {
        base.and("i.created_by_id = {}", Arg::Uuid(p.user_id()));
    }
}

async fn list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let mut base = Base::new(
        format!("i.project_id = $1 AND i.workspace_id = $2 AND {ACTIVE}"),
        vec![Arg::Uuid(project_id), Arg::Uuid(p.workspace_id())],
    );
    scoped(&p, &mut base);
    workitems::list(&st.db, base, &q, Scope::Project(project_id)).await
}

async fn retrieve(State(st): State<AppState>, user: CurrentUser, Path((slug, project_id, pk)): ItemPath) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let mut sql = format!(
        "SELECT {ITEM_V1} FROM issues i WHERE i.id = $1 AND i.project_id = $2 AND i.workspace_id = $3 AND {ACTIVE}"
    );
    if p.restricted_guest() {
        sql.push_str(" AND i.created_by_id = $4");
    }
    let q = sqlx::query_scalar(&sql)
        .bind(pk)
        .bind(project_id)
        .bind(p.workspace_id());
    let q = if p.restricted_guest() { q.bind(user.id) } else { q };
    json_row(q, &st.db).await
}

/// Allocates `sequence_id` like `issues.rs::create_issue`: advisory lock per project, then max + 1.
async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): ProjectPath,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let name = s(&b, "name")
        .map(str::trim)
        .filter(|n| !n.is_empty() && n.chars().count() <= 255)
        .ok_or_else(|| ApiError::bad_body(json!({"name": ["A name of at most 255 characters is required."]})))?;
    let priority = s(&b, "priority").unwrap_or("none");
    if !["none", "urgent", "high", "medium", "low"].contains(&priority) {
        return Err(ApiError::bad_body(
            json!({"priority": [format!("\"{priority}\" is not a valid choice.")]}),
        ));
    }
    let html = match b.get("description_html") {
        None | Some(Value::Null) => "<p></p>".to_string(),
        Some(Value::String(v)) => sanitize_html(v)?,
        _ => return Err(ApiError::bad_body(json!({"description_html": ["Invalid HTML."]}))),
    };
    let start = opt_date(&b, "start_date")?;
    let target = opt_date(&b, "target_date")?;
    if start.zip(target).is_some_and(|(a, z)| a > z) {
        return Err(ApiError::bad("Start date cannot exceed target date"));
    }
    let assignees = uuid_list(&b, "assignee_ids")?;
    let labels = uuid_list(&b, "label_ids")?;
    let state = opt_uuid(&b, "state_id")?;
    let parent = opt_uuid(&b, "parent_id")?;
    let estimate = opt_uuid(&b, "estimate_point")?;

    let mut tx = st.db.begin().await?;
    // Default state when none given, same rule as issues.rs (non-triage, default first).
    let state: Uuid = sqlx::query_scalar(
        "SELECT id FROM states WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL
         AND NOT is_triage AND ($3::uuid IS NULL OR id = $3) ORDER BY \"default\" DESC, sequence LIMIT 1",
    )
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(state)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::bad("Invalid or missing project state"))?;
    if let Some(parent) = parent {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM issues WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL)",
        )
        .bind(parent)
        .bind(project_id)
        .bind(p.workspace_id())
        .fetch_one(&mut *tx)
        .await?;
        if !ok {
            return Err(ApiError::bad("Invalid related object"));
        }
    }
    // Same advisory lock key as issues.rs::create_issue.
    let digest = <sha2::Sha256 as sha2::Digest>::digest(project_id.to_string().as_bytes());
    let lock = i64::from_be_bytes(digest[..8].try_into().expect("SHA-256 length"));
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock)
        .execute(&mut *tx)
        .await?;
    let sequence: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sequence),0)+1 FROM issue_sequences WHERE project_id=$1 AND workspace_id=$2 AND deleted_at IS NULL",
    )
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_one(&mut *tx)
    .await?;
    let sequence = i32::try_from(sequence).map_err(|_| ApiError::bad("Issue sequence exhausted"))?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO issues (name, project_id, workspace_id, state_id, sequence_id, created_by_id, updated_by_id,
            sort_order, completed_at, priority, description_html, description_stripped, parent_id, start_date,
            target_date, estimate_point_id)
         VALUES ($1,$2,$3,$4,$5,$6,$6,
            COALESCE((SELECT MAX(sort_order)+10000 FROM issues WHERE project_id=$2 AND workspace_id=$3 AND state_id=$4 AND deleted_at IS NULL), 65535),
            CASE WHEN (SELECT \"group\" FROM states WHERE id=$4) IN ('completed','cancelled') THEN now() ELSE NULL END,
            $7, $8, $9, $10, $11, $12, $13)
         RETURNING id",
    )
    .bind(name)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(state)
    .bind(sequence)
    .bind(user.id)
    .bind(priority)
    .bind(&html)
    .bind(strip_tags(&html))
    .bind(parent)
    .bind(start)
    .bind(target)
    .bind(estimate)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO issue_sequences (issue_id, project_id, workspace_id, sequence, created_by_id, updated_by_id) VALUES ($1,$2,$3,$4,$5,$5)")
        .bind(id)
        .bind(project_id)
        .bind(p.workspace_id())
        .bind(i64::from(sequence))
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    for a in assignees {
        sqlx::query("INSERT INTO issue_assignees (issue_id,project_id,workspace_id,assignee_id,created_by_id,updated_by_id) SELECT $1,$2,$3,$4,$5,$5 WHERE EXISTS (SELECT 1 FROM project_members WHERE project_id=$2 AND workspace_id=$3 AND member_id=$4 AND is_active AND role>=15 AND deleted_at IS NULL)")
            .bind(id)
            .bind(project_id)
            .bind(p.workspace_id())
            .bind(a)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
    }
    for l in labels {
        sqlx::query("INSERT INTO issue_labels (issue_id,project_id,workspace_id,label_id,created_by_id,updated_by_id) SELECT $1,$2,$3,$4,$5,$5 WHERE EXISTS (SELECT 1 FROM labels WHERE id=$4 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)")
            .bind(id)
            .bind(project_id)
            .bind(p.workspace_id())
            .bind(l)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let v = read_item(&st, &p, id).await?;
    created(v)
}

async fn read_item(st: &AppState, p: &ProjectCtx, id: Uuid) -> Result<Value, ApiError> {
    let v: Value = sqlx::query_scalar(&format!(
        "SELECT {ITEM_V1} FROM issues i WHERE i.id = $1 AND i.project_id = $2 AND i.workspace_id = $3"
    ))
    .bind(id)
    .bind(p.project_id)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;
    Ok(v)
}

/// PATCH: whitelisted columns only. Plane's `IssueSerializer(partial=True)`.
async fn update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): ItemPath,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    if b.as_object().is_none() {
        return Err(ApiError::bad("Invalid request"));
    }
    if let Some(v) = b.get("name")
        && v.as_str()
            .is_none_or(|n| n.trim().is_empty() || n.chars().count() > 255)
    {
        return Err(ApiError::bad_body(
            json!({"name": ["A name of at most 255 characters is required."]}),
        ));
    }
    if let Some(v) = s(&b, "priority")
        && !["none", "urgent", "high", "medium", "low"].contains(&v)
    {
        return Err(ApiError::bad_body(
            json!({"priority": [format!("\"{v}\" is not a valid choice.")]}),
        ));
    }
    let html = match b.get("description_html") {
        None => None,
        Some(Value::String(v)) => Some(sanitize_html(v)?),
        Some(_) => return Err(ApiError::bad_body(json!({"description_html": ["Invalid HTML."]}))),
    };
    let mut tx = st.db.begin().await?;
    let exists: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM issues i WHERE i.id = $1 AND i.project_id = $2 AND i.workspace_id = $3 AND i.deleted_at IS NULL AND {ACTIVE} AND (NOT $4 OR i.created_by_id = $5))"
    ))
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(p.restricted_guest())
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Err(ApiError::not_found());
    }
    sqlx::query(
        "UPDATE issues SET name = COALESCE($4, name), priority = COALESCE($5, priority),
            description_html = COALESCE($6, description_html),
            description_stripped = CASE WHEN $6::text IS NULL THEN description_stripped ELSE $7 END,
            updated_at = now(), updated_by_id = $8
         WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(s(&b, "name").map(str::trim))
    .bind(s(&b, "priority"))
    .bind(html.as_deref())
    .bind(html.as_deref().map(strip_tags))
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    retrieve(State(st), user, Path((slug, project_id, pk))).await
}

/// Soft delete. Only admins or the creator, as Plane.
async fn destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): ItemPath,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let r = sqlx::query(
        "UPDATE issues SET deleted_at = now(), updated_at = now(), updated_by_id = $4
         WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL AND ($5 OR created_by_id = $4)",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .bind(p.is_admin() || p.ws.is_admin())
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

fn uuid_list(b: &Value, k: &str) -> Result<Vec<Uuid>, ApiError> {
    match b.get(k) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(v)) => v
            .iter()
            .map(|x| {
                x.as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| ApiError::bad("Invalid UUID list"))
            })
            .collect(),
        _ => Err(ApiError::bad("Invalid UUID list")),
    }
}

fn opt_uuid(b: &Value, k: &str) -> Result<Option<Uuid>, ApiError> {
    match b.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .map(Some)
            .ok_or_else(|| ApiError::bad("Invalid UUID")),
    }
}

fn opt_date(b: &Value, k: &str) -> Result<Option<chrono::NaiveDate>, ApiError> {
    match b.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .map(Some)
            .ok_or_else(|| {
                ApiError::bad_body(
                    json!({ k: ["Date has wrong format. Use one of these formats instead: YYYY-MM-DD."]}),
                )
            }),
    }
}
