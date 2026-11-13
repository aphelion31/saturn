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
        .route("/api/workspaces/{slug}/draft-issues/", get(draft_list).post(draft_create))
        .route("/api/workspaces/{slug}/draft-issues/{draft_id}/", get(draft_detail).patch(draft_update).delete(draft_delete))
        .route("/api/workspaces/{slug}/draft-to-issue/{draft_id}/", axum::routing::post(draft_convert))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/",
            get(list).post(create),
        )
        .route("/api/workspaces/{slug}/projects/{project_id}/issues/list/", get(list))
        .route("/api/workspaces/{slug}/projects/{project_id}/issues-detail/", get(list))
        .route("/api/workspaces/{slug}/projects/{project_id}/deleted-issues/", get(deleted_issues))
        .route("/api/workspaces/{slug}/projects/{project_id}/issue-dates/", axum::routing::post(issue_dates))
        .route("/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/issue-attachments/{attachment_id}/", axum::routing::delete(legacy_attachment_delete))
        .route("/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/meta/", get(issue_meta))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/issue-attachments/",
            get(legacy_attachments)
                .post(crate::assets::legacy_attachment_upload)
                .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route("/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/versions/", get(issue_versions))
        .route("/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/versions/{version_id}/", get(issue_version))
        .route("/api/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/description-versions/", get(description_versions))
        .route("/api/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/description-versions/{version_id}/", get(description_version))
        .route("/api/workspaces/{slug}/projects/{project_id}/intake-work-items/{issue_id}/description-versions/", get(description_versions))
        .route("/api/workspaces/{slug}/projects/{project_id}/intake-work-items/{issue_id}/description-versions/{version_id}/", get(description_version))
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
            "/api/workspaces/{slug}/projects/{project_id}/bulk-archive-issues/",
            axum::routing::post(bulk_archive),
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

const DRAFT_JSON: &str = "jsonb_build_object('id',d.id,'name',d.name,'state_id',d.state_id,'sort_order',d.sort_order,'completed_at',d.completed_at,'estimate_point',d.estimate_point_id,'priority',d.priority,'start_date',d.start_date,'target_date',d.target_date,'project_id',d.project_id,'parent_id',d.parent_id,'cycle_id',(SELECT c.cycle_id FROM draft_issue_cycles c WHERE c.draft_issue_id=d.id AND c.workspace_id=d.workspace_id AND c.deleted_at IS NULL LIMIT 1),'module_ids',COALESCE((SELECT jsonb_agg(m.module_id) FROM draft_issue_modules m WHERE m.draft_issue_id=d.id AND m.workspace_id=d.workspace_id AND m.deleted_at IS NULL),'[]'::jsonb),'label_ids',COALESCE((SELECT jsonb_agg(l.label_id) FROM draft_issue_labels l WHERE l.draft_issue_id=d.id AND l.workspace_id=d.workspace_id AND l.deleted_at IS NULL),'[]'::jsonb),'assignee_ids',COALESCE((SELECT jsonb_agg(a.assignee_id) FROM draft_issue_assignees a WHERE a.draft_issue_id=d.id AND a.workspace_id=d.workspace_id AND a.deleted_at IS NULL),'[]'::jsonb),'created_at',d.created_at,'updated_at',d.updated_at,'created_by',d.created_by_id,'updated_by',d.updated_by_id,'type_id',d.type_id,'description_html',d.description_html)";

async fn draft_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let size = q
        .get("per_page")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(1000)
        .clamp(1, 1000);
    let page = q
        .get("cursor")
        .and_then(|s| s.split(':').nth(1))
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
        .clamp(0, i64::MAX / size);
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM draft_issues WHERE workspace_id=$1 AND created_by_id=$2 AND deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    let results: Vec<Value> = sqlx::query_scalar(&format!("SELECT {DRAFT_JSON} FROM draft_issues d WHERE d.workspace_id=$1 AND d.created_by_id=$2 AND d.deleted_at IS NULL ORDER BY d.created_at DESC,d.id DESC LIMIT $3 OFFSET $4")).bind(ws.workspace_id).bind(user.id).bind(size).bind(page*size).fetch_all(&st.db).await?;
    Ok(Json(
        serde_json::json!({"grouped_by":null,"sub_grouped_by":null,"total_count":total,"next_cursor":format!("{size}:{}:0",page+1),"prev_cursor":format!("{size}:{}:1",page-1),"next_page_results":page.saturating_add(1).saturating_mul(size)<total,"prev_page_results":page>0,"count":results.len(),"total_pages":(total+size-1)/size,"total_results":total,"extra_stats":null,"results":results}),
    ))
}

async fn draft_detail(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(sqlx::query_scalar(&format!("SELECT {DRAFT_JSON} FROM draft_issues d WHERE d.workspace_id=$1 AND d.created_by_id=$2 AND d.id=$3 AND d.deleted_at IS NULL")).bind(ws.workspace_id).bind(user.id).bind(id),&st.db).await
}

