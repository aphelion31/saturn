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
            get(detail).delete(destroy),
        )
}

async fn destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let result = sqlx::query("UPDATE issues SET deleted_at=now(), updated_at=now(), updated_by_id=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
        .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(user.id).execute(&st.db).await?;
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
        b.keys()
            .any(|k| !["name", "state_id", "assignee_ids", "label_ids", "priority"].contains(&k.as_str()))
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
    sqlx::query("UPDATE issues SET priority=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3")
        .bind(id)
        .bind(project_id)
        .bind(p.workspace_id())
        .bind(priority)
        .execute(&mut *tx)
        .await?;
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
        "SELECT {} FROM issues i WHERE i.id=$1 AND i.project_id=$2 AND i.workspace_id=$3",
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
