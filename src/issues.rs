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
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/issue-links/",
            get(links).post(link_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/issue-links/{link_id}/",
            get(link_detail).patch(link_update).put(link_update).delete(link_delete),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/reactions/",
            get(issue_reactions).post(issue_reaction_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/reactions/{code}/",
            axum::routing::delete(issue_reaction_delete),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/comments/{comment_id}/reactions/",
            get(comment_reactions).post(comment_reaction_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/comments/{comment_id}/reactions/{code}/",
            axum::routing::delete(comment_reaction_delete),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/subscribe/",
            get(subscription_status).post(subscribe).delete(unsubscribe),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/issue-subscribers/",
            get(subscribers).post(subscriber_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/issue-subscribers/{subscriber_id}/",
            axum::routing::delete(subscriber_delete),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/issue-relation/",
            get(relations).post(relation_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/remove-relation/",
            axum::routing::post(relation_remove),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/sub-issues/",
            get(sub_issues).post(sub_issue_assign),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/archive/",
            get(archive_detail).post(archive_issue).delete(unarchive_issue),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/bulk-delete-issues/",
            axum::routing::delete(bulk_delete),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/bulk-operation-issues/",
            axum::routing::post(bulk_operation),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/search-issues/",
            get(search_issues),
        )
        .route("/api/workspaces/{slug}/search/", get(global_search))
        .route(
            "/api/workspaces/{slug}/work-items/{identifier}/",
            get(identifier_detail),
        )
}

type IssuePath = (String, Uuid, Uuid);
type CommentPath = (String, Uuid, Uuid, Uuid);

async fn search_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let search = q.get("search").map(String::as_str).unwrap_or("");
    let wide = q.get("workspace_search").is_some_and(|v| v == "true");
    let issue = q
        .get("issue_id")
        .map(|v| Uuid::parse_str(v).map_err(|_| ApiError::bad("Invalid issue_id")))
        .transpose()?;
    let module = q
        .get("module")
        .map(|v| Uuid::parse_str(v).map_err(|_| ApiError::bad("Invalid module")))
        .transpose()?;
    let sql = format!(
        "SELECT jsonb_build_object('name',i.name,'id',i.id,'start_date',i.start_date,'sequence_id',i.sequence_id,'project__name',p.name,'project__identifier',p.identifier,'project_id',p.id,'workspace__slug',$4::text,'state__name',s.name,'state__group',s.group,'state__color',s.color) FROM issues i JOIN projects p ON p.id=i.project_id AND p.workspace_id=i.workspace_id JOIN states s ON s.id=i.state_id AND s.workspace_id=i.workspace_id JOIN project_members pm ON pm.project_id=i.project_id AND pm.workspace_id=i.workspace_id AND pm.member_id=$3 AND pm.is_active AND pm.deleted_at IS NULL WHERE i.workspace_id=$1 AND ($5 OR i.project_id=$2) AND {ACTIVE} AND (pm.role>5 OR p.guest_view_all_features OR i.created_by_id=$3) AND ($6='' OR i.name ILIKE '%'||$6||'%' OR (p.identifier||'-'||i.sequence_id::text) ILIKE '%'||$6||'%') AND (NOT $7 OR $8::uuid IS NULL OR (i.id<>$8 AND i.parent_id IS DISTINCT FROM $8 AND i.id IS DISTINCT FROM (SELECT parent_id FROM issues WHERE id=$8 AND workspace_id=$1 AND project_id=$2 AND deleted_at IS NULL))) AND (NOT $9 OR $8::uuid IS NULL OR (i.id<>$8 AND NOT EXISTS(SELECT 1 FROM issue_relations r WHERE r.workspace_id=$1 AND r.deleted_at IS NULL AND ((r.issue_id=$8 AND r.related_issue_id=i.id) OR (r.related_issue_id=$8 AND r.issue_id=i.id))))) AND (NOT $10 OR NOT EXISTS(SELECT 1 FROM cycle_issues ci WHERE ci.issue_id=i.id AND ci.workspace_id=$1 AND ci.project_id=i.project_id AND ci.deleted_at IS NULL)) AND ($11::uuid IS NULL OR NOT EXISTS(SELECT 1 FROM module_issues mi WHERE mi.issue_id=i.id AND mi.workspace_id=$1 AND mi.project_id=i.project_id AND mi.module_id=$11 AND mi.deleted_at IS NULL)) AND (NOT $12 OR i.target_date IS NULL) AND (NOT $13 OR (i.parent_id IS NULL AND i.id IS DISTINCT FROM $8)) ORDER BY i.created_at DESC LIMIT 100"
    );
    json_rows(
        sqlx::query_scalar(&sql)
            .bind(p.workspace_id())
            .bind(pid)
            .bind(user.id)
            .bind(slug)
            .bind(wide)
            .bind(search)
            .bind(q.get("parent").is_some_and(|v| v == "true"))
            .bind(issue)
            .bind(q.get("issue_relation").is_some_and(|v| v == "true"))
            .bind(q.get("cycle").is_some_and(|v| v == "true"))
            .bind(module)
            .bind(q.get("target_date").is_some_and(|v| v == "none"))
            .bind(q.get("sub_issue").is_some_and(|v| v == "true")),
        &st.db,
    )
    .await
}
async fn global_search(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let search = q.get("search").map(String::as_str).unwrap_or("");
    let requested = q
        .get("entities")
        .map(|s| s.split(',').map(str::trim).collect::<Vec<_>>())
        .unwrap_or_else(|| {
            vec![
                "workspace",
                "project",
                "issue",
                "cycle",
                "module",
                "issue_view",
                "page",
                "intake",
            ]
        });
    let project = q
        .get("project_id")
        .map(|v| Uuid::parse_str(v).map_err(|_| ApiError::bad("Invalid project_id")))
        .transpose()?;
    let wide = q.get("workspace_search").is_some_and(|v| v == "true");
    let mut results = serde_json::Map::new();
    for entity in requested {
        let sql = match entity {
            "workspace" => {
                "SELECT jsonb_build_object('name',w.name,'id',w.id,'slug',w.slug) FROM workspaces w JOIN workspace_members m ON m.workspace_id=w.id WHERE m.member_id=$2 AND m.is_active AND m.deleted_at IS NULL AND w.deleted_at IS NULL AND w.name ILIKE '%'||$3||'%' AND $1::uuid IS NOT NULL AND $4::uuid IS NOT NULL AND $5::text IS NOT NULL AND $6::boolean IS NOT NULL ORDER BY w.created_at DESC LIMIT 100"
            }
            "project" => {
                "SELECT jsonb_build_object('name',p.name,'id',p.id,'identifier',p.identifier,'workspace__slug',$5::text) FROM projects p JOIN project_members m ON m.project_id=p.id AND m.workspace_id=p.workspace_id WHERE p.workspace_id=$1 AND m.member_id=$2 AND m.is_active AND m.deleted_at IS NULL AND p.deleted_at IS NULL AND p.archived_at IS NULL AND (p.name ILIKE '%'||$3||'%' OR p.identifier ILIKE '%'||$3||'%') AND ($4::uuid IS NULL OR $6 OR p.id=$4) ORDER BY p.created_at DESC LIMIT 100"
            }
            "issue" => {
                "SELECT jsonb_build_object('name',i.name,'id',i.id,'sequence_id',i.sequence_id,'project__identifier',p.identifier,'project_id',p.id,'workspace__slug',$5::text) FROM issues i JOIN projects p ON p.id=i.project_id AND p.workspace_id=i.workspace_id JOIN project_members m ON m.project_id=p.id AND m.workspace_id=p.workspace_id WHERE i.workspace_id=$1 AND m.member_id=$2 AND m.is_active AND m.deleted_at IS NULL AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft AND p.deleted_at IS NULL AND p.archived_at IS NULL AND (m.role>5 OR p.guest_view_all_features OR i.created_by_id=$2) AND (i.name ILIKE '%'||$3||'%' OR (p.identifier||'-'||i.sequence_id::text) ILIKE '%'||$3||'%') AND ($4::uuid IS NULL OR $6 OR p.id=$4) ORDER BY i.created_at DESC LIMIT 100"
            }
            "cycle" => {
                "SELECT jsonb_build_object('name',c.name,'id',c.id,'project_id',p.id,'project__identifier',p.identifier,'workspace__slug',$5::text) FROM cycles c JOIN projects p ON p.id=c.project_id AND p.workspace_id=c.workspace_id JOIN project_members m ON m.project_id=p.id AND m.workspace_id=p.workspace_id WHERE c.workspace_id=$1 AND m.member_id=$2 AND m.is_active AND m.deleted_at IS NULL AND c.deleted_at IS NULL AND p.deleted_at IS NULL AND p.archived_at IS NULL AND c.name ILIKE '%'||$3||'%' AND ($4::uuid IS NULL OR $6 OR p.id=$4) ORDER BY c.created_at DESC LIMIT 100"
            }
            "module" => {
                "SELECT jsonb_build_object('name',c.name,'id',c.id,'project_id',p.id,'project__identifier',p.identifier,'workspace__slug',$5::text) FROM modules c JOIN projects p ON p.id=c.project_id AND p.workspace_id=c.workspace_id JOIN project_members m ON m.project_id=p.id AND m.workspace_id=p.workspace_id WHERE c.workspace_id=$1 AND m.member_id=$2 AND m.is_active AND m.deleted_at IS NULL AND c.deleted_at IS NULL AND p.deleted_at IS NULL AND p.archived_at IS NULL AND c.name ILIKE '%'||$3||'%' AND ($4::uuid IS NULL OR $6 OR p.id=$4) ORDER BY c.created_at DESC LIMIT 100"
            }
            "page" | "issue_view" | "intake" => {
                results.insert(entity.to_owned(), serde_json::json!([]));
                continue;
            }
            _ => continue,
        };
        let sql = sql.replace(
            "$4::uuid IS NOT NULL AND $5::text",
            "($4::uuid IS NULL OR $4::uuid IS NOT NULL) AND $5::text",
        );
        let rows: Vec<Value> = sqlx::query_scalar(&sql)
            .bind(ws.workspace_id)
            .bind(user.id)
            .bind(search)
            .bind(project)
            .bind(&slug)
            .bind(wide)
            .fetch_all(&st.db)
            .await?;
        results.insert(entity.to_owned(), Value::Array(rows));
    }
    Ok(Json(serde_json::json!({"results":results})))
}

async fn archive_detail(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let sql = format!(
        "SELECT {} || jsonb_build_object('description_html',i.description_html) FROM issues i WHERE i.id=$1 AND i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL AND i.archived_at IS NOT NULL",
        workitems::ITEM_JSON
    );
    json_row(
        sqlx::query_scalar(&sql).bind(iid).bind(pid).bind(p.workspace_id()),
        &st.db,
    )
    .await
}
async fn archive_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let date:Option<chrono::NaiveDate>=sqlx::query_scalar("UPDATE issues i SET archived_at=CURRENT_DATE,updated_at=now(),updated_by_id=$4 WHERE i.id=$1 AND i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL AND i.archived_at IS NULL AND EXISTS(SELECT 1 FROM states s WHERE s.id=i.state_id AND s.project_id=$2 AND s.workspace_id=$3 AND s.deleted_at IS NULL AND s.group IN ('completed','cancelled')) RETURNING archived_at").bind(iid).bind(pid).bind(p.workspace_id()).bind(user.id).fetch_optional(&st.db).await?;
    Ok(Json(
        serde_json::json!({"archived_at":date.ok_or_else(||ApiError::bad("Can only archive completed or cancelled issues"))?}),
    ))
}
async fn unarchive_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
) -> ApiResult<axum::http::StatusCode> {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let r=sqlx::query("UPDATE issues SET archived_at=NULL,updated_at=now(),updated_by_id=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND archived_at IS NOT NULL").bind(iid).bind(pid).bind(p.workspace_id()).bind(user.id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}
async fn bulk_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Json(body): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN])?;
    let ids = uuid_list(&body, "issue_ids")?;
    if ids.is_empty() {
        return Err(ApiError::bad("Issue IDs are required"));
    }
    let mut tx = st.db.begin().await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(&format!(
        "SELECT i.id FROM issues i WHERE i.id=ANY($1) AND i.project_id=$2 AND i.workspace_id=$3 AND {ACTIVE}"
    ))
    .bind(&ids)
    .bind(pid)
    .bind(p.workspace_id())
    .fetch_all(&mut *tx)
    .await?;
    for sql in [
        "UPDATE cycle_issues SET deleted_at=now(),updated_at=now() WHERE issue_id=ANY($1) AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
        "UPDATE module_issues SET deleted_at=now(),updated_at=now() WHERE issue_id=ANY($1) AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
        "UPDATE issues SET deleted_at=now(),updated_at=now() WHERE id=ANY($1) AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    ] {
        sqlx::query(sql)
            .bind(&ids)
            .bind(pid)
            .bind(p.workspace_id())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(
        serde_json::json!({"message":format!("{} issues were deleted",ids.len())}),
    ))
}
async fn bulk_operation(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid)): Path<(String, Uuid)>,
    Json(body): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, pid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let ids = uuid_list(&body, "issue_ids")?;
    if ids.is_empty() {
        return Err(ApiError::bad("Issue IDs are required"));
    }
    let props = body
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError::bad("Properties required"))?;
    if props
        .keys()
        .any(|k| !["state_id", "priority", "start_date", "target_date"].contains(&k.as_str()))
    {
        return Err(ApiError::bad("Unsupported bulk property"));
    }
    let state = optional_uuid(&body["properties"], "state_id")?;
    let start = optional_date(&body["properties"], "start_date")?;
    let end = optional_date(&body["properties"], "target_date")?;
    let priority = s(&body["properties"], "priority");
    if priority.is_some_and(|v| !["none", "urgent", "high", "medium", "low"].contains(&v)) {
        return Err(ApiError::bad("Invalid priority"));
    }
    let mut tx = st.db.begin().await?;
    if let Some(id) = state {
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM states WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND NOT is_triage)").bind(id).bind(pid).bind(p.workspace_id()).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(ApiError::bad("Invalid state"));
        }
    }
    let count=sqlx::query(&format!("UPDATE issues i SET state_id=COALESCE($4,state_id),priority=COALESCE($5,priority),start_date=CASE WHEN $6 THEN $7 ELSE start_date END,target_date=CASE WHEN $8 THEN $9 ELSE target_date END,completed_at=CASE WHEN $4::uuid IS NULL THEN completed_at WHEN (SELECT s.group FROM states s WHERE s.id=$4 AND s.project_id=$2 AND s.workspace_id=$3) IN ('completed','cancelled') THEN now() ELSE NULL END,updated_at=now(),updated_by_id=$10 WHERE i.id=ANY($1) AND i.project_id=$2 AND i.workspace_id=$3 AND {ACTIVE} AND (CASE WHEN $6 THEN $7 ELSE start_date END IS NULL OR CASE WHEN $8 THEN $9 ELSE target_date END IS NULL OR CASE WHEN $6 THEN $7 ELSE start_date END <= CASE WHEN $8 THEN $9 ELSE target_date END)")).bind(&ids).bind(pid).bind(p.workspace_id()).bind(state).bind(priority).bind(props.contains_key("start_date")).bind(start).bind(props.contains_key("target_date")).bind(end).bind(user.id).execute(&mut *tx).await?.rows_affected();
    if count as usize != ids.len() {
        return Err(ApiError::bad("Invalid issues or date range"));
    }
    tx.commit().await?;
    Ok(Json(serde_json::json!({"updated":count})))
}
async fn identifier_detail(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, ident)): Path<(String, String)>,
) -> ApiResult {
    let (project, seq) = ident
        .rsplit_once('-')
        .ok_or_else(|| ApiError::bad("Invalid issue identifier"))?;
    let seq: i32 = seq.parse().map_err(|_| ApiError::bad("Invalid issue identifier"))?;
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let pid: Uuid = sqlx::query_scalar(
        "SELECT id FROM projects WHERE workspace_id=$1 AND upper(identifier)=upper($2) AND deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .bind(project)
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;
    let p = require_project(&st.db, &user, &slug, pid).await?;
    let id:Uuid=sqlx::query_scalar("SELECT id FROM issues WHERE workspace_id=$1 AND project_id=$2 AND sequence_id=$3 AND deleted_at IS NULL AND (NOT $4 OR created_by_id=$5)").bind(ws.workspace_id).bind(pid).bind(seq).bind(p.restricted_guest()).bind(user.id).fetch_optional(&st.db).await?.ok_or_else(ApiError::not_found)?;
    let sql = format!(
        "SELECT {} || jsonb_build_object('description_html',i.description_html) FROM issues i WHERE i.workspace_id=$1 AND i.project_id=$2 AND i.id=$3 AND i.deleted_at IS NULL",
        workitems::ITEM_JSON
    );
    json_row(
        sqlx::query_scalar(&sql).bind(ws.workspace_id).bind(pid).bind(id),
        &st.db,
    )
    .await
}

async fn subscription_status(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    let subscribed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM issue_subscribers WHERE workspace_id=$1 AND project_id=$2 AND issue_id=$3 AND subscriber_id=$4 AND deleted_at IS NULL)").bind(p.workspace_id()).bind(pid).bind(iid).bind(user.id).fetch_one(&st.db).await?;
    Ok(Json(serde_json::json!({"subscribed":subscribed})))
}
async fn add_subscriber(
    st: AppState,
    user: CurrentUser,
    path: IssuePath,
    subscriber: Uuid,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let (slug, pid, iid) = path;
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    let v:Option<Value>=sqlx::query_scalar("INSERT INTO issue_subscribers (workspace_id,project_id,issue_id,subscriber_id,created_by_id,updated_by_id) SELECT $1,$2,$3,$4,$5,$5 WHERE EXISTS(SELECT 1 FROM project_members WHERE workspace_id=$1 AND project_id=$2 AND member_id=$4 AND is_active AND deleted_at IS NULL) AND NOT EXISTS(SELECT 1 FROM issue_subscribers WHERE workspace_id=$1 AND project_id=$2 AND issue_id=$3 AND subscriber_id=$4 AND deleted_at IS NULL) RETURNING (to_jsonb(issue_subscribers)-'workspace_id'-'project_id'-'issue_id'-'subscriber_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('workspace',workspace_id,'project',project_id,'issue',issue_id,'subscriber',subscriber_id,'created_by',created_by_id,'updated_by',updated_by_id)").bind(p.workspace_id()).bind(pid).bind(iid).bind(subscriber).bind(user.id).fetch_optional(&st.db).await?;
    created(v.ok_or_else(|| ApiError::bad("Subscriber is invalid or already subscribed"))?)
}
async fn subscribe(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(path): Path<IssuePath>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let id = user.id;
    add_subscriber(st, user, path, id).await
}
async fn subscriber_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(path): Path<IssuePath>,
    Json(body): Json<Value>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let id = uuid_of(&body, "subscriber").ok_or_else(|| ApiError::bad("Invalid subscriber"))?;
    add_subscriber(st, user, path, id).await
}
async fn remove_subscriber(
    st: AppState,
    user: CurrentUser,
    path: IssuePath,
    subscriber: Uuid,
) -> ApiResult<axum::http::StatusCode> {
    let (slug, pid, iid) = path;
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    if subscriber != user.id {
        p.require(&[ADMIN, MEMBER])?;
    }
    let r=sqlx::query("UPDATE issue_subscribers SET deleted_at=now(),updated_at=now(),updated_by_id=$5 WHERE workspace_id=$1 AND project_id=$2 AND issue_id=$3 AND subscriber_id=$4 AND deleted_at IS NULL").bind(p.workspace_id()).bind(pid).bind(iid).bind(subscriber).bind(user.id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}
async fn unsubscribe(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(path): Path<IssuePath>,
) -> ApiResult<axum::http::StatusCode> {
    let id = user.id;
    remove_subscriber(st, user, path, id).await
}
async fn subscriber_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid, id)): Path<CommentPath>,
) -> ApiResult<axum::http::StatusCode> {
    remove_subscriber(st, user, (slug, pid, iid), id).await
}
async fn subscribers(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    json_rows(sqlx::query_scalar("SELECT (to_jsonb(m)-'member_id'-'workspace_id'-'project_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('member',m.member_id,'workspace',m.workspace_id,'project',m.project_id,'created_by',m.created_by_id,'updated_by',m.updated_by_id,'member_detail',jsonb_build_object('id',u.id,'display_name',u.display_name,'first_name',u.first_name,'last_name',u.last_name,'avatar',u.avatar,'is_bot',u.is_bot)) FROM project_members m JOIN users u ON u.id=m.member_id WHERE m.workspace_id=$1 AND m.project_id=$2 AND m.is_active AND m.deleted_at IS NULL").bind(p.workspace_id()).bind(pid),&st.db).await
}

