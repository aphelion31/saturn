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
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/comments/",
            get(comments).post(comment_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/comments/{comment_id}/",
            get(comment_detail)
                .patch(comment_update)
                .put(comment_update)
                .delete(comment_delete),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/history/",
            get(history),
        )
}

type IssuePath = (String, Uuid, Uuid);
type CommentPath = (String, Uuid, Uuid, Uuid);

async fn visible_issue(
    st: &AppState,
    user: &CurrentUser,
    slug: &str,
    project: Uuid,
    issue: Uuid,
) -> Result<ProjectCtx, ApiError> {
    let p = require_project(&st.db, user, slug, project).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let owner: Option<Option<Uuid>> = sqlx::query_scalar(
        "SELECT created_by_id FROM issues WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(issue)
    .bind(project)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    let owner = owner.ok_or_else(ApiError::not_found)?;
    if p.restricted_guest() && owner != Some(user.id) {
        return Err(ApiError::forbidden());
    }
    Ok(p)
}

// Serializer aliases use DRF's relation names rather than database column names.
const COMMENT_JSON: &str = "(to_jsonb(c) - 'actor_id' - 'issue_id' - 'project_id' - 'workspace_id' - 'created_by_id' - 'updated_by_id' - 'description_id' - 'parent_id') || jsonb_build_object(
 'actor',c.actor_id,'issue',c.issue_id,'project',c.project_id,'workspace',c.workspace_id,'created_by',c.created_by_id,'updated_by',c.updated_by_id,'description',c.description_id,'parent',c.parent_id,
 'actor_detail',(SELECT jsonb_build_object('id',u.id,'first_name',u.first_name,'last_name',u.last_name,'avatar',u.avatar,'avatar_url',CASE WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id || '/' ELSE NULLIF(u.avatar,'') END,'is_bot',u.is_bot,'display_name',u.display_name) FROM users u WHERE u.id=c.actor_id),
 'issue_detail',(SELECT jsonb_build_object('id',i.id,'name',i.name,'description_json',i.description_json,'description_html',i.description_html,'priority',i.priority,'start_date',i.start_date,'target_date',i.target_date,'sequence_id',i.sequence_id,'sort_order',i.sort_order,'is_draft',i.is_draft) FROM issues i WHERE i.id=c.issue_id AND i.project_id=c.project_id AND i.workspace_id=c.workspace_id),
 'project_detail',(SELECT jsonb_build_object('id',p.id,'identifier',p.identifier,'name',p.name,'cover_image',p.cover_image,'cover_image_url',NULLIF(p.cover_image,''),'logo_props',p.logo_props,'description',p.description) FROM projects p WHERE p.id=c.project_id AND p.workspace_id=c.workspace_id),
 'workspace_detail',(SELECT jsonb_build_object('id',w.id,'name',w.name,'slug',w.slug,'logo_url',NULLIF(w.logo,'')) FROM workspaces w WHERE w.id=c.workspace_id),
 'comment_reactions',COALESCE((SELECT jsonb_agg((to_jsonb(r)-'actor_id'-'comment_id'-'project_id'-'workspace_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('actor',r.actor_id,'comment',r.comment_id,'project',r.project_id,'workspace',r.workspace_id,'created_by',r.created_by_id,'updated_by',r.updated_by_id,'display_name',u.display_name)) FROM comment_reactions r JOIN users u ON u.id=r.actor_id WHERE r.comment_id=c.id AND r.project_id=c.project_id AND r.workspace_id=c.workspace_id AND r.deleted_at IS NULL),'[]'::jsonb),'is_member',true)";

async fn comments(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue)): Path<IssuePath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    json_rows(sqlx::query_scalar(&format!("SELECT {COMMENT_JSON} FROM issue_comments c WHERE c.issue_id=$1 AND c.project_id=$2 AND c.workspace_id=$3 AND c.deleted_at IS NULL ORDER BY c.created_at"))
        .bind(issue).bind(project).bind(p.workspace_id()), &st.db).await
}

async fn comment_detail(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue, comment)): Path<CommentPath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    json_row(sqlx::query_scalar(&format!("SELECT {COMMENT_JSON} FROM issue_comments c WHERE c.id=$1 AND c.issue_id=$2 AND c.project_id=$3 AND c.workspace_id=$4 AND c.deleted_at IS NULL"))
        .bind(comment).bind(issue).bind(project).bind(p.workspace_id()), &st.db).await
}

fn comment_fields(body: &Value) -> Result<(Option<String>, Option<Value>, Option<&str>), ApiError> {
    let object = body.as_object().ok_or_else(|| ApiError::bad("Invalid request"))?;
    let html = object
        .get("comment_html")
        .map(|v| {
            v.as_str()
                .ok_or_else(|| ApiError::bad("Invalid comment_html"))
                .and_then(sanitize_html)
        })
        .transpose()?;
    let json = object.get("comment_json").cloned();
    let access = object
        .get("access")
        .map(|v| {
            v.as_str()
                .filter(|s| ["INTERNAL", "EXTERNAL"].contains(s))
                .ok_or_else(|| ApiError::bad("Invalid access"))
        })
        .transpose()?;
    Ok((html, json, access))
}

async fn comment_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue)): Path<IssuePath>,
    Json(body): Json<Value>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    let (html, json, access) = comment_fields(&body)?;
    let html = html.unwrap_or_else(|| "<p></p>".into());
    let json = json.unwrap_or_else(|| serde_json::json!({}));
    let mut tx = st.db.begin().await?;
    let description: Uuid = sqlx::query_scalar("INSERT INTO descriptions (workspace_id,project_id,created_by_id,updated_by_id,description_html,description_stripped,description_json) VALUES ($1,$2,$3,$3,$4,$5,$6) RETURNING id")
        .bind(p.workspace_id()).bind(project).bind(user.id).bind(&html).bind(strip_tags(&html)).bind(&json).fetch_one(&mut *tx).await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO issue_comments (workspace_id,project_id,issue_id,actor_id,created_by_id,updated_by_id,comment_html,comment_stripped,comment_json,access,attachments,description_id) VALUES ($1,$2,$3,$4,$4,$4,$5,$6,$7,$8,ARRAY[]::varchar[],$9) RETURNING id")
        .bind(p.workspace_id()).bind(project).bind(issue).bind(user.id).bind(&html).bind(strip_tags(&html)).bind(json).bind(access.unwrap_or("INTERNAL")).bind(description).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    let Json(value) = comment_detail(State(st), user, Path((slug, project, issue, id))).await?;
    created(value)
}