async fn draft_save(
    st: &AppState,
    user: &CurrentUser,
    slug: &str,
    id: Option<Uuid>,
    mut body: Value,
) -> Result<Value, ApiError> {
    let ws = require_workspace(&st.db, user, slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    if body.as_object().is_none_or(|fields| {
        fields.keys().any(|key| {
            ![
                "project_id",
                "name",
                "description_html",
                "description_json",
                "priority",
                "start_date",
                "target_date",
                "sort_order",
                "state_id",
                "parent_id",
                "estimate_point",
                "type_id",
                "assignee_ids",
                "label_ids",
                "module_ids",
                "cycle_id",
            ]
            .contains(&key.as_str())
        })
    }) {
        return Err(ApiError::bad("Unsupported draft field"));
    }
    let existing: Option<Option<Uuid>> = if let Some(id) = id {
        Some(sqlx::query_scalar("SELECT project_id FROM draft_issues WHERE id=$1 AND workspace_id=$2 AND created_by_id=$3 AND deleted_at IS NULL").bind(id).bind(ws.workspace_id).bind(user.id).fetch_optional(&st.db).await?.ok_or_else(ApiError::not_found)?)
    } else {
        None
    };
    let supplied = optional_uuid(&body, "project_id")?;
    let project = existing.unwrap_or(supplied);
    if existing.is_some() && body.get("project_id").is_some() && supplied != project {
        return Err(ApiError::bad("Draft project cannot be changed"));
    }
    if let Some(project) = project {
        require_project(&st.db, user, slug, project)
            .await?
            .require(&[ADMIN, MEMBER])?;
    }
    if let Some(html) = body.get("description_html") {
        let clean = sanitize_html(html.as_str().ok_or_else(|| ApiError::bad("Invalid description_html"))?)?;
        body["description_stripped"] = Value::String(strip_tags(&clean));
        body["description_html"] = Value::String(clean);
    }
    if let Some(name) = body.get("name") {
        if !name.is_null() && name.as_str().is_none_or(|s| s.chars().count() > 255) {
            return Err(ApiError::bad("Invalid name"));
        }
    }
    if let Some(priority) = body.get("priority") {
        if !priority
            .as_str()
            .is_some_and(|p| ["none", "urgent", "high", "medium", "low"].contains(&p))
        {
            return Err(ApiError::bad("Invalid priority"));
        }
    }
    let start = optional_date(&body, "start_date")?;
    let target = optional_date(&body, "target_date")?;
    if start.zip(target).is_some_and(|(a, b)| a > b) {
        return Err(ApiError::bad("Start date cannot exceed target date"));
    }
    let mut tx = st.db.begin().await?;
    for (field, sql) in [
        (
            "state_id",
            "SELECT EXISTS(SELECT 1 FROM states WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
        ),
        (
            "parent_id",
            "SELECT EXISTS(SELECT 1 FROM issues WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
        ),
        (
            "estimate_point",
            "SELECT EXISTS(SELECT 1 FROM estimate_points WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
        ),
        (
            "type_id",
            "SELECT EXISTS(SELECT 1 FROM issue_types WHERE id=$1 AND $2::uuid IS NOT NULL AND workspace_id=$3 AND deleted_at IS NULL)",
        ),
    ] {
        if let Some(value) = optional_uuid(&body, field)? {
            let valid: bool = sqlx::query_scalar(sql)
                .bind(value)
                .bind(project)
                .bind(ws.workspace_id)
                .fetch_one(&mut *tx)
                .await?;
            if !valid {
                return Err(ApiError::bad("Invalid related object"));
            }
        }
    }
    let id = if let Some(id) = id {
        id
    } else {
        sqlx::query_scalar("INSERT INTO draft_issues (workspace_id,project_id,created_by_id,updated_by_id) VALUES ($1,$2,$3,$3) RETURNING id").bind(ws.workspace_id).bind(project).bind(user.id).fetch_one(&mut *tx).await?
    };
    let fields = [
        ("name", "text"),
        ("description_html", "text"),
        ("description_stripped", "text"),
        ("description_json", "jsonb"),
        ("priority", "text"),
        ("start_date", "date"),
        ("target_date", "date"),
        ("sort_order", "float8"),
        ("state_id", "uuid"),
        ("parent_id", "uuid"),
        ("type_id", "uuid"),
    ];
    let mut sets = crate::users::patch_sets(&body, &fields, 4);
    if body.get("estimate_point").is_some() {
        sets.push("estimate_point_id=($4::jsonb->>'estimate_point')::uuid".into());
    }
    sets.push("updated_at=now()".into());
    sets.push("updated_by_id=$2".into());
    sqlx::query(&format!(
        "UPDATE draft_issues SET {} WHERE id=$1 AND created_by_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
        sets.join(",")
    ))
    .bind(id)
    .bind(user.id)
    .bind(ws.workspace_id)
    .bind(&body)
    .execute(&mut *tx)
    .await?;
    let dates: (Option<chrono::NaiveDate>,Option<chrono::NaiveDate>)=sqlx::query_as("SELECT start_date,target_date FROM draft_issues WHERE id=$1 AND workspace_id=$2 AND created_by_id=$3 AND deleted_at IS NULL").bind(id).bind(ws.workspace_id).bind(user.id).fetch_one(&mut *tx).await?;
    if dates.0.zip(dates.1).is_some_and(|(a, b)| a > b) {
        return Err(ApiError::bad("Start date cannot exceed target date"));
    }
    for (field, table, column, check) in [
        (
            "assignee_ids",
            "draft_issue_assignees",
            "assignee_id",
            "SELECT EXISTS(SELECT 1 FROM project_members WHERE member_id=$1 AND project_id=$2 AND workspace_id=$3 AND is_active AND role>=15 AND deleted_at IS NULL)",
        ),
        (
            "label_ids",
            "draft_issue_labels",
            "label_id",
            "SELECT EXISTS(SELECT 1 FROM labels WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
        ),
        (
            "module_ids",
            "draft_issue_modules",
            "module_id",
            "SELECT EXISTS(SELECT 1 FROM modules WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
        ),
        (
            "cycle_id",
            "draft_issue_cycles",
            "cycle_id",
            "SELECT EXISTS(SELECT 1 FROM cycles WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL)",
        ),
    ] {
        if body.get(field).is_none() {
            continue;
        }
        let values = if field == "cycle_id" {
            optional_uuid(&body, field)?.into_iter().collect()
        } else {
            uuid_list(&body, field)?
        };
        sqlx::query(&format!(
            "UPDATE {table} SET deleted_at=now() WHERE draft_issue_id=$1 AND workspace_id=$2 AND deleted_at IS NULL"
        ))
        .bind(id)
        .bind(ws.workspace_id)
        .execute(&mut *tx)
        .await?;
        let mut seen = std::collections::HashSet::new();
        for value in values {
            let valid: bool = sqlx::query_scalar(check)
                .bind(value)
                .bind(project)
                .bind(ws.workspace_id)
                .fetch_one(&mut *tx)
                .await?;
            if !valid || !seen.insert(value) {
                return Err(ApiError::bad("Invalid or duplicate related object"));
            }
            sqlx::query(&format!("INSERT INTO {table} (draft_issue_id,workspace_id,project_id,{column},created_by_id,updated_by_id) VALUES ($1,$2,$3,$4,$5,$5)")).bind(id).bind(ws.workspace_id).bind(project).bind(value).bind(user.id).execute(&mut *tx).await?;
        }
    }
    let result=sqlx::query_scalar(&format!("SELECT {DRAFT_JSON} FROM draft_issues d WHERE d.id=$1 AND d.workspace_id=$2 AND d.created_by_id=$3 AND d.deleted_at IS NULL")).bind(id).bind(ws.workspace_id).bind(user.id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(result)
}

async fn draft_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    created(draft_save(&st, &user, &slug, None, body).await?)
}
async fn draft_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(body): Json<Value>,
) -> ApiResult<axum::http::StatusCode> {
    draft_save(&st, &user, &slug, Some(id), body).await?;
    no_content()
}
async fn draft_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let n=sqlx::query("UPDATE draft_issues SET deleted_at=now() WHERE id=$1 AND workspace_id=$2 AND created_by_id=$3 AND deleted_at IS NULL").bind(id).bind(ws.workspace_id).bind(user.id).execute(&st.db).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn draft_convert(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(body): Json<Value>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let mut tx = st.db.begin().await?;
    let draft:Value=sqlx::query_scalar(&format!("SELECT {DRAFT_JSON} FROM draft_issues d WHERE d.id=$1 AND d.workspace_id=$2 AND d.created_by_id=$3 AND d.deleted_at IS NULL FOR UPDATE")).bind(id).bind(ws.workspace_id).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(ApiError::not_found)?;
    let project =
        uuid_of(&draft, "project_id").ok_or_else(|| ApiError::bad("Project is required to create an issue."))?;
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER])?;
    let mut input = serde_json::Map::new();
    for key in [
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
    ] {
        if let Some(v) = body.get(key).or_else(|| draft.get(key)) {
            if !v.is_null() {
                input.insert(key.into(), v.clone());
            }
        }
    }
    let result = create_issue(&mut tx, &user, &p, &Value::Object(input)).await?;
    let issue = uuid_of(&result, "id").ok_or_else(|| ApiError::bad("Missing issue id"))?;
    sqlx::query("UPDATE issues SET description_json=(SELECT description_json FROM draft_issues WHERE id=$2 AND workspace_id=$3 AND created_by_id=$5 AND deleted_at IS NULL) WHERE id=$1 AND workspace_id=$3 AND project_id=$4 AND deleted_at IS NULL")
        .bind(issue).bind(id).bind(ws.workspace_id).bind(project).bind(user.id).execute(&mut *tx).await?;
    sqlx::query("UPDATE file_assets SET issue_id=$1,draft_issue_id=NULL,entity_type='ISSUE_DESCRIPTION' WHERE draft_issue_id=$2 AND workspace_id=$3 AND project_id=$4 AND deleted_at IS NULL").bind(issue).bind(id).bind(ws.workspace_id).bind(project).execute(&mut *tx).await?;
    sqlx::query("UPDATE draft_issues SET deleted_at=now() WHERE id=$1 AND workspace_id=$2 AND created_by_id=$3 AND deleted_at IS NULL").bind(id).bind(ws.workspace_id).bind(user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    created(result)
}

async fn issue_dates(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project)): Path<(String, Uuid)>,
    Json(body): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER])?;
    let updates = body
        .get("updates")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad("Invalid updates"))?;
    let mut tx = st.db.begin().await?;
    for update in updates {
        let id = optional_uuid(update, "id")?.ok_or_else(|| ApiError::bad("Invalid issue id"))?;
        let start = optional_date(update, "start_date")?;
        let target = optional_date(update, "target_date")?;
        let dates: Option<(Option<chrono::NaiveDate>, Option<chrono::NaiveDate>)> = sqlx::query_as("SELECT start_date,target_date FROM issues WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL FOR UPDATE").bind(id).bind(project).bind(p.workspace_id()).fetch_optional(&mut *tx).await?;
        if let Some((old_start, old_target)) = dates {
            if start
                .or(old_start)
                .zip(target.or(old_target))
                .is_some_and(|(a, b)| a > b)
            {
                return Err(ApiError::bad_body(
                    serde_json::json!({"message":"Start date cannot exceed target date"}),
                ));
            }
            sqlx::query("UPDATE issues SET start_date=COALESCE($4,start_date),target_date=COALESCE($5,target_date),updated_at=now(),updated_by_id=$6 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL").bind(id).bind(project).bind(p.workspace_id()).bind(start).bind(target).bind(user.id).execute(&mut *tx).await?;
        }
    }
    // ponytail: asynchronous activity fan-out is omitted until a shared activity helper is available.
    tx.commit().await?;
    Ok(Json(serde_json::json!({"message":"Issues updated successfully"})))
}