async fn relations(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    let sql = format!(
        "SELECT jsonb_build_object('kind',CASE WHEN r.relation_type='blocked_by' AND r.related_issue_id=$1 THEN 'blocking' WHEN r.relation_type='start_before' AND r.related_issue_id=$1 THEN 'start_after' WHEN r.relation_type='finish_before' AND r.related_issue_id=$1 THEN 'finish_after' ELSE r.relation_type END,'item',{item}) FROM issue_relations r JOIN issues i ON i.id=CASE WHEN r.issue_id=$1 THEN r.related_issue_id ELSE r.issue_id END AND i.workspace_id=$3 WHERE r.workspace_id=$3 AND (r.issue_id=$1 OR r.related_issue_id=$1) AND r.deleted_at IS NULL AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft AND EXISTS(SELECT 1 FROM project_members pm WHERE pm.project_id=i.project_id AND pm.workspace_id=$3 AND pm.member_id=$4 AND pm.is_active AND pm.deleted_at IS NULL) AND (NOT $5 OR i.created_by_id=$4)",
        item = workitems::ITEM_JSON
    );
    let sql = sql.replace(
        "r.workspace_id=$3 AND (r.issue_id",
        "r.workspace_id=$3 AND $2::uuid IS NOT NULL AND (r.issue_id",
    );
    let rows: Vec<Value> = sqlx::query_scalar(&sql)
        .bind(iid)
        .bind(pid)
        .bind(p.workspace_id())
        .bind(user.id)
        .bind(p.restricted_guest())
        .fetch_all(&st.db)
        .await?;
    let mut out = serde_json::Map::new();
    for k in [
        "blocking",
        "blocked_by",
        "duplicate",
        "relates_to",
        "start_after",
        "start_before",
        "finish_after",
        "finish_before",
    ] {
        out.insert(k.into(), serde_json::json!([]));
    }
    for row in rows {
        if let Some(k) = row["kind"].as_str() {
            if let Some(v) = out.get_mut(k).and_then(Value::as_array_mut) {
                let mut item = row["item"].clone();
                item["relation_type"] = Value::String(k.into());
                v.push(item);
            }
        }
    }
    Ok(Json(Value::Object(out)))
}
async fn relation_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
    Json(body): Json<Value>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let kind = s(&body, "relation_type").ok_or_else(|| ApiError::bad("Relation type required"))?;
    let (stored, reverse) = match kind {
        "blocking" => ("blocked_by", true),
        "blocked_by" => ("blocked_by", false),
        "start_after" => ("start_before", true),
        "start_before" => ("start_before", false),
        "finish_after" => ("finish_before", true),
        "finish_before" => ("finish_before", false),
        "duplicate" => ("duplicate", false),
        "relates_to" => ("relates_to", false),
        _ => return Err(ApiError::bad("Invalid relation type")),
    };
    let ids = uuid_list(&body, "issues")?;
    let mut tx = st.db.begin().await?;
    let mut rows = Vec::new();
    for id in ids {
        if id == iid {
            return Err(ApiError::bad("Cannot relate an issue to itself"));
        }
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM issues i JOIN project_members pm ON pm.project_id=i.project_id AND pm.workspace_id=i.workspace_id WHERE i.id=$1 AND i.workspace_id=$2 AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft AND pm.member_id=$3 AND pm.is_active AND pm.deleted_at IS NULL AND (NOT $4 OR i.created_by_id=$3))").bind(id).bind(p.workspace_id()).bind(user.id).bind(p.restricted_guest()).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(ApiError::bad("Invalid related issue"));
        }
        let (a, b) = if reverse { (id, iid) } else { (iid, id) };
        let row:Option<Value>=sqlx::query_scalar("INSERT INTO issue_relations (workspace_id,project_id,issue_id,related_issue_id,relation_type,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$5,$6,$6) ON CONFLICT DO NOTHING RETURNING (to_jsonb(issue_relations)-'workspace_id'-'project_id'-'issue_id'-'related_issue_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('workspace',workspace_id,'project',project_id,'issue',issue_id,'related_issue',related_issue_id,'created_by',created_by_id,'updated_by',updated_by_id)").bind(p.workspace_id()).bind(pid).bind(a).bind(b).bind(stored).bind(user.id).fetch_optional(&mut *tx).await?;
        if let Some(v) = row {
            rows.push(v);
        }
    }
    tx.commit().await?;
    created(Value::Array(rows))
}
async fn relation_remove(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
    Json(body): Json<Value>,
) -> ApiResult<axum::http::StatusCode> {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let other = uuid_of(&body, "related_issue").ok_or_else(|| ApiError::bad("Invalid related issue"))?;
    let r=sqlx::query("UPDATE issue_relations SET deleted_at=now(),updated_at=now(),updated_by_id=$4 WHERE id=(SELECT id FROM issue_relations WHERE workspace_id=$1 AND ((issue_id=$2 AND related_issue_id=$3) OR (issue_id=$3 AND related_issue_id=$2)) AND deleted_at IS NULL LIMIT 1)").bind(p.workspace_id()).bind(iid).bind(other).bind(user.id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}
async fn sub_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    let sql = format!(
        "SELECT {} || jsonb_build_object('state_group',(SELECT s.group FROM states s WHERE s.id=i.state_id)) FROM issues i WHERE i.parent_id=$1 AND i.project_id=$2 AND i.workspace_id=$3 AND {} AND (NOT $4 OR i.created_by_id=$5) ORDER BY i.created_at DESC",
        workitems::ITEM_JSON,
        ACTIVE
    );
    let rows: Vec<Value> = sqlx::query_scalar(&sql)
        .bind(iid)
        .bind(pid)
        .bind(p.workspace_id())
        .bind(p.restricted_guest())
        .bind(user.id)
        .fetch_all(&st.db)
        .await?;
    let mut groups = serde_json::Map::new();
    for row in &rows {
        let key = row["state_group"].as_str().unwrap_or("None");
        groups
            .entry(key.to_string())
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .unwrap()
            .push(row["id"].clone());
    }
    Ok(Json(serde_json::json!({"sub_issues":rows,"state_distribution":groups})))
}
async fn sub_issue_assign(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
    Json(body): Json<Value>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let ids = uuid_list(&body, "sub_issue_ids")?;
    if ids.is_empty() || ids.contains(&iid) {
        return Err(ApiError::bad("Invalid sub issue IDs"));
    }
    let mut tx = st.db.begin().await?;
    let cycle:bool=sqlx::query_scalar("WITH RECURSIVE ancestors AS (SELECT id,parent_id FROM issues WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL UNION SELECT i.id,i.parent_id FROM issues i JOIN ancestors a ON i.id=a.parent_id WHERE i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL) SELECT EXISTS(SELECT 1 FROM ancestors WHERE id=ANY($4))").bind(iid).bind(pid).bind(p.workspace_id()).bind(&ids).fetch_one(&mut *tx).await?;
    if cycle {
        return Err(ApiError::bad("Parent cycle"));
    }
    let r=sqlx::query("UPDATE issues SET parent_id=$1,updated_at=now(),updated_by_id=$5 WHERE id=ANY($4) AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND archived_at IS NULL AND NOT is_draft").bind(iid).bind(pid).bind(p.workspace_id()).bind(&ids).bind(user.id).execute(&mut *tx).await?;
    if r.rows_affected() as usize != ids.len() {
        return Err(ApiError::bad("Invalid sub issue IDs"));
    }
    tx.commit().await?;
    sub_issues(State(st), user, Path((slug, pid, iid))).await
}