async fn comment_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue, comment)): Path<CommentPath>,
    Json(body): Json<Value>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    let (html, json, access) = comment_fields(&body)?;
    let mut tx = st.db.begin().await?;
    let row: Option<(Uuid,)> = sqlx::query_as("UPDATE issue_comments SET edited_at=CASE WHEN $5::text IS NOT NULL AND comment_html IS DISTINCT FROM $5 THEN now() ELSE edited_at END, comment_html=COALESCE($5,comment_html),comment_stripped=COALESCE($6,comment_stripped),comment_json=COALESCE($7,comment_json),access=COALESCE($8,access),updated_at=now(),updated_by_id=$9 WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL AND ($10 OR created_by_id=$9) RETURNING id")
        .bind(comment).bind(issue).bind(project).bind(p.workspace_id()).bind(&html).bind(html.as_deref().map(strip_tags)).bind(&json).bind(access).bind(user.id).bind(p.is_admin() || p.ws.is_admin()).fetch_optional(&mut *tx).await?;
    if row.is_none() {
        return Err(ApiError::not_found());
    }
    sqlx::query("UPDATE descriptions d SET description_html=c.comment_html,description_stripped=c.comment_stripped,description_json=c.comment_json,updated_at=now(),updated_by_id=$5 FROM issue_comments c WHERE c.id=$1 AND c.issue_id=$2 AND c.project_id=$3 AND c.workspace_id=$4 AND d.id=c.description_id AND d.project_id=$3 AND d.workspace_id=$4 AND d.deleted_at IS NULL")
        .bind(comment).bind(issue).bind(project).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    comment_detail(State(st), user, Path((slug, project, issue, comment))).await
}