async fn legacy_attachment_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue, id)): Path<CommentPath>,
) -> ApiResult<axum::http::StatusCode> {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    let owner: Option<Option<Uuid>> = sqlx::query_scalar("SELECT created_by_id FROM file_assets WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND entity_type='ISSUE_ATTACHMENT' AND deleted_at IS NULL").bind(id).bind(issue).bind(project).bind(p.workspace_id()).fetch_optional(&st.db).await?;
    if owner.ok_or_else(ApiError::not_found)? != Some(user.id) {
        p.require(&[ADMIN])?;
    }
    sqlx::query("UPDATE file_assets SET deleted_at=now(),is_deleted=true WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL").bind(id).bind(issue).bind(project).bind(p.workspace_id()).execute(&st.db).await?;
    no_content()
}

async fn issue_meta(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue)): Path<IssuePath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    json_row(sqlx::query_scalar("SELECT jsonb_build_object('sequence_id',i.sequence_id,'project_identifier',p.identifier) FROM issues i JOIN projects p ON p.id=i.project_id AND p.workspace_id=i.workspace_id WHERE i.id=$1 AND i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL AND p.deleted_at IS NULL").bind(issue).bind(project).bind(p.workspace_id()), &st.db).await
}

async fn deleted_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let after = q
        .get("updated_at__gt")
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| d.to_utc())
                .map_err(|_| ApiError::bad("Invalid updated_at__gt"))
        })
        .transpose()?;
    json_rows(sqlx::query_scalar("SELECT to_jsonb(id) FROM issues WHERE workspace_id=$1 AND project_id=$2 AND (deleted_at IS NOT NULL OR archived_at IS NOT NULL) AND ($3::timestamptz IS NULL OR updated_at>$3) AND (NOT $4 OR created_by_id=$5) ORDER BY created_at DESC").bind(p.workspace_id()).bind(project).bind(after).bind(p.restricted_guest()).bind(user.id), &st.db).await
}

async fn legacy_attachments(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue)): Path<IssuePath>,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    json_rows(sqlx::query_scalar("SELECT (to_jsonb(f)-'workspace_id'-'project_id'-'issue_id'-'created_by_id'-'updated_by_id'-'comment_id'-'page_id'-'user_id'-'draft_issue_id') || jsonb_build_object('workspace',f.workspace_id,'project',f.project_id,'issue',f.issue_id,'created_by',f.created_by_id,'updated_by',f.updated_by_id,'comment',f.comment_id,'page',f.page_id,'user',f.user_id,'draft_issue',f.draft_issue_id,'asset_url','/api/assets/v2/workspaces/' || $4 || '/projects/' || f.project_id || '/issues/' || f.issue_id || '/attachments/' || f.id || '/') FROM file_assets f WHERE f.workspace_id=$1 AND f.project_id=$2 AND f.issue_id=$3 AND f.entity_type='ISSUE_ATTACHMENT' AND f.deleted_at IS NULL ORDER BY f.created_at DESC").bind(p.workspace_id()).bind(project).bind(issue).bind(slug), &st.db).await
}

const VERSION_SUMMARY: &str = "jsonb_build_object('id',v.id,'workspace',v.workspace_id,'project',v.project_id,'issue',v.issue_id,'last_saved_at',v.last_saved_at,'owned_by',v.owned_by_id,'created_at',v.created_at,'updated_at',v.updated_at,'created_by',v.created_by_id,'updated_by',v.updated_by_id)";

async fn versions(
    st: AppState,
    user: CurrentUser,
    (slug, project, issue): IssuePath,
    q: HashMap<String, String>,
    description: bool,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    let (size, page) = if let Some(cursor) = q.get("cursor") {
        let parts: Vec<_> = cursor.split(':').collect();
        if parts.len() != 3 {
            return Err(ApiError::bad("Invalid cursor"));
        }
        let size = parts[0]
            .parse::<i64>()
            .map_err(|_| ApiError::bad("Invalid cursor"))?
            .clamp(1, 1000);
        let page = parts[1]
            .parse::<i64>()
            .map_err(|_| ApiError::bad("Invalid cursor"))?
            .max(0);
        parts[2].parse::<i64>().map_err(|_| ApiError::bad("Invalid cursor"))?;
        (size, page)
    } else {
        (1000, 0)
    };
    let offset = page.checked_mul(size).ok_or_else(|| ApiError::bad("Invalid cursor"))?;
    let table = if description {
        "issue_description_versions"
    } else {
        "issue_versions"
    };
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table} v WHERE v.workspace_id=$1 AND v.project_id=$2 AND v.issue_id=$3 AND v.deleted_at IS NULL")).bind(p.workspace_id()).bind(project).bind(issue).fetch_one(&st.db).await?;
    let results: Vec<Value> = sqlx::query_scalar(&format!("SELECT {VERSION_SUMMARY} FROM {table} v WHERE v.workspace_id=$1 AND v.project_id=$2 AND v.issue_id=$3 AND v.deleted_at IS NULL ORDER BY v.created_at DESC,v.id DESC LIMIT $4 OFFSET $5")).bind(p.workspace_id()).bind(project).bind(issue).bind(size).bind(offset).fetch_all(&st.db).await?;
    let next = offset.saturating_add(size) < total;
    Ok(Json(
        serde_json::json!({"prev_cursor":format!("{size}:{}:0",page-1),"cursor":format!("{size}:{page}:0"),"next_cursor":if next { Some(format!("{size}:{}:0",page+1)) } else { None },"prev_page_results":page>0,"next_page_results":next,"page_count":results.len(),"total_results":total,"total_pages":(total+size-1)/size,"results":results}),
    ))
}

async fn issue_versions(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(path): Path<IssuePath>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    versions(st, user, path, q, false).await
}
async fn description_versions(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(path): Path<IssuePath>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    versions(st, user, path, q, true).await
}

