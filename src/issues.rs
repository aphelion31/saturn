//! Work items (issues): CRUD, comments, links, relations, sub-issues, activity, reactions, archive, search.
//! The list engine lives in `workitems.rs`; this module owns the issue routes.
//! TODO (worker): port the rest of `app/urls/issue.py`.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::common::*;
use crate::workitems::{self, ACTIVE, Arg, Base, Scope};

fn uuid_list(body: &Value, field: &str) -> Result<Vec<Uuid>, ApiError> {
    match body.get(field) {
        None => Ok(Vec::new()),
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| ApiError::bad("Invalid UUID list"))
            })
            .collect(),
        _ => Err(ApiError::bad("Invalid UUID list")),
    }
}

fn optional_uuid(body: &Value, field: &str) -> Result<Option<Uuid>, ApiError> {
    match body.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .map(Some)
            .ok_or_else(|| ApiError::bad("Invalid UUID")),
    }
}

fn optional_date(body: &Value, field: &str) -> Result<Option<chrono::NaiveDate>, ApiError> {
    match body.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .map(Some)
            .ok_or_else(|| ApiError::bad("Invalid date")),
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/",
            get(list).post(create),
        )
        .route("/api/workspaces/{slug}/projects/{project_id}/issues/list/", get(list))
        .route("/api/workspaces/{slug}/projects/{project_id}/v2/issues/", get(list))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/archived-issues/",
            get(archived_list),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/",
            get(detail).patch(update).delete(destroy),
        )
}