async fn comment_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue, comment)): Path<CommentPath>,
) -> ApiResult<axum::http::StatusCode> {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    let result = sqlx::query("UPDATE issue_comments SET deleted_at=now(),updated_at=now(),updated_by_id=$5 WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL AND ($6 OR created_by_id=$5)")
        .bind(comment).bind(issue).bind(project).bind(p.workspace_id()).bind(user.id).bind(p.is_admin() || p.ws.is_admin()).execute(&st.db).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn history(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue)): Path<IssuePath>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    let since = q
        .get("created_at__gt")
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|v| v.with_timezone(&chrono::Utc))
                .map_err(|_| ApiError::bad("Invalid created_at__gt"))
        })
        .transpose()?;
    let activity_type = q.get("activity_type").map(String::as_str).unwrap_or("");
    let sql = format!(
        "SELECT value FROM (SELECT c.created_at, {COMMENT_JSON} AS value FROM issue_comments c WHERE c.issue_id=$1 AND c.project_id=$2 AND c.workspace_id=$3 AND c.deleted_at IS NULL AND ($4::timestamptz IS NULL OR c.created_at>$4) AND $5 <> 'issue-property' UNION ALL SELECT a.created_at, (to_jsonb(a)-'actor_id'-'issue_id'-'project_id'-'workspace_id'-'created_by_id'-'updated_by_id'-'issue_comment_id') || jsonb_build_object('actor',a.actor_id,'issue',a.issue_id,'project',a.project_id,'workspace',a.workspace_id,'created_by',a.created_by_id,'updated_by',a.updated_by_id,'issue_comment',a.issue_comment_id,'source_data',NULL) FROM issue_activities a WHERE a.issue_id=$1 AND a.project_id=$2 AND a.workspace_id=$3 AND a.deleted_at IS NULL AND (a.field IS NULL OR a.field NOT IN ('comment','vote','reaction','draft')) AND ($4::timestamptz IS NULL OR a.created_at>$4) AND $5 <> 'issue-comment') x ORDER BY created_at"
    );
    json_rows(
        sqlx::query_scalar(&sql)
            .bind(issue)
            .bind(project)
            .bind(p.workspace_id())
            .bind(since)
            .bind(activity_type),
        &st.db,
    )
    .await
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
    if object.keys().any(|k| {
        ![
            "name",
            "priority",
            "description_html",
            "state_id",
            "assignee_ids",
            "label_ids",
        ]
        .contains(&k.as_str())
    }) {
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
    if body.get("state_id").is_some() {
        let state = optional_uuid(&body, "state_id")?.ok_or_else(|| ApiError::bad("Invalid state_id"))?;
        let result = sqlx::query("UPDATE issues SET state_id=$4,completed_at=CASE WHEN (SELECT \"group\" FROM states WHERE id=$4 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL) IN ('completed','cancelled') THEN now() ELSE NULL END WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND EXISTS (SELECT 1 FROM states WHERE id=$4 AND project_id=$2 AND workspace_id=$3 AND NOT is_triage AND deleted_at IS NULL)")
            .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(state).execute(&mut *tx).await?;
        if result.rows_affected() == 0 {
            return Err(ApiError::bad("Invalid state_id"));
        }
        if old.get("state_id").and_then(Value::as_str) != Some(state.to_string().as_str()) {
            sqlx::query("INSERT INTO issue_activities (issue_id,project_id,workspace_id,actor_id,created_by_id,updated_by_id,verb,field,old_value,new_value,comment,attachments,epoch) VALUES ($1,$2,$3,$4,$4,$4,'updated','state',$5,$6,'',ARRAY[]::varchar[],extract(epoch from now()))")
                .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(user.id).bind(old.get("state_id").and_then(Value::as_str)).bind(state.to_string()).execute(&mut *tx).await?;
        }
    }
    for field in ["assignee_ids", "label_ids"] {
        if body.get(field).is_none() {
            continue;
        }
        let ids = uuid_list(&body, field)?;
        let (existing_sql, remove_sql, add_sql) = if field == "assignee_ids" {
            (
                "SELECT assignee_id FROM issue_assignees WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
                "UPDATE issue_assignees SET deleted_at=now(),updated_at=now(),updated_by_id=$5 WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND NOT (assignee_id=ANY($4))",
                "INSERT INTO issue_assignees (issue_id,project_id,workspace_id,assignee_id,created_by_id,updated_by_id) SELECT $1,$2,$3,$4,$5,$5 WHERE EXISTS (SELECT 1 FROM project_members WHERE project_id=$2 AND workspace_id=$3 AND member_id=$4 AND is_active AND role>=15 AND deleted_at IS NULL) AND NOT EXISTS (SELECT 1 FROM issue_assignees WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND assignee_id=$4 AND deleted_at IS NULL)",
            )
        } else {
            (
                "SELECT label_id FROM issue_labels WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
                "UPDATE issue_labels SET deleted_at=now(),updated_at=now(),updated_by_id=$5 WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND NOT (label_id=ANY($4))",
                "INSERT INTO issue_labels (issue_id,project_id,workspace_id,label_id,created_by_id,updated_by_id) SELECT $1,$2,$3,$4,$5,$5 WHERE EXISTS (SELECT 1 FROM labels WHERE id=$4 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL) AND NOT EXISTS (SELECT 1 FROM issue_labels WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND label_id=$4 AND deleted_at IS NULL)",
            )
        };
        let before: Vec<Uuid> = sqlx::query_scalar(existing_sql)
            .bind(issue_id)
            .bind(project_id)
            .bind(p.workspace_id())
            .fetch_all(&mut *tx)
            .await?;
        sqlx::query(remove_sql)
            .bind(issue_id)
            .bind(project_id)
            .bind(p.workspace_id())
            .bind(&ids)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
        for id in &ids {
            if !before.contains(id) {
                let result = sqlx::query(add_sql)
                    .bind(issue_id)
                    .bind(project_id)
                    .bind(p.workspace_id())
                    .bind(id)
                    .bind(user.id)
                    .execute(&mut *tx)
                    .await?;
                if result.rows_affected() == 0 {
                    return Err(ApiError::bad("Invalid or duplicate relation"));
                }
            }
        }
        if before.iter().any(|id| !ids.contains(id)) || ids.iter().any(|id| !before.contains(id)) {
            sqlx::query("INSERT INTO issue_activities (issue_id,project_id,workspace_id,actor_id,created_by_id,updated_by_id,verb,field,old_value,new_value,comment,attachments,epoch) VALUES ($1,$2,$3,$4,$4,$4,'updated',$5,$6,$7,'',ARRAY[]::varchar[],extract(epoch from now()))")
                .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(user.id).bind(field).bind(serde_json::to_string(&before).unwrap_or_default()).bind(serde_json::to_string(&ids).unwrap_or_default()).execute(&mut *tx).await?;
        }
    }
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