async fn version(
    st: AppState,
    user: CurrentUser,
    (slug, project, issue, id): CommentPath,
    description: bool,
) -> ApiResult {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    let table = if description {
        "issue_description_versions"
    } else {
        "issue_versions"
    };
    let excluded = if description {
        "'deleted_at'"
    } else {
        "'deleted_at'-'properties'-'activity_id'"
    };
    let sql = format!(
        "SELECT (to_jsonb(v)-{excluded}-'workspace_id'-'project_id'-'issue_id'-'owned_by_id'-'created_by_id'-'updated_by_id') || {VERSION_SUMMARY} FROM {table} v WHERE v.workspace_id=$1 AND v.project_id=$2 AND v.issue_id=$3 AND v.id=$4 AND v.deleted_at IS NULL"
    );
    json_row(
        sqlx::query_scalar(&sql)
            .bind(p.workspace_id())
            .bind(project)
            .bind(issue)
            .bind(id),
        &st.db,
    )
    .await
}
async fn issue_version(State(st): State<AppState>, user: CurrentUser, Path(path): Path<CommentPath>) -> ApiResult {
    version(st, user, path, false).await
}
async fn description_version(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(path): Path<CommentPath>,
) -> ApiResult {
    version(st, user, path, true).await
}

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
            "issue_view" => {
                "SELECT jsonb_build_object('name',v.name,'id',v.id,'project_id',p.id,'project__identifier',p.identifier,'workspace__slug',$5::text) FROM issue_views v JOIN projects p ON p.id=v.project_id AND p.workspace_id=v.workspace_id JOIN project_members m ON m.project_id=p.id AND m.workspace_id=p.workspace_id WHERE v.workspace_id=$1 AND m.member_id=$2 AND m.is_active AND m.deleted_at IS NULL AND v.deleted_at IS NULL AND v.archived_at IS NULL AND p.deleted_at IS NULL AND p.archived_at IS NULL AND (m.role>5 OR p.guest_view_all_features OR v.owned_by_id=$2) AND (v.owned_by_id=$2 OR v.access=1) AND v.name ILIKE '%'||$3||'%' AND ($4::uuid IS NULL OR $6 OR p.id=$4) ORDER BY v.created_at DESC LIMIT 100"
            }
            "page" => {
                "SELECT jsonb_build_object('name',pg.name,'id',pg.id,'project_ids',jsonb_agg(DISTINCT p.id),'project_identifiers',jsonb_agg(DISTINCT p.identifier),'workspace__slug',$5::text) FROM pages pg JOIN project_pages pp ON pp.page_id=pg.id AND pp.workspace_id=pg.workspace_id AND pp.deleted_at IS NULL JOIN projects p ON p.id=pp.project_id AND p.workspace_id=pp.workspace_id JOIN project_members m ON m.project_id=p.id AND m.workspace_id=p.workspace_id WHERE pg.workspace_id=$1 AND m.member_id=$2 AND m.is_active AND m.deleted_at IS NULL AND pg.deleted_at IS NULL AND pg.archived_at IS NULL AND p.deleted_at IS NULL AND p.archived_at IS NULL AND (m.role>5 OR p.guest_view_all_features OR pg.owned_by_id=$2) AND (pg.owned_by_id=$2 OR pg.access=1) AND pg.name ILIKE '%'||$3||'%' AND ($4::uuid IS NULL OR $6 OR p.id=$4) GROUP BY pg.id ORDER BY pg.created_at DESC LIMIT 100"
            }
            "intake" => {
                "SELECT jsonb_build_object('name',i.name,'id',i.id,'sequence_id',i.sequence_id,'project__identifier',p.identifier,'project_id',p.id,'workspace__slug',$5::text) FROM issues i JOIN projects p ON p.id=i.project_id AND p.workspace_id=i.workspace_id JOIN project_members m ON m.project_id=p.id AND m.workspace_id=p.workspace_id WHERE i.workspace_id=$1 AND m.member_id=$2 AND m.is_active AND m.deleted_at IS NULL AND i.deleted_at IS NULL AND p.deleted_at IS NULL AND p.archived_at IS NULL AND (m.role>5 OR p.guest_view_all_features OR i.created_by_id=$2) AND EXISTS (SELECT 1 FROM intake_issues ii WHERE ii.issue_id=i.id AND ii.workspace_id=i.workspace_id AND ii.project_id=i.project_id AND ii.deleted_at IS NULL AND ii.status IN (0,-2)) AND (i.name ILIKE '%'||$3||'%' OR (p.identifier||'-'||i.sequence_id::text) ILIKE '%'||$3||'%') AND ($4::uuid IS NULL OR $6 OR p.id=$4) ORDER BY i.created_at DESC LIMIT 100"
            }
            _ => continue,
        };
        let sql = sql
            .replace(
                "$4::uuid IS NOT NULL AND $5::text",
                "($4::uuid IS NULL OR $4::uuid IS NOT NULL) AND $5::text",
            )
            .replace("pg.owned_by_id=$2 OR pg.access=1", "pg.owned_by_id=$2 OR pg.access=0");
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
    let mut tx = st.db.begin().await?;
    let date:Option<chrono::NaiveDate>=sqlx::query_scalar("UPDATE issues i SET archived_at=CURRENT_DATE,updated_at=now(),updated_by_id=$4 WHERE i.id=$1 AND i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL AND i.archived_at IS NULL AND EXISTS(SELECT 1 FROM states s WHERE s.id=i.state_id AND s.project_id=$2 AND s.workspace_id=$3 AND s.deleted_at IS NULL AND s.group IN ('completed','cancelled')) RETURNING archived_at").bind(iid).bind(pid).bind(p.workspace_id()).bind(user.id).fetch_optional(&mut *tx).await?;
    let date = date.ok_or_else(|| ApiError::bad("Can only archive completed or cancelled issues"))?;
    let mut event = crate::activity::ActivityEvent::new(
        iid,
        pid,
        p.workspace_id(),
        user.id,
        "updated",
        "Actor has archived the issue",
    );
    event.field = Some("archived_at");
    event.new_value = Some("manual_archive");
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({"archived_at":date})))
}
async fn unarchive_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid)): Path<IssuePath>,
) -> ApiResult<axum::http::StatusCode> {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let mut tx = st.db.begin().await?;
    let r=sqlx::query("UPDATE issues SET archived_at=NULL,updated_at=now(),updated_by_id=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND archived_at IS NOT NULL").bind(iid).bind(pid).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    let mut event =
        crate::activity::ActivityEvent::new(iid, pid, p.workspace_id(), user.id, "updated", "has restored the issue");
    event.field = Some("archived_at");
    event.old_value = Some("archive");
    event.new_value = Some("restore");
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
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
    for id in &ids {
        let mut event =
            crate::activity::ActivityEvent::new(*id, pid, p.workspace_id(), user.id, "deleted", "deleted the issue");
        event.field = Some("issue");
        crate::activity::record(&mut *tx, event).await?;
    }
    tx.commit().await?;
    Ok(Json(
        serde_json::json!({"message":format!("{} issues were deleted",ids.len())}),
    ))
}
async fn bulk_archive(
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
    let mut tx = st.db.begin().await?;
    let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT i.id, s.group FROM issues i JOIN states s ON s.id=i.state_id AND s.project_id=i.project_id AND s.workspace_id=i.workspace_id AND s.deleted_at IS NULL WHERE i.id=ANY($1) AND i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL FOR UPDATE OF i")
        .bind(&ids).bind(pid).bind(p.workspace_id()).fetch_all(&mut *tx).await?;
    if rows.len() != ids.len()
        || rows
            .iter()
            .any(|(_, group)| group != "completed" && group != "cancelled")
    {
        return Err(ApiError::bad(
            "Can only archive completed or cancelled issues in this project",
        ));
    }
    let date: chrono::NaiveDate = sqlx::query_scalar("SELECT CURRENT_DATE").fetch_one(&mut *tx).await?;
    sqlx::query("UPDATE issues SET archived_at=$4,updated_at=now(),updated_by_id=$5 WHERE id=ANY($1) AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
        .bind(&ids).bind(pid).bind(p.workspace_id()).bind(date).bind(user.id).execute(&mut *tx).await?;
    for id in &ids {
        let mut event = crate::activity::ActivityEvent::new(
            *id,
            pid,
            p.workspace_id(),
            user.id,
            "updated",
            "Actor has archived the issue",
        );
        event.field = Some("archived_at");
        event.new_value = Some("manual_archive");
        crate::activity::record(&mut *tx, event).await?;
    }
    tx.commit().await?;
    Ok(Json(serde_json::json!({"archived_at":date})))
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
    let before: Vec<Value> = sqlx::query_scalar("SELECT to_jsonb(i) FROM issues i WHERE i.id=ANY($1) AND i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL FOR UPDATE")
        .bind(&ids).bind(pid).bind(p.workspace_id()).fetch_all(&mut *tx).await?;
    let count=sqlx::query(&format!("UPDATE issues i SET state_id=COALESCE($4,state_id),priority=COALESCE($5,priority),start_date=CASE WHEN $6 THEN $7 ELSE start_date END,target_date=CASE WHEN $8 THEN $9 ELSE target_date END,completed_at=CASE WHEN $4::uuid IS NULL THEN completed_at WHEN (SELECT s.group FROM states s WHERE s.id=$4 AND s.project_id=$2 AND s.workspace_id=$3) IN ('completed','cancelled') THEN now() ELSE NULL END,updated_at=now(),updated_by_id=$10 WHERE i.id=ANY($1) AND i.project_id=$2 AND i.workspace_id=$3 AND {ACTIVE} AND (CASE WHEN $6 THEN $7 ELSE start_date END IS NULL OR CASE WHEN $8 THEN $9 ELSE target_date END IS NULL OR CASE WHEN $6 THEN $7 ELSE start_date END <= CASE WHEN $8 THEN $9 ELSE target_date END)")).bind(&ids).bind(pid).bind(p.workspace_id()).bind(state).bind(priority).bind(props.contains_key("start_date")).bind(start).bind(props.contains_key("target_date")).bind(end).bind(user.id).execute(&mut *tx).await?.rows_affected();
    if count as usize != ids.len() {
        return Err(ApiError::bad("Invalid issues or date range"));
    }
    for old in before {
        let Some(id) = old
            .get("id")
            .and_then(Value::as_str)
            .and_then(|v| Uuid::parse_str(v).ok())
        else {
            continue;
        };
        let after: Value = sqlx::query_scalar(
            "SELECT to_jsonb(i) FROM issues i WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(pid)
        .bind(p.workspace_id())
        .fetch_one(&mut *tx)
        .await?;
        for (field, comment) in [
            ("priority", "updated the priority to"),
            ("start_date", "updated the start date to "),
            ("target_date", "updated the target date to"),
        ] {
            if old.get(field) != after.get(field) {
                let mut event =
                    crate::activity::ActivityEvent::new(id, pid, p.workspace_id(), user.id, "updated", comment);
                event.field = Some(field);
                event.old_value =
                    old.get(field)
                        .and_then(Value::as_str)
                        .or(if field.ends_with("_date") { Some("") } else { None });
                event.new_value = after
                    .get(field)
                    .and_then(Value::as_str)
                    .or(if field.ends_with("_date") { Some("") } else { None });
                crate::activity::record(&mut *tx, event).await?;
            }
        }
        if old.get("state_id") != after.get("state_id") {
            let old_id = old
                .get("state_id")
                .and_then(Value::as_str)
                .and_then(|v| Uuid::parse_str(v).ok());
            let new_id = after
                .get("state_id")
                .and_then(Value::as_str)
                .and_then(|v| Uuid::parse_str(v).ok());
            let names: Vec<(Uuid, String)> = sqlx::query_as("SELECT id,name FROM states WHERE id=ANY($1) AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
                .bind([old_id, new_id].into_iter().flatten().collect::<Vec<_>>()).bind(pid).bind(p.workspace_id()).fetch_all(&mut *tx).await?;
            let mut event = crate::activity::ActivityEvent::new(
                id,
                pid,
                p.workspace_id(),
                user.id,
                "updated",
                "updated the state to",
            );
            event.field = Some("state");
            event.old_identifier = old_id;
            event.new_identifier = new_id;
            event.old_value = names
                .iter()
                .find(|(id, _)| Some(*id) == old_id)
                .map(|(_, name)| name.as_str());
            event.new_value = names
                .iter()
                .find(|(id, _)| Some(*id) == new_id)
                .map(|(_, name)| name.as_str());
            crate::activity::record(&mut *tx, event).await?;
        }
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
            relation_activity(&mut tx, iid, id, pid, p.workspace_id(), user.id, kind, true).await?;
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
    let mut tx = st.db.begin().await?;
    let relation: Option<(String, Uuid)> = sqlx::query_as("UPDATE issue_relations SET deleted_at=now(),updated_at=now(),updated_by_id=$4 WHERE id=(SELECT id FROM issue_relations WHERE workspace_id=$1 AND ((issue_id=$2 AND related_issue_id=$3) OR (issue_id=$3 AND related_issue_id=$2)) AND deleted_at IS NULL LIMIT 1) RETURNING relation_type,issue_id").bind(p.workspace_id()).bind(iid).bind(other).bind(user.id).fetch_optional(&mut *tx).await?;
    let (stored, source) = relation.ok_or_else(ApiError::not_found)?;
    let kind = s(&body, "relation_type").unwrap_or(if source == iid {
        &stored
    } else {
        inverse_relation(&stored)
    });
    relation_activity(&mut tx, iid, other, pid, p.workspace_id(), user.id, kind, false).await?;
    tx.commit().await?;
    no_content()
}

fn inverse_relation(kind: &str) -> &str {
    match kind {
        "blocking" => "blocked_by",
        "blocked_by" => "blocking",
        "start_after" => "start_before",
        "start_before" => "start_after",
        "finish_after" => "finish_before",
        "finish_before" => "finish_after",
        _ => kind,
    }
}

async fn relation_activity(
    tx: &mut sqlx::PgConnection,
    issue: Uuid,
    other: Uuid,
    project: Uuid,
    workspace: Uuid,
    actor: Uuid,
    kind: &str,
    added: bool,
) -> Result<(), sqlx::Error> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT i.id,p.identifier || '-' || i.sequence_id::text FROM issues i JOIN projects p ON p.id=i.project_id AND p.workspace_id=i.workspace_id WHERE i.id=ANY($1) AND i.workspace_id=$2 AND i.deleted_at IS NULL")
        .bind([issue, other].to_vec()).bind(workspace).fetch_all(&mut *tx).await?;
    for (target, related, relation) in [(issue, other, kind), (other, issue, inverse_relation(kind))] {
        if let Some((_, name)) = rows.iter().find(|(id, _)| *id == related) {
            let comment = format!(
                "{} {} relation",
                if added { "added" } else { "deleted" },
                if added { relation } else { kind }
            );
            let mut event = crate::activity::ActivityEvent::new(
                target,
                project,
                workspace,
                actor,
                if added { "updated" } else { "deleted" },
                &comment,
            );
            event.field = Some(relation);
            event.old_value = Some(if added { "" } else { name });
            event.new_value = Some(if added { name } else { "" });
            event.old_identifier = Some(if added || target == issue { related } else { other });
            crate::activity::record(&mut *tx, event).await?;
        }
    }
    Ok(())
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
    let before: Vec<(Uuid, Option<Uuid>)> = sqlx::query_as("SELECT id,parent_id FROM issues WHERE id=ANY($1) AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL FOR UPDATE")
        .bind(&ids).bind(pid).bind(p.workspace_id()).fetch_all(&mut *tx).await?;
    let r=sqlx::query("UPDATE issues SET parent_id=$1,updated_at=now(),updated_by_id=$5 WHERE id=ANY($4) AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND archived_at IS NULL AND NOT is_draft").bind(iid).bind(pid).bind(p.workspace_id()).bind(&ids).bind(user.id).execute(&mut *tx).await?;
    if r.rows_affected() as usize != ids.len() {
        return Err(ApiError::bad("Invalid sub issue IDs"));
    }
    for (child, old_parent) in before {
        if old_parent != Some(iid) {
            parent_activity(&mut tx, child, pid, p.workspace_id(), user.id, old_parent, Some(iid)).await?;
        }
    }
    tx.commit().await?;
    sub_issues(State(st), user, Path((slug, pid, iid))).await
}

async fn parent_activity(
    tx: &mut sqlx::PgConnection,
    issue: Uuid,
    project: Uuid,
    workspace: Uuid,
    actor: Uuid,
    old: Option<Uuid>,
    new: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    let ids: Vec<Uuid> = [old, new].into_iter().flatten().collect();
    let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT i.id,p.identifier || '-' || i.sequence_id::text FROM issues i JOIN projects p ON p.id=i.project_id AND p.workspace_id=i.workspace_id WHERE i.id=ANY($1) AND i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL")
        .bind(ids).bind(project).bind(workspace).fetch_all(&mut *tx).await?;
    let mut event = crate::activity::ActivityEvent::new(
        issue,
        project,
        workspace,
        actor,
        "updated",
        "updated the parent issue to",
    );
    event.field = Some("parent");
    event.old_value = Some(
        rows.iter()
            .find(|(id, _)| Some(*id) == old)
            .map(|(_, name)| name.as_str())
            .unwrap_or(""),
    );
    event.new_value = Some(
        rows.iter()
            .find(|(id, _)| Some(*id) == new)
            .map(|(_, name)| name.as_str())
            .unwrap_or(""),
    );
    event.old_identifier = old;
    event.new_identifier = new;
    crate::activity::record(tx, event).await
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
    let mut tx = st.db.begin().await?;
    let id: Uuid=sqlx::query_scalar("INSERT INTO issue_links (workspace_id,project_id,issue_id,url,title,created_by_id,updated_by_id) SELECT $1,$2,$3,$4,$5,$6,$6 WHERE NOT EXISTS (SELECT 1 FROM issue_links WHERE workspace_id=$1 AND project_id=$2 AND issue_id=$3 AND url=$4 AND deleted_at IS NULL) RETURNING id").bind(p.workspace_id()).bind(pid).bind(iid).bind(&url).bind(title).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(||ApiError::bad("URL already exists"))?;
    let mut event =
        crate::activity::ActivityEvent::new(iid, pid, p.workspace_id(), user.id, "created", "created a link");
    event.field = Some("link");
    event.new_value = Some(&url);
    event.new_identifier = Some(id);
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
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
    let mut tx = st.db.begin().await?;
    let old: Option<String> = sqlx::query_scalar("SELECT url FROM issue_links WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL FOR UPDATE").bind(id).bind(iid).bind(pid).bind(p.workspace_id()).fetch_optional(&mut *tx).await?;
    let r=sqlx::query("UPDATE issue_links SET url=COALESCE($5,url),title=COALESCE($6,title),updated_by_id=$7,updated_at=now() WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL AND NOT EXISTS (SELECT 1 FROM issue_links other WHERE other.issue_id=$2 AND other.project_id=$3 AND other.workspace_id=$4 AND other.id<>$1 AND other.url=$5 AND other.deleted_at IS NULL)").bind(id).bind(iid).bind(pid).bind(p.workspace_id()).bind(&url).bind(title).bind(user.id).execute(&mut *tx).await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    if url.is_some() && old != url {
        let mut event =
            crate::activity::ActivityEvent::new(iid, pid, p.workspace_id(), user.id, "updated", "updated a link");
        event.field = Some("link");
        event.old_value = old.as_deref();
        event.new_value = url.as_deref();
        event.old_identifier = Some(id);
        event.new_identifier = Some(id);
        crate::activity::record(&mut *tx, event).await?;
    }
    tx.commit().await?;
    link_detail(State(st), user, Path((slug, pid, iid, id))).await
}
async fn link_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pid, iid, id)): Path<CommentPath>,
) -> ApiResult<axum::http::StatusCode> {
    let p = visible_issue(&st, &user, &slug, pid, iid).await?;
    p.require(&[ADMIN, MEMBER])?;
    let mut tx = st.db.begin().await?;
    let old: String = sqlx::query_scalar("UPDATE issue_links SET deleted_at=now(),updated_at=now(),updated_by_id=$5 WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL RETURNING url").bind(id).bind(iid).bind(pid).bind(p.workspace_id()).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(ApiError::not_found)?;
    let mut event =
        crate::activity::ActivityEvent::new(iid, pid, p.workspace_id(), user.id, "deleted", "deleted the link");
    event.field = Some("link");
    event.old_value = Some(&old);
    event.new_value = Some("");
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
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
    let mut tx = st.db.begin().await?;
    let issue = if comment {
        sqlx::query_scalar("SELECT issue_id FROM issue_comments WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
            .bind(id).bind(pid).bind(p.workspace_id()).fetch_one(&mut *tx).await?
    } else {
        id
    };
    let rid: Uuid = sqlx::query_scalar(sql)
        .bind(p.workspace_id())
        .bind(pid)
        .bind(id)
        .bind(user.id)
        .bind(code)
        .fetch_one(&mut *tx)
        .await?;
    let mut event =
        crate::activity::ActivityEvent::new(issue, pid, p.workspace_id(), user.id, "created", "added the reaction");
    event.field = Some("reaction");
    event.new_value = Some(code);
    event.new_identifier = Some(rid);
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
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
        "UPDATE comment_reactions SET deleted_at=now(),updated_at=now() WHERE comment_id=$1 AND project_id=$2 AND workspace_id=$3 AND actor_id=$4 AND reaction=$5 AND deleted_at IS NULL RETURNING id"
    } else {
        "UPDATE issue_reactions SET deleted_at=now(),updated_at=now() WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND actor_id=$4 AND reaction=$5 AND deleted_at IS NULL RETURNING id"
    };
    let mut tx = st.db.begin().await?;
    let issue = if comment {
        sqlx::query_scalar("SELECT issue_id FROM issue_comments WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
            .bind(id).bind(pid).bind(p.workspace_id()).fetch_one(&mut *tx).await?
    } else {
        id
    };
    let rid: Option<Uuid> = sqlx::query_scalar(sql)
        .bind(id)
        .bind(pid)
        .bind(p.workspace_id())
        .bind(user.id)
        .bind(&code)
        .fetch_optional(&mut *tx)
        .await?;
    let rid = rid.ok_or_else(ApiError::not_found)?;
    let mut event =
        crate::activity::ActivityEvent::new(issue, pid, p.workspace_id(), user.id, "deleted", "removed the reaction");
    event.field = Some("reaction");
    event.old_value = Some(&code);
    event.old_identifier = Some(rid);
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
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

pub(crate) async fn visible_issue(
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
pub(crate) const COMMENT_JSON: &str = "(to_jsonb(c) - 'actor_id' - 'issue_id' - 'project_id' - 'workspace_id' - 'created_by_id' - 'updated_by_id' - 'description_id' - 'parent_id') || jsonb_build_object(
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
    let mut event = crate::activity::ActivityEvent::new(
        issue,
        project,
        p.workspace_id(),
        user.id,
        "created",
        "created a comment",
    );
    event.field = Some("comment");
    event.new_value = Some(&html);
    event.new_identifier = Some(id);
    event.issue_comment_id = Some(id);
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "issue_comment", "created", serde_json::json!({"id": id}));
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
    let old: Option<String> = sqlx::query_scalar("SELECT comment_html FROM issue_comments WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL FOR UPDATE")
        .bind(comment).bind(issue).bind(project).bind(p.workspace_id()).fetch_optional(&mut *tx).await?;
    let row: Option<(Uuid,)> = sqlx::query_as("UPDATE issue_comments SET edited_at=CASE WHEN $5::text IS NOT NULL AND comment_html IS DISTINCT FROM $5 THEN now() ELSE edited_at END, comment_html=COALESCE($5,comment_html),comment_stripped=COALESCE($6,comment_stripped),comment_json=COALESCE($7,comment_json),access=COALESCE($8,access),updated_at=now(),updated_by_id=$9 WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL AND ($10 OR created_by_id=$9) RETURNING id")
        .bind(comment).bind(issue).bind(project).bind(p.workspace_id()).bind(&html).bind(html.as_deref().map(strip_tags)).bind(&json).bind(access).bind(user.id).bind(p.is_admin() || p.ws.is_admin()).fetch_optional(&mut *tx).await?;
    if row.is_none() {
        return Err(ApiError::not_found());
    }
    sqlx::query("UPDATE descriptions d SET description_html=c.comment_html,description_stripped=c.comment_stripped,description_json=c.comment_json,updated_at=now(),updated_by_id=$5 FROM issue_comments c WHERE c.id=$1 AND c.issue_id=$2 AND c.project_id=$3 AND c.workspace_id=$4 AND d.id=c.description_id AND d.project_id=$3 AND d.workspace_id=$4 AND d.deleted_at IS NULL")
        .bind(comment).bind(issue).bind(project).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
    if html.is_some() && old.as_ref() != html.as_ref() {
        let mut event = crate::activity::ActivityEvent::new(
            issue,
            project,
            p.workspace_id(),
            user.id,
            "updated",
            "updated a comment",
        );
        event.field = Some("comment");
        event.old_value = old.as_deref();
        event.new_value = html.as_deref();
        event.old_identifier = Some(comment);
        event.new_identifier = Some(comment);
        event.issue_comment_id = Some(comment);
        crate::activity::record(&mut *tx, event).await?;
    }
    tx.commit().await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "issue_comment", "updated", serde_json::json!({"id": comment}));
    comment_detail(State(st), user, Path((slug, project, issue, comment))).await
}