const LINK_JSON: &str = "(to_jsonb(l)-'issue_id'-'project_id'-'workspace_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('issue',l.issue_id,'project',l.project_id,'workspace',l.workspace_id,'created_by',l.created_by_id,'updated_by',l.updated_by_id,'created_by_detail',(SELECT jsonb_build_object('id',u.id,'first_name',u.first_name,'last_name',u.last_name,'display_name',u.display_name,'avatar',u.avatar,'is_bot',u.is_bot) FROM users u WHERE u.id=l.created_by_id))";

fn link_url(body: &Value) -> Result<Option<String>, ApiError> {
    body.get("url")
        .map(|v| {
            let raw = v.as_str().ok_or_else(|| ApiError::bad("Invalid URL"))?;
            let value = if raw.starts_with("http://") || raw.starts_with("https://") {
                raw.to_owned()
            } else {
                format!("http://{raw}")
            };
            let parsed = url::Url::parse(&value).map_err(|_| ApiError::bad("Invalid URL"))?;
            if parsed.host_str().is_none() || !parsed.username().is_empty() || parsed.password().is_some() {
                return Err(ApiError::bad("Invalid URL"));
            }
            Ok(value)
        })
        .transpose()
}

async fn links(State(st): State<AppState>, user: CurrentUser, Path((slug, pid, iid)): Path<IssuePath>) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    json_rows(sqlx::query_scalar(&format!("SELECT {LINK_JSON} FROM issue_links l WHERE l.issue_id=$1 AND l.project_id=$2 AND l.workspace_id=$3 AND l.deleted_at IS NULL ORDER BY l.created_at DESC")).bind(iid).bind(pid).bind(p.workspace_id()),&st.db).await
}
async fn link_detail(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid, id)): Path<CommentPath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    json_row(sqlx::query_scalar(&format!("SELECT {LINK_JSON} FROM issue_links l WHERE l.issue_id=$1 AND l.project_id=$2 AND l.workspace_id=$3 AND l.id=$4 AND l.deleted_at IS NULL")).bind(iid).bind(pid).bind(p.workspace_id()).bind(id),&st.db).await
}
async fn link_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
    Json(body): Json<Value>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let url = link_url(&body)?.ok_or_else(|| ApiError::bad("URL required"))?;
    let title = s(&body, "title");
    if title.is_some_and(|v| v.chars().count() > 255) {
        return Err(ApiError::bad("Title too long"));
    }
    let id: Uuid=sqlx::query_scalar("INSERT INTO issue_links (workspace_id,project_id,issue_id,url,title,created_by_id,updated_by_id) SELECT $1,$2,$3,$4,$5,$6,$6 WHERE NOT EXISTS (SELECT 1 FROM issue_links WHERE workspace_id=$1 AND project_id=$2 AND issue_id=$3 AND url=$4 AND deleted_at IS NULL) RETURNING id").bind(p.workspace_id()).bind(pid).bind(iid).bind(url).bind(title).bind(user.id).fetch_optional(&st.db).await?.ok_or_else(||ApiError::bad("URL already exists"))?;
    let Json(v) = link_detail(State(st), user, Path((slug, pid, iid, id))).await?;
    created(v)
}
async fn link_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid, id)): Path<CommentPath>,
    Json(body): Json<Value>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let url = link_url(&body)?;
    let title = s(&body, "title");
    if title.is_some_and(|v| v.chars().count() > 255) {
        return Err(ApiError::bad("Title too long"));
    }
    let r=sqlx::query("UPDATE issue_links SET url=COALESCE($5,url),title=COALESCE($6,title),updated_by_id=$7,updated_at=now() WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL AND NOT EXISTS (SELECT 1 FROM issue_links other WHERE other.issue_id=$2 AND other.project_id=$3 AND other.workspace_id=$4 AND other.id<>$1 AND other.url=$5 AND other.deleted_at IS NULL)").bind(id).bind(iid).bind(pid).bind(p.workspace_id()).bind(url).bind(title).bind(user.id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    link_detail(State(st), user, Path((slug, pid, iid, id))).await
}
async fn link_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid, id)): Path<CommentPath>,
) -> ApiResult<axum::http::StatusCode> {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let r=sqlx::query("UPDATE issue_links SET deleted_at=now(),updated_at=now(),updated_by_id=$5 WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL").bind(id).bind(iid).bind(pid).bind(p.workspace_id()).bind(user.id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn reaction_context(
    st: &AppState,
    user: &CurrentUser,
    slug: &str,
    pid: Uuid,
    id: Uuid,
    comment: bool,
) -> Result<ProjectCtx, ApiError> {
    if comment {
        let p = require_project(&st.db, user, slug, pid).await?;
        p.require(&[ADMIN, MEMBER, GUEST])?;
        let issue:Uuid=sqlx::query_scalar("SELECT issue_id FROM issue_comments WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL").bind(id).bind(pid).bind(p.workspace_id()).fetch_optional(&st.db).await?.ok_or_else(ApiError::not_found)?;
        visible_issue(st, user, slug, pid, issue).await
    } else {
        visible_issue(st, user, slug, pid, id).await
    }
}
async fn reactions(st: AppState, user: CurrentUser, path: IssuePath, comment: bool) -> ApiResult {
    let (slug, pid, id) = path;
    let p = reaction_context(&st, &user, &slug, pid, id, comment).await?;
    let sql = if comment {
        "SELECT (to_jsonb(r)-'actor_id'-'comment_id'-'project_id'-'workspace_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('actor',r.actor_id,'comment',r.comment_id,'project',r.project_id,'workspace',r.workspace_id,'created_by',r.created_by_id,'updated_by',r.updated_by_id,'display_name',u.display_name) FROM comment_reactions r JOIN users u ON u.id=r.actor_id WHERE r.comment_id=$1 AND r.project_id=$2 AND r.workspace_id=$3 AND r.deleted_at IS NULL ORDER BY r.created_at DESC"
    } else {
        "SELECT (to_jsonb(r)-'actor_id'-'issue_id'-'project_id'-'workspace_id'-'created_by_id'-'updated_by_id') || jsonb_build_object('actor',r.actor_id,'issue',r.issue_id,'project',r.project_id,'workspace',r.workspace_id,'created_by',r.created_by_id,'updated_by',r.updated_by_id,'actor_detail',jsonb_build_object('id',u.id,'display_name',u.display_name,'first_name',u.first_name,'last_name',u.last_name,'avatar',u.avatar,'is_bot',u.is_bot)) FROM issue_reactions r JOIN users u ON u.id=r.actor_id WHERE r.issue_id=$1 AND r.project_id=$2 AND r.workspace_id=$3 AND r.deleted_at IS NULL ORDER BY r.created_at DESC"
    };
    json_rows(
        sqlx::query_scalar(sql).bind(id).bind(pid).bind(p.workspace_id()),
        &st.db,
    )
    .await
}
async fn reaction_create(
    st: AppState,
    user: CurrentUser,
    path: IssuePath,
    body: Value,
    comment: bool,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let (slug, pid, id) = path;
    let p = reaction_context(&st, &user, &slug, pid, id, comment).await?;
    let code = s(&body, "reaction")
        .filter(|v| !v.is_empty() && v.len() <= 255)
        .ok_or_else(|| ApiError::bad("Invalid reaction"))?;
    let sql = if comment {
        "INSERT INTO comment_reactions (workspace_id,project_id,comment_id,actor_id,reaction,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$5,$4,$4) RETURNING id"
    } else {
        "INSERT INTO issue_reactions (workspace_id,project_id,issue_id,actor_id,reaction,created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$5,$4,$4) RETURNING id"
    };
    let rid: Uuid = sqlx::query_scalar(sql)
        .bind(p.workspace_id())
        .bind(pid)
        .bind(id)
        .bind(user.id)
        .bind(code)
        .fetch_one(&st.db)
        .await?;
    let Json(Value::Array(rows)) = reactions(st, user, (slug, pid, id), comment).await? else {
        return Err(ApiError::not_found());
    };
    created(
        rows.into_iter()
            .find(|v| v["id"] == rid.to_string())
            .ok_or_else(ApiError::not_found)?,
    )
}
async fn reaction_delete(
    st: AppState,
    user: CurrentUser,
    path: (String, Uuid, Uuid, String),
    comment: bool,
) -> ApiResult<axum::http::StatusCode> {
    let (slug, pid, id, code) = path;
    let p = reaction_context(&st, &user, &slug, pid, id, comment).await?;
    let sql = if comment {
        "UPDATE comment_reactions SET deleted_at=now(),updated_at=now() WHERE comment_id=$1 AND project_id=$2 AND workspace_id=$3 AND actor_id=$4 AND reaction=$5 AND deleted_at IS NULL"
    } else {
        "UPDATE issue_reactions SET deleted_at=now(),updated_at=now() WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND actor_id=$4 AND reaction=$5 AND deleted_at IS NULL"
    };
    let r = sqlx::query(sql)
        .bind(id)
        .bind(pid)
        .bind(p.workspace_id())
        .bind(user.id)
        .bind(code)
        .execute(&st.db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}
async fn issue_reactions(State(s): State<AppState>, u: CurrentUser, Path(p): Path<IssuePath>) -> ApiResult {
    reactions(s, u, p, false).await
}
async fn comment_reactions(State(s): State<AppState>, u: CurrentUser, Path(p): Path<IssuePath>) -> ApiResult {
    reactions(s, u, p, true).await
}
async fn issue_reaction_create(
    State(s): State<AppState>,
    u: CurrentUser,
    Path(p): Path<IssuePath>,
    Json(b): Json<Value>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    reaction_create(s, u, p, b, false).await
}
async fn comment_reaction_create(
    State(s): State<AppState>,
    u: CurrentUser,
    Path(p): Path<IssuePath>,
    Json(b): Json<Value>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    reaction_create(s, u, p, b, true).await
}
async fn issue_reaction_delete(
    State(s): State<AppState>,
    u: CurrentUser,
    Path(p): Path<(String, Uuid, Uuid, String)>,
) -> ApiResult<axum::http::StatusCode> {
    reaction_delete(s, u, p, false).await
}
async fn comment_reaction_delete(
    State(s): State<AppState>,
    u: CurrentUser,
    Path(p): Path<(String, Uuid, Uuid, String)>,
) -> ApiResult<axum::http::StatusCode> {
    reaction_delete(s, u, p, true).await
}

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