async fn update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): Path<(String, Uuid, Uuid)>,
    Json(mut body): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let object = body.as_object().ok_or_else(|| ApiError::bad("Invalid request"))?;
    if object
        .keys()
        .any(|k| !["name", "priority", "description_html"].contains(&k.as_str()))
    {
        return Err(ApiError::bad("Unsupported issue update field"));
    }
    if body.get("name").is_some() && s(&body, "name").is_none_or(|v| v.trim().is_empty() || v.chars().count() > 255) {
        return Err(ApiError::bad("Invalid name"));
    }
    if body.get("priority").is_some()
        && s(&body, "priority").is_none_or(|v| !["none", "urgent", "high", "medium", "low"].contains(&v))
    {
        return Err(ApiError::bad("Invalid priority"));
    }
    if let Some(v) = body.get("description_html") {
        let html = sanitize_html(v.as_str().ok_or_else(|| ApiError::bad("Invalid description_html"))?)?;
        body["description_html"] = Value::String(html);
    }
    let mut tx = st.db.begin().await?;
    let old: Option<Value> = sqlx::query_scalar("SELECT to_jsonb(i) FROM issues i WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL FOR UPDATE")
        .bind(issue_id).bind(project_id).bind(p.workspace_id()).fetch_optional(&mut *tx).await?;
    let old = old.ok_or_else(ApiError::not_found)?;
    let html = s(&body, "description_html");
    sqlx::query("UPDATE issues SET name=COALESCE($4,name), priority=COALESCE($5,priority), description_html=COALESCE($6,description_html), description_stripped=CASE WHEN $6::text IS NULL THEN description_stripped ELSE $7 END,updated_at=now(),updated_by_id=$8 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
        .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(s(&body,"name")).bind(s(&body,"priority")).bind(html).bind(html.map(strip_tags)).bind(user.id).execute(&mut *tx).await?;
    for field in ["name", "priority", "description_html"] {
        if let Some(value) = body.get(field)
            && old.get(field) != Some(value)
        {
            sqlx::query("INSERT INTO issue_activities (issue_id,project_id,workspace_id,actor_id,created_by_id,updated_by_id,verb,field,old_value,new_value,comment,attachments,epoch) VALUES ($1,$2,$3,$4,$4,$4,'updated',$5,$6,$7,'',ARRAY[]::varchar[],extract(epoch from now()))")
                .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(user.id).bind(field).bind(old.get(field).and_then(Value::as_str)).bind(value.as_str()).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    detail(State(st), user, Path((slug, project_id, issue_id))).await
}

async fn destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let result = sqlx::query("UPDATE issues SET deleted_at=now(), updated_at=now(), updated_by_id=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND ($5 OR created_by_id=$4)")
        .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(user.id).bind(p.role == ADMIN || p.ws.is_admin()).execute(&st.db).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(body): Json<Value>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let name = s(&body, "name")
        .filter(|n| !n.trim().is_empty() && n.chars().count() <= 255)
        .ok_or_else(|| ApiError::bad("A name of at most 255 characters is required"))?;
    // Reject fields not yet implemented rather than silently discard user data.
    if body.as_object().is_none_or(|b| {
        b.keys().any(|k| {
            ![
                "name",
                "state_id",
                "assignee_ids",
                "label_ids",
                "priority",
                "description_html",
                "parent_id",
                "start_date",
                "target_date",
                "estimate_point",
                "sort_order",
                "cycle_id",
                "module_ids",
                "type_id",
            ]
            .contains(&k.as_str())
        })
    }) {
        return Err(ApiError::bad("Unsupported issue creation field"));
    }
    let state = if body.get("state_id").is_some_and(|v| !v.is_null()) {
        Some(uuid_of(&body, "state_id").ok_or_else(|| ApiError::bad("Invalid state_id"))?)
    } else {
        None
    };
    let mut tx = st.db.begin().await?;
    let assignees = uuid_list(&body, "assignee_ids")?;
    let labels = uuid_list(&body, "label_ids")?;
    let modules = uuid_list(&body, "module_ids")?;
    let parent = optional_uuid(&body, "parent_id")?;
    let estimate = optional_uuid(&body, "estimate_point")?;
    let cycle = optional_uuid(&body, "cycle_id")?;
    let issue_type = optional_uuid(&body, "type_id")?;
    let start = optional_date(&body, "start_date")?;
    let target = optional_date(&body, "target_date")?;
    if start.zip(target).is_some_and(|(a, b)| a > b) {
        return Err(ApiError::bad("Start date cannot exceed target date"));
    }
    let html = match body.get("description_html") {
        None => "<p></p>".to_string(),
        Some(Value::String(v)) => sanitize_html(v)?,
        _ => return Err(ApiError::bad("Invalid description_html")),
    };
    let sort = match body.get("sort_order") {
        None => None,
        Some(v) => Some(
            v.as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| ApiError::bad("Invalid sort_order"))?,
        ),
    };
    for (id, sql) in [
        (
            parent,
            "SELECT EXISTS(SELECT 1 FROM issues WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
        ),
        (
            estimate,
            "SELECT EXISTS(SELECT 1 FROM estimate_points WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
        ),
        (
            cycle,
            "SELECT EXISTS(SELECT 1 FROM cycles WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
        ),
        (
            issue_type,
            "SELECT EXISTS(SELECT 1 FROM issue_types WHERE id=$1 AND workspace_id=$3 AND deleted_at IS NULL AND $2::uuid IS NOT NULL)",
        ),
    ] {
        if let Some(id) = id {
            let valid: bool = sqlx::query_scalar(sql)
                .bind(id)
                .bind(project_id)
                .bind(p.workspace_id())
                .fetch_one(&mut *tx)
                .await?;
            if !valid {
                return Err(ApiError::bad("Invalid related object"));
            }
        }
    }
    let priority = s(&body, "priority").unwrap_or("none");
    if !["none", "urgent", "high", "medium", "low"].contains(&priority) {
        return Err(ApiError::bad("Invalid priority"));
    }
    let state: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM states WHERE project_id=$1 AND workspace_id=$2 AND deleted_at IS NULL
         AND NOT is_triage AND ($3::uuid IS NULL OR id=$3) ORDER BY \"default\" DESC, sequence LIMIT 1",
    )
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(state)
    .fetch_optional(&mut *tx)
    .await?;
    if state.is_none() {
        return Err(ApiError::bad("Invalid or missing project state"));
    }
    let digest = Sha256::digest(project_id.to_string().as_bytes());
    let lock = i64::from_be_bytes(digest[..8].try_into().expect("SHA-256 length"));
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock)
        .execute(&mut *tx)
        .await?;
    let sequence: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(sequence),0)+1 FROM issue_sequences WHERE project_id=$1 AND workspace_id=$2 AND deleted_at IS NULL")
        .bind(project_id).bind(p.workspace_id()).fetch_one(&mut *tx).await?;
    let sequence = i32::try_from(sequence).map_err(|_| ApiError::bad("Issue sequence exhausted"))?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO issues (name, project_id, workspace_id, state_id, sequence_id, created_by_id, updated_by_id, sort_order, completed_at)
        VALUES ($1,$2,$3,$4,$5,$6,$6,COALESCE((SELECT MAX(sort_order)+10000 FROM issues WHERE project_id=$2 AND workspace_id=$3 AND state_id=$4 AND deleted_at IS NULL),65535),
        CASE WHEN (SELECT \"group\" FROM states WHERE id=$4 AND project_id=$2 AND workspace_id=$3)='completed' THEN now() ELSE NULL END) RETURNING id")
        .bind(name).bind(project_id).bind(p.workspace_id()).bind(state).bind(sequence).bind(user.id).fetch_one(&mut *tx).await?;
    sqlx::query("UPDATE issues SET priority=$4,description_html=$5,description_stripped=$6,parent_id=$7,start_date=$8,target_date=$9,estimate_point_id=$10,sort_order=COALESCE($11,sort_order),type_id=$12 WHERE id=$1 AND project_id=$2 AND workspace_id=$3")
        .bind(id).bind(project_id).bind(p.workspace_id()).bind(priority).bind(&html).bind(if html.is_empty() { None } else { Some(strip_tags(&html)) }).bind(parent).bind(start).bind(target).bind(estimate).bind(sort).bind(issue_type).execute(&mut *tx).await?;
    if let Some(cycle) = cycle {
        sqlx::query("INSERT INTO cycle_issues (issue_id,project_id,workspace_id,cycle_id,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$5,$5)")
            .bind(id).bind(project_id).bind(p.workspace_id()).bind(cycle).bind(user.id).execute(&mut *tx).await?;
    }
    for module in modules {
        let result = sqlx::query("INSERT INTO module_issues (issue_id,project_id,workspace_id,module_id,created_by_id,updated_by_id) SELECT $1,$2,$3,$4,$5,$5 WHERE EXISTS (SELECT 1 FROM modules WHERE id=$4 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL) AND NOT EXISTS (SELECT 1 FROM module_issues WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND module_id=$4 AND deleted_at IS NULL)")
            .bind(id).bind(project_id).bind(p.workspace_id()).bind(module).bind(user.id).execute(&mut *tx).await?;
        if result.rows_affected() == 0 {
            return Err(ApiError::bad("Invalid or duplicate module"));
        }
    }
    for assignee in assignees {
        let result = sqlx::query("INSERT INTO issue_assignees (issue_id,project_id,workspace_id,assignee_id,created_by_id,updated_by_id) SELECT $1,$2,$3,$4,$5,$5 WHERE EXISTS (SELECT 1 FROM project_members WHERE project_id=$2 AND workspace_id=$3 AND member_id=$4 AND is_active AND role>=15 AND deleted_at IS NULL) AND NOT EXISTS (SELECT 1 FROM issue_assignees WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND assignee_id=$4 AND deleted_at IS NULL)")
            .bind(id).bind(project_id).bind(p.workspace_id()).bind(assignee).bind(user.id).execute(&mut *tx).await?;
        if result.rows_affected() == 0 {
            return Err(ApiError::bad("Invalid or duplicate assignee"));
        }
    }
    for label in labels {
        let result = sqlx::query("INSERT INTO issue_labels (issue_id,project_id,workspace_id,label_id,created_by_id,updated_by_id) SELECT $1,$2,$3,$4,$5,$5 WHERE EXISTS (SELECT 1 FROM labels WHERE id=$4 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL) AND NOT EXISTS (SELECT 1 FROM issue_labels WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND label_id=$4 AND deleted_at IS NULL)")
            .bind(id).bind(project_id).bind(p.workspace_id()).bind(label).bind(user.id).execute(&mut *tx).await?;
        if result.rows_affected() == 0 {
            return Err(ApiError::bad("Invalid or duplicate label"));
        }
    }
    sqlx::query("INSERT INTO issue_sequences (issue_id, project_id, workspace_id, sequence, created_by_id, updated_by_id) VALUES ($1,$2,$3,$4,$5,$5)")
        .bind(id).bind(project_id).bind(p.workspace_id()).bind(i64::from(sequence)).bind(user.id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO issue_activities (issue_id, project_id, workspace_id, actor_id, created_by_id, updated_by_id, verb, comment, attachments, epoch) VALUES ($1,$2,$3,$4,$4,$4,'created','',ARRAY[]::varchar[],extract(epoch from now()))")
        .bind(id).bind(project_id).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
    let result: Value = sqlx::query_scalar(&format!(
        "SELECT {} || jsonb_build_object('description_html',i.description_html,'is_subscribed',false,'is_intake',false) FROM issues i WHERE i.id=$1 AND i.project_id=$2 AND i.workspace_id=$3",
        workitems::ITEM_JSON
    ))
    .bind(id)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    created(result)
}

async fn archived_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let base = Base::new(
        "i.project_id = $1 AND i.workspace_id = $2 AND i.deleted_at IS NULL AND i.archived_at IS NOT NULL",
        vec![Arg::Uuid(project_id), Arg::Uuid(p.workspace_id())],
    );
    workitems::list(&st.db, base, &q, Scope::Project(project_id)).await
}