async fn comment_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project, issue, comment)): Path<CommentPath>,
) -> ApiResult<axum::http::StatusCode> {
    let p = visible_issue(&st, &user, &slug, project, issue).await?;
    let mut tx = st.db.begin().await?;
    let result = sqlx::query("UPDATE issue_comments SET deleted_at=now(),updated_at=now(),updated_by_id=$5 WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL AND ($6 OR created_by_id=$5)")
        .bind(comment).bind(issue).bind(project).bind(p.workspace_id()).bind(user.id).bind(p.is_admin() || p.ws.is_admin()).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    let mut event = crate::activity::ActivityEvent::new(
        issue,
        project,
        p.workspace_id(),
        user.id,
        "deleted",
        "deleted the comment",
    );
    event.field = Some("comment");
    event.issue_comment_id = Some(comment);
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "issue_comment", "deleted", serde_json::json!({"id": comment}));
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
    let sql = sql.replace("'source_data',NULL)", "'source_data',CASE WHEN $5='issue-property' THEN (SELECT jsonb_build_object('source',ii.source,'source_email',ii.source_email,'extra',ii.extra) FROM intake_issues ii WHERE ii.issue_id=a.issue_id AND ii.project_id=a.project_id AND ii.workspace_id=a.workspace_id AND ii.deleted_at IS NULL LIMIT 1) ELSE NULL END,'actor_detail',(SELECT jsonb_build_object('id',u.id,'first_name',u.first_name,'last_name',u.last_name,'avatar',u.avatar,'avatar_url',CASE WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/'||u.avatar_asset_id||'/' ELSE NULLIF(u.avatar,'') END,'is_bot',u.is_bot,'display_name',u.display_name) FROM users u WHERE u.id=a.actor_id),'issue_detail',(SELECT jsonb_build_object('id',i.id,'name',i.name,'description_json',i.description_json,'description_html',i.description_html,'priority',i.priority,'start_date',i.start_date,'target_date',i.target_date,'sequence_id',i.sequence_id,'sort_order',i.sort_order,'is_draft',i.is_draft) FROM issues i WHERE i.id=a.issue_id AND i.project_id=a.project_id AND i.workspace_id=a.workspace_id),'project_detail',(SELECT jsonb_build_object('id',p.id,'identifier',p.identifier,'name',p.name,'cover_image',p.cover_image,'cover_image_url',CASE WHEN p.cover_image_asset_id IS NOT NULL THEN '/api/assets/v2/static/'||p.cover_image_asset_id||'/' ELSE NULLIF(p.cover_image,'') END,'logo_props',p.logo_props,'description',p.description) FROM projects p WHERE p.id=a.project_id AND p.workspace_id=a.workspace_id),'workspace_detail',(SELECT jsonb_build_object('id',w.id,'name',w.name,'slug',w.slug,'logo_url',NULLIF(w.logo,'')) FROM workspaces w WHERE w.id=a.workspace_id))");
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

pub(crate) async fn update(
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
            "start_date",
            "target_date",
            "parent_id",
            "estimate_point",
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
    if body.get("estimate_point").is_some() {
        let estimate = optional_uuid(&body, "estimate_point")?;
        let previous = old
            .get("estimate_point_id")
            .and_then(Value::as_str)
            .and_then(|v| Uuid::parse_str(v).ok());
        if previous != estimate {
            let ids: Vec<Uuid> = [previous, estimate].into_iter().flatten().collect();
            let points: Vec<(Uuid, String, String)> = sqlx::query_as("SELECT ep.id,ep.value,e.type FROM estimate_points ep JOIN estimates e ON e.id=ep.estimate_id AND e.project_id=ep.project_id AND e.workspace_id=ep.workspace_id WHERE ep.id=ANY($1) AND ep.project_id=$2 AND ep.workspace_id=$3 AND ep.deleted_at IS NULL AND e.deleted_at IS NULL")
                .bind(ids).bind(project_id).bind(p.workspace_id()).fetch_all(&mut *tx).await?;
            if estimate.is_some() && !points.iter().any(|(id, _, _)| Some(*id) == estimate) {
                return Err(ApiError::bad("Invalid estimate point"));
            }
            sqlx::query("UPDATE issues SET estimate_point_id=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
                .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(estimate).execute(&mut *tx).await?;
            let kind = points
                .iter()
                .find(|(id, _, _)| Some(*id) == estimate.or(previous))
                .map(|(_, _, kind)| kind.as_str())
                .unwrap_or("points");
            let field = format!("estimate_{kind}");
            let mut event = crate::activity::ActivityEvent::new(
                issue_id,
                project_id,
                p.workspace_id(),
                user.id,
                if estimate.is_none() { "removed" } else { "updated" },
                "updated the estimate point to ",
            );
            event.field = Some(&field);
            event.old_identifier = previous;
            event.new_identifier = estimate;
            event.old_value = points
                .iter()
                .find(|(id, _, _)| Some(*id) == previous)
                .map(|(_, value, _)| value.as_str());
            event.new_value = points
                .iter()
                .find(|(id, _, _)| Some(*id) == estimate)
                .map(|(_, value, _)| value.as_str());
            crate::activity::record(&mut *tx, event).await?;
        }
    }
    if body.get("parent_id").is_some() {
        let parent = optional_uuid(&body, "parent_id")?;
        if let Some(parent) = parent {
            let valid: bool = sqlx::query_scalar("WITH RECURSIVE ancestors AS (SELECT id,parent_id FROM issues WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL UNION SELECT i.id,i.parent_id FROM issues i JOIN ancestors a ON i.id=a.parent_id WHERE i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL) SELECT EXISTS(SELECT 1 FROM ancestors) AND NOT EXISTS(SELECT 1 FROM ancestors WHERE id=$4)")
                .bind(parent).bind(project_id).bind(p.workspace_id()).bind(issue_id).fetch_one(&mut *tx).await?;
            if !valid {
                return Err(ApiError::bad("Invalid parent"));
            }
        }
        let previous = old
            .get("parent_id")
            .and_then(Value::as_str)
            .and_then(|v| Uuid::parse_str(v).ok());
        if previous != parent {
            sqlx::query("UPDATE issues SET parent_id=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
                .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(parent).execute(&mut *tx).await?;
            parent_activity(
                &mut tx,
                issue_id,
                project_id,
                p.workspace_id(),
                user.id,
                previous,
                parent,
            )
            .await?;
        }
    }
    let start = optional_date(&body, "start_date")?;
    let target = optional_date(&body, "target_date")?;
    let old_start = old
        .get("start_date")
        .and_then(Value::as_str)
        .and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
    let old_target = old
        .get("target_date")
        .and_then(Value::as_str)
        .and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
    let start = if body.get("start_date").is_some() {
        start
    } else {
        old_start
    };
    let target = if body.get("target_date").is_some() {
        target
    } else {
        old_target
    };
    if start.zip(target).is_some_and(|(a, b)| a > b) {
        return Err(ApiError::bad("Start date cannot exceed target date"));
    }
    if body.get("start_date").is_some() || body.get("target_date").is_some() {
        sqlx::query("UPDATE issues SET start_date=$4,target_date=$5 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
            .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(start).bind(target).execute(&mut *tx).await?;
    }
    if body.get("state_id").is_some() {
        let state = optional_uuid(&body, "state_id")?.ok_or_else(|| ApiError::bad("Invalid state_id"))?;
        let result = sqlx::query("UPDATE issues SET state_id=$4,completed_at=CASE WHEN (SELECT \"group\" FROM states WHERE id=$4 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL) IN ('completed','cancelled') THEN now() ELSE NULL END WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND EXISTS (SELECT 1 FROM states WHERE id=$4 AND project_id=$2 AND workspace_id=$3 AND NOT is_triage AND deleted_at IS NULL)")
            .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(state).execute(&mut *tx).await?;
        if result.rows_affected() == 0 {
            return Err(ApiError::bad("Invalid state_id"));
        }
        if old.get("state_id").and_then(Value::as_str) != Some(state.to_string().as_str()) {
            let old_id = old
                .get("state_id")
                .and_then(Value::as_str)
                .and_then(|v| Uuid::parse_str(v).ok());
            let names: Vec<(Uuid, String)> = sqlx::query_as("SELECT id,name FROM states WHERE project_id=$1 AND workspace_id=$2 AND id=ANY($3) AND deleted_at IS NULL")
                .bind(project_id).bind(p.workspace_id()).bind([old_id, Some(state)].into_iter().flatten().collect::<Vec<_>>()).fetch_all(&mut *tx).await?;
            let mut event = crate::activity::ActivityEvent::new(
                issue_id,
                project_id,
                p.workspace_id(),
                user.id,
                "updated",
                "updated the state to",
            );
            event.field = Some("state");
            event.old_identifier = old_id;
            event.new_identifier = Some(state);
            event.old_value = names
                .iter()
                .find(|(id, _)| Some(*id) == old_id)
                .map(|(_, name)| name.as_str());
            event.new_value = names.iter().find(|(id, _)| *id == state).map(|(_, name)| name.as_str());
            crate::activity::record(&mut *tx, event).await?;
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
        for (id, added) in ids
            .iter()
            .filter(|id| !before.contains(id))
            .map(|id| (*id, true))
            .chain(before.iter().filter(|id| !ids.contains(id)).map(|id| (*id, false)))
        {
            let name: Option<String> = if field == "assignee_ids" {
                sqlx::query_scalar("SELECT u.display_name FROM users u WHERE u.id=$1 AND EXISTS (SELECT 1 FROM workspace_members wm WHERE wm.member_id=u.id AND wm.workspace_id=$2 AND wm.deleted_at IS NULL)")
                    .bind(id).bind(p.workspace_id())
                    .fetch_optional(&mut *tx)
                    .await?
            } else {
                sqlx::query_scalar(
                    "SELECT name FROM labels WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
                )
                .bind(id)
                .bind(project_id)
                .bind(p.workspace_id())
                .fetch_optional(&mut *tx)
                .await?
            };
            if let Some(name) = name.as_deref() {
                let (activity_field, comment) = match (field, added) {
                    ("assignee_ids", true) => ("assignees", "added assignee "),
                    ("assignee_ids", false) => ("assignees", "removed assignee "),
                    (_, true) => ("labels", "added label "),
                    (_, false) => ("labels", "removed label "),
                };
                let mut event = crate::activity::ActivityEvent::new(
                    issue_id,
                    project_id,
                    p.workspace_id(),
                    user.id,
                    "updated",
                    comment,
                );
                event.field = Some(activity_field);
                event.old_value = Some(if added { "" } else { name });
                event.new_value = Some(if added { name } else { "" });
                if added {
                    event.new_identifier = Some(id);
                } else {
                    event.old_identifier = Some(id);
                }
                crate::activity::record(&mut *tx, event).await?;
            }
        }
    }
    let html = s(&body, "description_html");
    sqlx::query("UPDATE issues SET name=COALESCE($4,name), priority=COALESCE($5,priority), description_html=COALESCE($6,description_html), description_stripped=CASE WHEN $6::text IS NULL THEN description_stripped ELSE $7 END,updated_at=now(),updated_by_id=$8 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
        .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(s(&body,"name")).bind(s(&body,"priority")).bind(html).bind(html.map(strip_tags)).bind(user.id).execute(&mut *tx).await?;
    for field in ["name", "priority", "description_html", "start_date", "target_date"] {
        if let Some(value) = body.get(field)
            && old.get(field) != Some(value)
        {
            let (activity_field, comment) = match field {
                "name" => ("name", "updated the name to"),
                "priority" => ("priority", "updated the priority to"),
                "description_html" => ("description", "updated the description to"),
                "start_date" => ("start_date", "updated the start date to "),
                _ => ("target_date", "updated the target date to"),
            };
            let mut event = crate::activity::ActivityEvent::new(
                issue_id,
                project_id,
                p.workspace_id(),
                user.id,
                "updated",
                comment,
            );
            event.field = Some(activity_field);
            event.old_value =
                old.get(field)
                    .and_then(Value::as_str)
                    .or(if field.ends_with("_date") { Some("") } else { None });
            event.new_value = value
                .as_str()
                .or(if field.ends_with("_date") { Some("") } else { None });
            crate::activity::record(&mut *tx, event).await?;
            if field == "description_html" {
                let updated = sqlx::query("UPDATE issue_description_versions v SET description_html=i.description_html,description_stripped=i.description_stripped,description_json=i.description_json,description_binary=i.description_binary,last_saved_at=now() FROM issues i WHERE v.id=(SELECT id FROM issue_description_versions WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL ORDER BY last_saved_at DESC LIMIT 1) AND v.owned_by_id=$4 AND v.last_saved_at >= now() - interval '10 minutes' AND i.id=$1 AND i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL")
                    .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
                if updated.rows_affected() == 0 {
                    sqlx::query("INSERT INTO issue_description_versions (issue_id,project_id,workspace_id,owned_by_id,created_by_id,updated_by_id,description_html,description_stripped,description_json,description_binary) SELECT id,project_id,workspace_id,$4,$4,$4,description_html,description_stripped,description_json,description_binary FROM issues WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
                        .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
                }
            }
        }
    }
    tx.commit().await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "issue", "updated", serde_json::json!({"id": issue_id}));
    detail(State(st), user, Path((slug, project_id, issue_id))).await
}

async fn destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let mut tx = st.db.begin().await?;
    let result = sqlx::query("UPDATE issues SET deleted_at=now(), updated_at=now(), updated_by_id=$4 WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL AND ($5 OR created_by_id=$4)")
        .bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(user.id).bind(p.role == ADMIN || p.ws.is_admin()).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    let mut event = crate::activity::ActivityEvent::new(
        issue_id,
        project_id,
        p.workspace_id(),
        user.id,
        "deleted",
        "deleted the issue",
    );
    event.field = Some("issue");
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "issue", "deleted", serde_json::json!({"id": issue_id}));
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
    let mut tx = st.db.begin().await?;
    let result = create_issue(&mut tx, &user, &p, &body).await?;
    tx.commit().await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "issue", "created", result.clone());
    created(result)
}

async fn create_issue(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: &CurrentUser,
    p: &ProjectCtx,
    body: &Value,
) -> Result<Value, ApiError> {
    let project_id = p.project_id;
    let tx = &mut **tx;
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
    crate::activity::record(
        &mut *tx,
        crate::activity::ActivityEvent::new(
            id,
            project_id,
            p.workspace_id(),
            user.id,
            "created",
            "created the issue",
        ),
    )
    .await?;
    for (field, ids) in [
        ("assignees", uuid_list(body, "assignee_ids")?),
        ("labels", uuid_list(body, "label_ids")?),
    ] {
        for relation_id in ids {
            let name: Option<String> = if field == "assignees" {
                sqlx::query_scalar("SELECT u.display_name FROM users u JOIN project_members m ON m.member_id=u.id WHERE u.id=$1 AND m.project_id=$2 AND m.workspace_id=$3 AND m.is_active AND m.role>=15 AND m.deleted_at IS NULL")
                    .bind(relation_id).bind(project_id).bind(p.workspace_id()).fetch_optional(&mut *tx).await?
            } else {
                sqlx::query_scalar(
                    "SELECT name FROM labels WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
                )
                .bind(relation_id)
                .bind(project_id)
                .bind(p.workspace_id())
                .fetch_optional(&mut *tx)
                .await?
            };
            if let Some(name) = name.as_deref() {
                let mut event = crate::activity::ActivityEvent::new(
                    id,
                    project_id,
                    p.workspace_id(),
                    user.id,
                    "updated",
                    if field == "assignees" {
                        "added assignee "
                    } else {
                        "added label "
                    },
                );
                event.field = Some(field);
                event.old_value = Some("");
                event.new_value = Some(name);
                event.new_identifier = Some(relation_id);
                crate::activity::record(&mut *tx, event).await?;
            }
        }
    }
    let result: Value = sqlx::query_scalar(&format!(
        "SELECT {} || jsonb_build_object('description_html',i.description_html,'is_subscribed',false,'is_intake',false) FROM issues i WHERE i.id=$1 AND i.project_id=$2 AND i.workspace_id=$3",
        workitems::ITEM_JSON
    ))
    .bind(id)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_one(&mut *tx)
    .await?;
    Ok(result)
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