async fn detail(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let sql = format!(
        "SELECT {} || jsonb_build_object('description_html', i.description_html,
          'is_subscribed', EXISTS (SELECT 1 FROM issue_subscribers sub WHERE sub.issue_id = i.id
          AND sub.workspace_id = i.workspace_id AND sub.project_id = i.project_id
          AND sub.subscriber_id = $4 AND sub.deleted_at IS NULL), 'is_intake',
          EXISTS (SELECT 1 FROM intake_issues ii WHERE ii.issue_id = i.id AND ii.project_id = i.project_id
          AND ii.workspace_id = i.workspace_id AND ii.status IN (-2, 0) AND ii.deleted_at IS NULL))
         FROM issues i WHERE i.project_id = $1 AND i.workspace_id = $2 AND i.id = $3
         AND {ACTIVE} AND (NOT $5 OR i.created_by_id = $4)",
        workitems::ITEM_JSON
    );
    json_row(
        sqlx::query_scalar(&sql)
            .bind(project_id)
            .bind(p.workspace_id())
            .bind(issue_id)
            .bind(user.id)
            .bind(p.restricted_guest()),
        &st.db,
    )
    .await
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
    if p.restricted_guest() {
        base.and("i.created_by_id = {}", Arg::Uuid(user.id));
    }
    workitems::list(&st.db, base, &q, Scope::Project(project_id)).await
}
