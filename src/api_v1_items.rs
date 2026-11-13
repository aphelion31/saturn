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
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/comments/",
            get(comment_list).post(comment_create),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/comments/",
            get(comment_list).post(comment_create),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/comments/{pk}/",
            get(comment_retrieve).patch(comment_update).delete(comment_delete),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/comments/{pk}/",
            get(comment_retrieve).patch(comment_update).delete(comment_delete),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/links/",
            get(link_list).post(link_create),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/links/",
            get(link_list).post(link_create),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/links/{pk}/",
            get(link_retrieve).patch(link_update).delete(link_delete),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/links/{pk}/",
            get(link_retrieve).patch(link_update).delete(link_delete),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/activities/",
            get(activity_list),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/activities/",
            get(activity_list),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/activities/{pk}/",
            get(activity_retrieve),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/activities/{pk}/",
            get(activity_retrieve),
        )
        .route("/api/v1/workspaces/{slug}/work-items/search/", get(search))
        .route("/api/v1/workspaces/{slug}/issues/search/", get(search))
        .route("/api/v1/workspaces/{slug}/work-items/{identifier}/", get(by_identifier))
        .route("/api/v1/workspaces/{slug}/issues/{identifier}/", get(by_identifier))
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/relations/",
            get(relation_list).post(relation_create),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/relations/remove/",
            axum::routing::post(relation_remove),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/relations/remove/",
            axum::routing::post(relation_remove),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/attachments/",
            get(attachment_list).post(attachment_create),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/issue-attachments/",
            get(attachment_list).post(attachment_create),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/work-items/{issue_id}/attachments/{pk}/",
            get(attachment_retrieve)
                .patch(attachment_confirm)
                .delete(attachment_delete),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/issue-attachments/{pk}/",
            get(attachment_retrieve)
                .patch(attachment_confirm)
                .delete(attachment_delete),
        )
}

// ---- relations ----

const RELATION_KEYS: [&str; 8] = [
    "blocking",
    "blocked_by",
    "duplicate",
    "relates_to",
    "start_after",
    "start_before",
    "finish_after",
    "finish_before",
];

/// Plane IssueRelationListCreateAPIEndpoint.get: relations grouped by type, `{project_id, issue_id}` entries.
async fn relation_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): SubPath,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    let rows: Vec<(String, Uuid, Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT r.relation_type, r.issue_id, r.related_issue_id, i.project_id, o.project_id
         FROM issue_relations r
         JOIN issues i ON i.id = r.issue_id AND i.workspace_id = r.workspace_id
         JOIN issues o ON o.id = r.related_issue_id AND o.workspace_id = r.workspace_id
         WHERE r.workspace_id = $1 AND (r.issue_id = $2 OR r.related_issue_id = $2) AND r.deleted_at IS NULL
           AND i.deleted_at IS NULL AND o.deleted_at IS NULL AND i.archived_at IS NULL AND o.archived_at IS NULL
           AND NOT i.is_draft AND NOT o.is_draft
           AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = i.project_id AND pm.workspace_id = $1 AND pm.member_id = $3 AND pm.is_active AND pm.deleted_at IS NULL)
           AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = o.project_id AND pm.workspace_id = $1 AND pm.member_id = $3 AND pm.is_active AND pm.deleted_at IS NULL)
           AND (NOT $4 OR (i.created_by_id = $3 AND o.created_by_id = $3))
         ORDER BY r.created_at, r.id",
    )
    .bind(p.workspace_id())
    .bind(issue_id)
    .bind(user.id)
    .bind(p.restricted_guest())
    .fetch_all(&st.db)
    .await?;
    let mut out: serde_json::Map<String, Value> = RELATION_KEYS.iter().map(|k| (k.to_string(), json!([]))).collect();
    let mut seen = std::collections::HashSet::new();
    for (rt, a, b, pa, pb) in rows {
        let from_url = a == issue_id;
        let (other, other_project) = if from_url { (b, pb) } else { (a, pa) };
        // Stored row is (issue_id, related_issue_id, type); Plane maps it per side of the URL issue.
        let entries: Vec<(&str, bool)> = match rt.as_str() {
            "blocked_by" => vec![if from_url {
                ("blocked_by", true)
            } else {
                ("blocking", true)
            }],
            "start_before" => vec![if from_url {
                ("start_before", true)
            } else {
                ("start_after", true)
            }],
            "finish_before" => vec![if from_url {
                ("finish_before", true)
            } else {
                ("finish_after", true)
            }],
            "duplicate" => vec![("duplicate", true)],
            "relates_to" => vec![("relates_to", true)],
            _ => vec![],
        };
        for (k, _) in entries {
            if matches!(k, "duplicate" | "relates_to") && !seen.insert((k, other)) {
                continue;
            }
            if let Some(Value::Array(v)) = out.get_mut(k) {
                v.push(json!({"project_id": other_project, "issue_id": other}));
            }
        }
    }
    Ok(Json(Value::Object(out)))
}

/// Plane IssueRelationListCreateAPIEndpoint.post: `{relation_type, issues: [...]}`, 201 with created rows.
async fn relation_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): SubPath,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let kind = s(&b, "relation_type")
        .ok_or_else(|| ApiError::bad_body(json!({"relation_type": ["This field is required."]})))?;
    // Plane get_actual_relation: blocking/start_after/finish_after store the reversed row.
    let (stored, reverse) = match kind {
        "blocking" => ("blocked_by", true),
        "blocked_by" => ("blocked_by", false),
        "start_after" => ("start_before", true),
        "start_before" => ("start_before", false),
        "finish_after" => ("finish_before", true),
        "finish_before" => ("finish_before", false),
        "duplicate" => ("duplicate", false),
        "relates_to" => ("relates_to", false),
        _ => return Err(ApiError::bad_body(json!({"relation_type": ["Invalid relation type."]}))),
    };
    let ids = uuid_list(&b, "issues")?;
    if ids.is_empty() {
        return Err(ApiError::bad_body(json!({"issues": ["This field is required."]})));
    }
    if ids.contains(&issue_id) {
        return Err(ApiError::bad("Cannot relate an issue to itself"));
    }
    let mut tx = st.db.begin().await?;
    let mut out = Vec::new();
    for id in ids {
        // Plane drops ids outside the workspace silently (Issue.issue_objects.filter by workspace).
        let other: Option<Uuid> = sqlx::query_scalar(&format!(
            "SELECT i.id FROM issues i WHERE i.id = $1 AND i.workspace_id = $2 AND {ACTIVE} AND (NOT $3 OR i.created_by_id = $4)"
        ))
        .bind(id)
        .bind(p.workspace_id())
        .bind(p.restricted_guest())
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(other) = other else { continue };
        let (a, b_id) = if reverse { (other, issue_id) } else { (issue_id, other) };
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO issue_relations (workspace_id, project_id, issue_id, related_issue_id, relation_type, created_by_id, updated_by_id)
             VALUES ($1, $2, $3, $4, $5, $6, $6)
             ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(p.workspace_id())
        .bind(project_id)
        .bind(a)
        .bind(b_id)
        .bind(stored)
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await?;
        if inserted.is_some() {
            crate::issues::relation_activity(
                &mut tx,
                issue_id,
                other,
                project_id,
                p.workspace_id(),
                user.id,
                kind,
                true,
            )
            .await?;
            out.push(json!({"issue_id": a, "related_issue_id": b_id, "relation_type": stored}));
        }
    }
    tx.commit().await?;
    created(Value::Array(out))
}

async fn relation_remove(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): SubPath,
    Json(b): Json<Value>,
) -> ApiResult<StatusCode> {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let related_id = b
        .get("related_issue")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| ApiError::bad("Invalid related_issue"))?;

    let mut tx = st.db.begin().await?;
    let row: Option<(String, Uuid)> = sqlx::query_as(
        "UPDATE issue_relations SET deleted_at = now(), updated_at = now(), updated_by_id = $4
         WHERE workspace_id = $1 AND deleted_at IS NULL
           AND ((issue_id = $2 AND related_issue_id = $3) OR (issue_id = $3 AND related_issue_id = $2)) RETURNING relation_type,issue_id",
    )
    .bind(p.workspace_id())
    .bind(issue_id)
    .bind(related_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;

    let (stored, source) = row.ok_or_else(ApiError::not_found)?;
    let kind = s(&b, "relation_type").unwrap_or(if source == issue_id {
        &stored
    } else {
        crate::issues::inverse_relation(&stored)
    });
    crate::issues::relation_activity(
        &mut tx,
        issue_id,
        related_id,
        project_id,
        p.workspace_id(),
        user.id,
        kind,
        false,
    )
    .await?;
    tx.commit().await?;
    no_content()
}

// ---- attachments: list and delete. Upload stays on /api/assets/v2 (see gaps in report). ----

const ATTACHMENT_V1: &str = "jsonb_build_object(
    'id', f.id, 'attributes', f.attributes, 'asset', f.asset, 'size', f.size,
    'issue_id', f.issue_id, 'project_id', f.project_id, 'workspace_id', f.workspace_id,
    'created_at', f.created_at, 'updated_at', f.updated_at,
    'created_by', f.created_by_id, 'updated_by', f.updated_by_id)";

/// Plane IssueAttachmentListCreateAPIEndpoint.get.
async fn attachment_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): SubPath,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {ATTACHMENT_V1} FROM file_assets f
             WHERE f.issue_id = $1 AND f.project_id = $2 AND f.workspace_id = $3 AND f.entity_type = 'ISSUE_ATTACHMENT'
               AND f.is_uploaded AND f.deleted_at IS NULL ORDER BY f.created_at DESC"
        ))
        .bind(issue_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

/// Plane IssueAttachmentListCreateAPIEndpoint.post: validate, insert pending row, return presign.
/// Plane's S3 presign is replaced by Saturn's own upload URL (see `assets::upload_response`).
async fn attachment_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): SubPath,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let name = s(&b, "name")
        .map(crate::assets::sanitize_filename)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ApiError::bad("Invalid request."))?;
    // Plane accepts size as a number or numeric string; non-positive values are rejected.
    let size = match b.get("size") {
        Some(Value::String(v)) => v.parse::<f64>().ok(),
        Some(v) => v.as_f64(),
        None => None,
    }
    .filter(|n| n.is_finite() && *n > 0.0)
    .ok_or_else(|| ApiError::bad("Invalid request."))?
    .min(st.cfg.file_size_limit as f64);
    let typ = s(&b, "type")
        .filter(|v| crate::api_v1_assets::MIME.contains(v))
        .ok_or_else(|| ApiError::bad("Invalid file type."))?;
    let ext_id = s(&b, "external_id").filter(|v| !v.is_empty());
    let ext_source = s(&b, "external_source").filter(|v| !v.is_empty());
    if let (Some(e), Some(source)) = (ext_id, ext_source) {
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM file_assets WHERE project_id = $1 AND workspace_id = $2 AND external_id = $3
               AND external_source = $4 AND issue_id = $5 AND entity_type = 'ISSUE_ATTACHMENT' AND deleted_at IS NULL
             LIMIT 1",
        )
        .bind(project_id)
        .bind(p.workspace_id())
        .bind(e)
        .bind(source)
        .bind(issue_id)
        .fetch_optional(&st.db)
        .await?;
        if let Some(id) = existing {
            return Err(ApiError(
                StatusCode::CONFLICT,
                json!({"error": "Issue with the same external id and external source already exists", "id": id}),
            ));
        }
    }
    let id = Uuid::new_v4();
    let key = format!("{}/{}-{}", p.workspace_id(), id.simple(), name);
    sqlx::query(
        "INSERT INTO file_assets (id, attributes, asset, size, workspace_id, project_id, issue_id, created_by_id,
           updated_by_id, external_id, external_source, entity_type, is_uploaded, is_deleted, is_archived,
           created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, $9, $10, 'ISSUE_ATTACHMENT', false, false, false, now(), now())",
    )
    .bind(id)
    .bind(json!({"name": name, "type": typ, "size": size}))
    .bind(&key)
    .bind(size)
    .bind(p.workspace_id())
    .bind(project_id)
    .bind(issue_id)
    .bind(user.id)
    .bind(ext_id)
    .bind(ext_source)
    .execute(&st.db)
    .await?;
    let attachment: Value = sqlx::query_scalar(&format!("SELECT {ATTACHMENT_V1} FROM file_assets f WHERE f.id = $1"))
        .bind(id)
        .fetch_one(&st.db)
        .await?;
    let mut v = crate::assets::upload_response(id, &st.cfg.secret_key);
    v["attachment"] = attachment;
    Ok(Json(v))
}

/// Plane IssueAttachmentDetailAPIEndpoint.get: uploaded attachment only, else 400.
async fn attachment_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, pk)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    let row: Option<(bool, Value)> = sqlx::query_as(
        "SELECT f.is_uploaded, jsonb_build_object('id', f.id, 'attributes', f.attributes, 'asset', f.asset, 'size', f.size)
         FROM file_assets f WHERE f.id = $1 AND f.issue_id = $2 AND f.project_id = $3 AND f.workspace_id = $4
           AND f.entity_type = 'ISSUE_ATTACHMENT' AND f.deleted_at IS NULL",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    let (uploaded, _) = row.as_ref().ok_or_else(ApiError::not_found)?;
    if !uploaded {
        return Err(ApiError::bad("The asset is not uploaded."));
    }
    let attachment: Value = sqlx::query_scalar(&format!("SELECT {ATTACHMENT_V1} FROM file_assets f WHERE f.id = $1"))
        .bind(pk)
        .fetch_one(&st.db)
        .await?;
    Ok(Json(attachment))
}

/// Plane IssueAttachmentDetailAPIEndpoint.patch: creator or admin marks uploaded, 204.
async fn attachment_confirm(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, pk)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let owner: Option<Option<Uuid>> = sqlx::query_scalar(
        "SELECT created_by_id FROM file_assets WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $4
           AND entity_type = 'ISSUE_ATTACHMENT' AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    if owner.ok_or_else(ApiError::not_found)? != Some(user.id) {
        p.require(&[ADMIN])?;
    }
    let mut tx = st.db.begin().await?;
    let asset: String = sqlx::query_scalar(
        "UPDATE file_assets SET is_uploaded = true, updated_at = now(), updated_by_id = $5
         WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $4 AND deleted_at IS NULL RETURNING asset",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    let mut event = crate::activity::ActivityEvent::new(
        issue_id,
        project_id,
        p.workspace_id(),
        user.id,
        "created",
        "created an attachment",
    );
    event.field = Some("attachment");
    event.new_value = Some(&asset);
    event.new_identifier = Some(pk);
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    no_content()
}

/// Plane IssueAttachmentDetailAPIEndpoint.delete: uploader or admin, soft delete, 204.
async fn attachment_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, pk)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    let owner: Option<Option<Uuid>> = sqlx::query_scalar(
        "SELECT created_by_id FROM file_assets WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $4
           AND entity_type = 'ISSUE_ATTACHMENT' AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    if owner.ok_or_else(ApiError::not_found)? != Some(user.id) {
        p.require(&[ADMIN])?;
    }
    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE file_assets SET deleted_at = now(), is_deleted = true, updated_at = now(), updated_by_id = $5
         WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $4 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    let mut event = crate::activity::ActivityEvent::new(
        issue_id,
        project_id,
        p.workspace_id(),
        user.id,
        "deleted",
        "deleted the attachment",
    );
    event.field = Some("attachment");
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    no_content()
}

type SubPath = Path<(String, Uuid, Uuid)>;
type SubItemPath = Path<(String, Uuid, Uuid, Uuid)>;

/// Work item must exist in the project and be visible to the caller (restricted guests: own items only).
async fn visible_item(
    st: &AppState,
    user: &CurrentUser,
    slug: &str,
    project_id: Uuid,
    issue_id: Uuid,
) -> Result<ProjectCtx, ApiError> {
    let p = require_project(&st.db, user, slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let ok: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM issues i WHERE i.id = $1 AND i.project_id = $2 AND i.workspace_id = $3 AND i.deleted_at IS NULL AND {ACTIVE} AND (NOT $4 OR i.created_by_id = $5))"
    ))
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(p.restricted_guest())
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    if !ok {
        return Err(ApiError::not_found());
    }
    Ok(p)
}

// ---- comments ----

/// IssueCommentSerializer fields (`comment_stripped`/`comment_json` excluded), relations as ids.
const COMMENT_V1: &str = "jsonb_build_object(
    'id', c.id, 'comment_html', c.comment_html, 'access', c.access, 'attachments', c.attachments,
    'edited_at', c.edited_at, 'actor', c.actor_id, 'issue', c.issue_id, 'project', c.project_id,
    'workspace', c.workspace_id, 'created_by', c.created_by_id, 'updated_by', c.updated_by_id,
    'created_at', c.created_at, 'updated_at', c.updated_at)";

async fn comment_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): SubPath,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {COMMENT_V1} FROM issue_comments c WHERE c.issue_id = $1 AND c.project_id = $2 AND c.workspace_id = $3 AND c.deleted_at IS NULL ORDER BY c.created_at"
        ))
        .bind(issue_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn comment_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, pk)): SubItemPath,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {COMMENT_V1} FROM issue_comments c WHERE c.id = $1 AND c.issue_id = $2 AND c.project_id = $3 AND c.workspace_id = $4 AND c.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(issue_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

/// Plane's comment serializer: `comment_html` is required on create and sanitized before storing.
async fn comment_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): SubPath,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let raw = s(&b, "comment_html")
        .ok_or_else(|| ApiError::bad_body(json!({"comment_html": ["This field is required."]})))?;
    let html = sanitize_html(raw)?;
    let access = match s(&b, "access") {
        None => "INTERNAL",
        Some(v) if v == "INTERNAL" || v == "EXTERNAL" => v,
        Some(_) => return Err(ApiError::bad_body(json!({"access": ["Invalid access"]}))),
    };
    // ponytail: Plane also writes a `descriptions` row and `comment_json`; skipped, UI reads comment_html.
    let mut tx = st.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO issue_comments (workspace_id, project_id, issue_id, actor_id, created_by_id, updated_by_id,
            comment_html, comment_stripped, comment_json, access, attachments)
         VALUES ($1,$2,$3,$4,$4,$4,$5,$6,'{}'::jsonb,$7,ARRAY[]::varchar[])
         RETURNING id",
    )
    .bind(p.workspace_id())
    .bind(project_id)
    .bind(issue_id)
    .bind(user.id)
    .bind(&html)
    .bind(strip_tags(&html))
    .bind(access)
    .fetch_one(&mut *tx)
    .await?;
    let mut event = crate::activity::ActivityEvent::new(
        issue_id,
        project_id,
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
    let activities = crate::notify::snapshots(&mut tx, issue_id, project_id, p.workspace_id()).await?;
    tx.commit().await?;
    crate::notify::after_activity(&st, issue_id, project_id, p.workspace_id(), user.id, activities);
    crate::webhook_out::emit(&st, p.workspace_id(), "issue_comment", "created", json!({"id": id}));
    let Json(v) = comment_retrieve(State(st), user, Path((slug, project_id, issue_id, id))).await?;
    created(v)
}

/// Only the author edits a comment.
async fn comment_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, pk)): SubItemPath,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let html = match b.get("comment_html") {
        None => None,
        Some(Value::String(v)) => Some(sanitize_html(v)?),
        Some(_) => return Err(ApiError::bad_body(json!({"comment_html": ["Invalid HTML."]}))),
    };
    let access = match s(&b, "access") {
        None => None,
        Some(v) if v == "INTERNAL" || v == "EXTERNAL" => Some(v),
        Some(_) => return Err(ApiError::bad_body(json!({"access": ["Invalid access"]}))),
    };
    let mut tx = st.db.begin().await?;
    let old: Option<String> = sqlx::query_scalar("SELECT comment_html FROM issue_comments WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND created_by_id=$5 AND deleted_at IS NULL FOR UPDATE")
        .bind(pk).bind(issue_id).bind(project_id).bind(p.workspace_id()).bind(user.id).fetch_optional(&mut *tx).await?;
    let r = sqlx::query(
        "UPDATE issue_comments SET comment_html = COALESCE($5, comment_html),
            comment_stripped = CASE WHEN $5::text IS NULL THEN comment_stripped ELSE $6 END,
            access = COALESCE($7, access),
            edited_at = CASE WHEN $5::text IS NOT NULL AND comment_html IS DISTINCT FROM $5 THEN now() ELSE edited_at END,
            updated_at = now(), updated_by_id = $4
         WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $8 AND deleted_at IS NULL AND created_by_id = $4",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(project_id)
    .bind(user.id)
    .bind(html.as_deref())
    .bind(html.as_deref().map(strip_tags))
    .bind(access)
    .bind(p.workspace_id())
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    if html.is_some() && old != html {
        let mut event = crate::activity::ActivityEvent::new(
            issue_id,
            project_id,
            p.workspace_id(),
            user.id,
            "updated",
            "updated a comment",
        );
        event.field = Some("comment");
        event.old_value = old.as_deref();
        event.new_value = html.as_deref();
        event.old_identifier = Some(pk);
        event.new_identifier = Some(pk);
        event.issue_comment_id = Some(pk);
        crate::activity::record(&mut *tx, event).await?;
    }
    let activities = crate::notify::snapshots(&mut tx, issue_id, project_id, p.workspace_id()).await?;
    tx.commit().await?;
    crate::notify::after_activity(&st, issue_id, project_id, p.workspace_id(), user.id, activities);
    crate::webhook_out::emit(&st, p.workspace_id(), "issue_comment", "updated", json!({"id": pk}));
    comment_retrieve(State(st), user, Path((slug, project_id, issue_id, pk))).await
}

/// Only the author deletes a comment. Soft delete.
async fn comment_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, pk)): SubItemPath,
) -> ApiResult<StatusCode> {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let mut tx = st.db.begin().await?;
    let r = sqlx::query(
        "UPDATE issue_comments SET deleted_at = now(), updated_at = now(), updated_by_id = $4
         WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $5 AND deleted_at IS NULL AND created_by_id = $4",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(project_id)
    .bind(user.id)
    .bind(p.workspace_id())
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    let mut event = crate::activity::ActivityEvent::new(
        issue_id,
        project_id,
        p.workspace_id(),
        user.id,
        "deleted",
        "deleted the comment",
    );
    event.field = Some("comment");
    event.issue_comment_id = Some(pk);
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "issue_comment", "deleted", json!({"id": pk}));
    no_content()
}

// ---- links ----

const LINK_V1: &str = "jsonb_build_object(
    'id', l.id, 'title', l.title, 'url', l.url, 'metadata', l.metadata, 'issue', l.issue_id,
    'project', l.project_id, 'workspace', l.workspace_id, 'created_by', l.created_by_id,
    'updated_by', l.updated_by_id, 'created_at', l.created_at, 'updated_at', l.updated_at)";

/// Plane accepts bare hosts and adds `http://`; rejects anything without a host or with credentials.
fn link_url(raw: &str) -> Result<String, ApiError> {
    let value = if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_owned()
    } else {
        format!("http://{raw}")
    };
    let parsed = url::Url::parse(&value).map_err(|_| ApiError::bad_body(json!({"url": ["Invalid URL."]})))?;
    if parsed.host_str().is_none() || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ApiError::bad_body(json!({"url": ["Invalid URL."]})));
    }
    Ok(value)
}

async fn link_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): SubPath,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {LINK_V1} FROM issue_links l WHERE l.issue_id = $1 AND l.project_id = $2 AND l.workspace_id = $3 AND l.deleted_at IS NULL ORDER BY l.created_at DESC"
        ))
        .bind(issue_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn link_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, pk)): SubItemPath,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {LINK_V1} FROM issue_links l WHERE l.id = $1 AND l.issue_id = $2 AND l.project_id = $3 AND l.workspace_id = $4 AND l.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(issue_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn link_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): SubPath,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let raw = s(&b, "url").ok_or_else(|| ApiError::bad_body(json!({"url": ["This field is required."]})))?;
    let url = link_url(raw)?;
    let title = s(&b, "title");
    if title.is_some_and(|t| t.chars().count() > 255) {
        return Err(ApiError::bad_body(
            json!({"title": ["Ensure this field has no more than 255 characters."]}),
        ));
    }
    let mut tx = st.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO issue_links (workspace_id, project_id, issue_id, url, title, metadata, created_by_id, updated_by_id)
         SELECT $1,$2,$3,$4,$5,'{}'::jsonb,$6,$6
         WHERE NOT EXISTS (SELECT 1 FROM issue_links WHERE issue_id = $3 AND project_id = $2 AND workspace_id = $1 AND url = $4 AND deleted_at IS NULL)
         RETURNING id",
    )
    .bind(p.workspace_id())
    .bind(project_id)
    .bind(issue_id)
    .bind(&url)
    .bind(title)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::bad_body(json!({"error": "URL already exists for this work item."})))?;
    let mut event = crate::activity::ActivityEvent::new(
        issue_id,
        project_id,
        p.workspace_id(),
        user.id,
        "created",
        "created a link",
    );
    event.field = Some("link");
    event.new_value = Some(&url);
    event.new_identifier = Some(id);
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    let Json(v) = link_retrieve(State(st), user, Path((slug, project_id, issue_id, id))).await?;
    created(v)
}

async fn link_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, pk)): SubItemPath,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let url = match s(&b, "url") {
        None => None,
        Some(raw) => Some(link_url(raw)?),
    };
    let title = s(&b, "title");
    if title.is_some_and(|t| t.chars().count() > 255) {
        return Err(ApiError::bad_body(
            json!({"title": ["Ensure this field has no more than 255 characters."]}),
        ));
    }
    let mut tx = st.db.begin().await?;
    let old: Option<String> = sqlx::query_scalar("SELECT url FROM issue_links WHERE id=$1 AND issue_id=$2 AND project_id=$3 AND workspace_id=$4 AND deleted_at IS NULL FOR UPDATE")
        .bind(pk).bind(issue_id).bind(project_id).bind(p.workspace_id()).fetch_optional(&mut *tx).await?;
    let r = sqlx::query(
        "UPDATE issue_links SET url = COALESCE($5, url), title = COALESCE($6, title), updated_by_id = $7, updated_at = now()
         WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $4 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(url.as_deref())
    .bind(title)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    if url.is_some() && old != url {
        let mut event = crate::activity::ActivityEvent::new(
            issue_id,
            project_id,
            p.workspace_id(),
            user.id,
            "updated",
            "updated a link",
        );
        event.field = Some("link");
        event.old_value = old.as_deref();
        event.new_value = url.as_deref();
        event.old_identifier = Some(pk);
        event.new_identifier = Some(pk);
        crate::activity::record(&mut *tx, event).await?;
    }
    tx.commit().await?;
    link_retrieve(State(st), user, Path((slug, project_id, issue_id, pk))).await
}

async fn link_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, pk)): SubItemPath,
) -> ApiResult<StatusCode> {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let mut tx = st.db.begin().await?;
    let old: Option<String> = sqlx::query_scalar(
        "UPDATE issue_links SET deleted_at = now(), updated_at = now(), updated_by_id = $5
         WHERE id = $1 AND issue_id = $2 AND project_id = $3 AND workspace_id = $4 AND deleted_at IS NULL RETURNING url",
    )
    .bind(pk)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    let old = old.ok_or_else(ApiError::not_found)?;
    let mut event = crate::activity::ActivityEvent::new(
        issue_id,
        project_id,
        p.workspace_id(),
        user.id,
        "deleted",
        "deleted the link",
    );
    event.field = Some("link");
    event.old_value = Some(&old);
    event.new_value = Some("");
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    no_content()
}

// ---- activities (read only) ----

const ACTIVITY_V1: &str = "jsonb_build_object(
    'id', a.id, 'verb', a.verb, 'field', a.field, 'old_value', a.old_value, 'new_value', a.new_value,
    'comment', a.comment, 'epoch', a.epoch, 'issue', a.issue_id, 'project', a.project_id,
    'workspace', a.workspace_id, 'actor', a.actor_id, 'created_by', a.created_by_id,
    'updated_by', a.updated_by_id, 'issue_comment', a.issue_comment_id,
    'created_at', a.created_at, 'updated_at', a.updated_at)";

async fn activity_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): SubPath,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {ACTIVITY_V1} FROM issue_activities a WHERE a.issue_id = $1 AND a.project_id = $2 AND a.workspace_id = $3 AND a.deleted_at IS NULL ORDER BY a.created_at"
        ))
        .bind(issue_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn activity_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id, pk)): SubItemPath,
) -> ApiResult {
    let p = visible_item(&st, &user, &slug, project_id, issue_id).await?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {ACTIVITY_V1} FROM issue_activities a WHERE a.id = $1 AND a.issue_id = $2 AND a.project_id = $3 AND a.workspace_id = $4 AND a.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(issue_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
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
    Query(mut q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    if let Some(v) = q.get("per_page") {
        let ok = v.parse::<usize>().ok().is_some_and(|n| (1..=1000).contains(&n));
        if !ok {
            return Err(ApiError::bad("Invalid per_page parameter."));
        }
    }
    let requested_group_by = q.get("group_by").cloned();
    if let Some(ref g) = requested_group_by {
        let mapped = match g.as_str() {
            "state" => "state_id",
            "labels" | "label_ids" => "labels__id",
            "assignees" | "assignee_ids" => "assignees__id",
            "cycle" | "cycles" => "cycle_id",
            "module" | "modules" | "module_ids" => "issue_module__module_id",
            other => other,
        };
        q.insert("group_by".to_string(), mapped.to_string());
    }
    if let Some(sg) = q.get("sub_group_by").cloned() {
        let mapped = match sg.as_str() {
            "state" => "state_id",
            "labels" | "label_ids" => "labels__id",
            "assignees" | "assignee_ids" => "assignees__id",
            "cycle" | "cycles" => "cycle_id",
            "module" | "modules" | "module_ids" => "issue_module__module_id",
            other => other,
        };
        q.insert("sub_group_by".to_string(), mapped.to_string());
    }
    let mut base = Base::new(
        format!("i.project_id = $1 AND i.workspace_id = $2 AND {ACTIVE}"),
        vec![Arg::Uuid(project_id), Arg::Uuid(p.workspace_id())],
    );
    scoped(&p, &mut base);
    let mut resp = workitems::list(&st.db, base, &q, Scope::Project(project_id)).await?;
    if let Some(orig) = requested_group_by {
        if let Some(obj) = resp.0.as_object_mut() {
            obj.insert("grouped_by".to_string(), json!(orig));
        }
    }
    Ok(resp)
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
    let state = opt_uuid(&b, "state_id")?.or(opt_uuid(&b, "state")?);
    let parent = opt_uuid(&b, "parent_id")?;
    let estimate = opt_uuid(&b, "estimate_point")?;

    let mut tx = st.db.begin().await?;
    // Default state when none given, same rule as issues.rs (non-triage, default first).
    let (state, state_group): (Uuid, String) = sqlx::query_as(
        "SELECT id, \"group\" FROM states WHERE project_id = $1 AND workspace_id = $2 AND deleted_at IS NULL
         AND NOT is_triage AND ($3::uuid IS NULL OR id = $3) ORDER BY \"default\" DESC, sequence LIMIT 1",
    )
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(state)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::bad("Invalid or missing project state"))?;
    let archived_at: Option<chrono::DateTime<chrono::Utc>> = match b.get("archived_at") {
        None | Some(Value::Null) => None,
        Some(v) => {
            if !["completed", "cancelled"].contains(&state_group.as_str()) {
                return Err(ApiError::bad("Only completed or cancelled work items can be archived"));
            }
            Some(parse_dt(v)?)
        }
    };
    let completed_at: Option<chrono::DateTime<chrono::Utc>> = match b.get("completed_at") {
        Some(v) if !v.is_null() => Some(parse_dt(v)?),
        Some(_) => None,
        None => {
            if ["completed", "cancelled"].contains(&state_group.as_str()) {
                Some(chrono::Utc::now())
            } else {
                None
            }
        }
    };
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
            target_date, estimate_point_id, archived_at)
         VALUES ($1,$2,$3,$4,$5,$6,$6,
            COALESCE((SELECT MAX(sort_order)+10000 FROM issues WHERE project_id=$2 AND workspace_id=$3 AND state_id=$4 AND deleted_at IS NULL), 65535),
            $7, $8, $9, $10, $11, $12, $13, $14, $15)
         RETURNING id",
    )
    .bind(name)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(state)
    .bind(sequence)
    .bind(user.id)
    .bind(completed_at)
    .bind(priority)
    .bind(&html)
    .bind(strip_tags(&html))
    .bind(parent)
    .bind(start)
    .bind(target)
    .bind(estimate)
    .bind(archived_at)
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
    let activities = crate::notify::snapshots(&mut tx, id, project_id, p.workspace_id()).await?;
    tx.commit().await?;
    crate::notify::after_activity(&st, id, project_id, p.workspace_id(), user.id, activities);
    let v = read_item(&st, &p, id).await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "issue", "created", v.clone());
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
    let old: Option<(Option<chrono::NaiveDate>, Option<chrono::NaiveDate>, Option<Uuid>)> = sqlx::query_as(
        "SELECT i.start_date, i.target_date, i.state_id FROM issues i WHERE i.id = $1 AND i.project_id = $2 AND i.workspace_id = $3 AND i.deleted_at IS NULL AND NOT i.is_draft AND (NOT $4 OR i.created_by_id = $5) FOR UPDATE"
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(p.restricted_guest())
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    let (old_start, old_target, old_state_id) = old.ok_or_else(ApiError::not_found)?;
    let before: Value = sqlx::query_scalar(
        "SELECT to_jsonb(i) FROM issues i WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_one(&mut *tx)
    .await?;

    let target_state_id = if b.get("state_id").is_some() || b.get("state").is_some() {
        opt_uuid(&b, "state_id")?.or(opt_uuid(&b, "state")?)
    } else {
        old_state_id
    };
    let state_group: Option<String> = match target_state_id {
        Some(sid) => {
            let grp: Option<String> = sqlx::query_scalar("SELECT \"group\" FROM states WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL")
                .bind(sid).bind(project_id).bind(p.workspace_id()).fetch_optional(&mut *tx).await?;
            if (b.get("state_id").is_some() || b.get("state").is_some()) && grp.is_none() {
                return Err(ApiError::bad("Invalid state_id"));
            }
            grp
        }
        None => None,
    };

    if let Some(archived_val) = b.get("archived_at") {
        if archived_val.is_null() {
            sqlx::query("UPDATE issues SET archived_at = NULL, updated_at = now(), updated_by_id = $4 WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL")
                .bind(pk).bind(project_id).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
        } else {
            let eligible = state_group
                .as_deref()
                .is_some_and(|g| ["completed", "cancelled"].contains(&g));
            if !eligible {
                return Err(ApiError::bad("Only completed or cancelled work items can be archived"));
            }
            let dt = parse_dt(archived_val)?;
            sqlx::query("UPDATE issues SET archived_at = $4, updated_at = now(), updated_by_id = $5 WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL")
                .bind(pk).bind(project_id).bind(p.workspace_id()).bind(dt).bind(user.id).execute(&mut *tx).await?;
        }
    }
    if let Some(comp_val) = b.get("completed_at") {
        if comp_val.is_null() {
            sqlx::query("UPDATE issues SET completed_at = NULL, updated_at = now(), updated_by_id = $4 WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL")
                .bind(pk).bind(project_id).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
        } else {
            let dt = parse_dt(comp_val)?;
            sqlx::query("UPDATE issues SET completed_at = $4, updated_at = now(), updated_by_id = $5 WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL")
                .bind(pk).bind(project_id).bind(p.workspace_id()).bind(dt).bind(user.id).execute(&mut *tx).await?;
        }
    } else if (b.get("state_id").is_some() || b.get("state").is_some()) && target_state_id != old_state_id {
        let is_done = state_group
            .as_deref()
            .is_some_and(|g| ["completed", "cancelled"].contains(&g));
        sqlx::query("UPDATE issues SET completed_at = CASE WHEN $4 THEN now() ELSE NULL END WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL")
            .bind(pk).bind(project_id).bind(p.workspace_id()).bind(is_done).execute(&mut *tx).await?;
    }
    if (b.get("state_id").is_some() || b.get("state").is_some()) && target_state_id != old_state_id {
        if let Some(sid) = target_state_id {
            sqlx::query("UPDATE issues SET state_id = $4, updated_at = now(), updated_by_id = $5 WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL")
                .bind(pk).bind(project_id).bind(p.workspace_id()).bind(sid).bind(user.id).execute(&mut *tx).await?;
        }
    }

    let has_start = b.get("start_date").is_some();
    let has_target = b.get("target_date").is_some();
    let start_val = if has_start {
        opt_date(&b, "start_date")?
    } else {
        old_start
    };
    let target_val = if has_target {
        opt_date(&b, "target_date")?
    } else {
        old_target
    };
    if start_val.zip(target_val).is_some_and(|(s, t)| s > t) {
        return Err(ApiError::bad("Start date cannot exceed target date"));
    }
    if has_start {
        sqlx::query("UPDATE issues SET start_date = $4, updated_at = now(), updated_by_id = $5 WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL")
            .bind(pk).bind(project_id).bind(p.workspace_id()).bind(start_val).bind(user.id).execute(&mut *tx).await?;
    }
    if has_target {
        sqlx::query("UPDATE issues SET target_date = $4, updated_at = now(), updated_by_id = $5 WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL")
            .bind(pk).bind(project_id).bind(p.workspace_id()).bind(target_val).bind(user.id).execute(&mut *tx).await?;
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
    let after: Value = sqlx::query_scalar(
        "SELECT to_jsonb(i) FROM issues i WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_one(&mut *tx)
    .await?;
    if old_state_id != target_state_id {
        let ids: Vec<Uuid> = [old_state_id, target_state_id].into_iter().flatten().collect();
        let states: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id,name FROM states WHERE id=ANY($1) AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL",
        )
        .bind(ids)
        .bind(project_id)
        .bind(p.workspace_id())
        .fetch_all(&mut *tx)
        .await?;
        let mut event = crate::activity::ActivityEvent::new(
            pk,
            project_id,
            p.workspace_id(),
            user.id,
            "updated",
            "updated the state to",
        );
        event.field = Some("state");
        event.old_identifier = old_state_id;
        event.new_identifier = target_state_id;
        event.old_value = states
            .iter()
            .find(|(id, _)| Some(*id) == old_state_id)
            .map(|(_, name)| name.as_str());
        event.new_value = states
            .iter()
            .find(|(id, _)| Some(*id) == target_state_id)
            .map(|(_, name)| name.as_str());
        crate::activity::record(&mut *tx, event).await?;
    }
    if before.get("archived_at") != after.get("archived_at") {
        let archived = after.get("archived_at").is_some_and(|v| !v.is_null());
        let mut event = crate::activity::ActivityEvent::new(
            pk,
            project_id,
            p.workspace_id(),
            user.id,
            "updated",
            if archived {
                "Actor has archived the issue"
            } else {
                "has restored the issue"
            },
        );
        event.field = Some("archived_at");
        event.old_value = if archived { None } else { Some("archive") };
        event.new_value = Some(if archived { "manual_archive" } else { "restore" });
        crate::activity::record(&mut *tx, event).await?;
    }
    for (key, field, comment) in [
        ("name", "name", "updated the name to"),
        ("priority", "priority", "updated the priority to"),
        ("description_html", "description", "updated the description to"),
        ("start_date", "start_date", "updated the start date to "),
        ("target_date", "target_date", "updated the target date to"),
    ] {
        if before.get(key) != after.get(key) {
            let mut event =
                crate::activity::ActivityEvent::new(pk, project_id, p.workspace_id(), user.id, "updated", comment);
            event.field = Some(field);
            event.old_value =
                before
                    .get(key)
                    .and_then(Value::as_str)
                    .or(if key.ends_with("_date") { Some("") } else { None });
            event.new_value =
                after
                    .get(key)
                    .and_then(Value::as_str)
                    .or(if key.ends_with("_date") { Some("") } else { None });
            crate::activity::record(&mut *tx, event).await?;
            if key == "description_html" {
                let result = sqlx::query("UPDATE issue_description_versions v SET description_html=i.description_html,description_stripped=i.description_stripped,description_json=i.description_json,description_binary=i.description_binary,last_saved_at=now() FROM issues i WHERE v.id=(SELECT id FROM issue_description_versions WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL ORDER BY last_saved_at DESC LIMIT 1) AND v.owned_by_id=$4 AND v.last_saved_at >= now() - interval '10 minutes' AND i.id=$1 AND i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL")
                    .bind(pk).bind(project_id).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
                if result.rows_affected() == 0 {
                    sqlx::query("INSERT INTO issue_description_versions (issue_id,project_id,workspace_id,owned_by_id,created_by_id,updated_by_id,description_html,description_stripped,description_json,description_binary) SELECT id,project_id,workspace_id,$4,$4,$4,description_html,description_stripped,description_json,description_binary FROM issues WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL")
                        .bind(pk).bind(project_id).bind(p.workspace_id()).bind(user.id).execute(&mut *tx).await?;
                }
            }
        }
    }
    let activities = crate::notify::snapshots(&mut tx, pk, project_id, p.workspace_id()).await?;
    tx.commit().await?;
    crate::notify::after_activity(&st, pk, project_id, p.workspace_id(), user.id, activities);
    crate::webhook_out::emit(&st, p.workspace_id(), "issue", "updated", serde_json::json!({"id": pk}));
    Ok(Json(read_item(&st, &p, pk).await?))
}

/// Soft delete. Only admins or the creator, as Plane.
async fn destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): ItemPath,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let mut tx = st.db.begin().await?;
    let r = sqlx::query(
        "UPDATE issues SET deleted_at = now(), updated_at = now(), updated_by_id = $4
         WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL AND ($5 OR created_by_id = $4)",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .bind(p.is_admin() || p.ws.is_admin())
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    let mut event = crate::activity::ActivityEvent::new(
        pk,
        project_id,
        p.workspace_id(),
        user.id,
        "deleted",
        "deleted the issue",
    );
    event.field = Some("issue");
    crate::activity::record(&mut *tx, event).await?;
    tx.commit().await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "issue", "deleted", serde_json::json!({"id": pk}));
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

fn parse_dt(v: &Value) -> Result<chrono::DateTime<chrono::Utc>, ApiError> {
    let s = v.as_str().ok_or_else(|| ApiError::bad("Invalid datetime"))?;
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).expect("valid midnight").and_utc());
    }
    Err(ApiError::bad("Invalid datetime"))
}

// ---- lookup and search ----

/// Work item by `{PROJECT_IDENTIFIER}-{SEQUENCE}` in the workspace, as Plane's `WorkspaceIssueAPIEndpoint`.
/// Restricted guests only resolve work items they created; anything else is 404.
async fn by_identifier(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, identifier)): Path<(String, String)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    // Split on the last '-' so project identifiers may not contain one, sequence is digits only.
    let (project_identifier, seq) = identifier.rsplit_once('-').ok_or_else(ApiError::not_found)?;
    let seq: i32 = seq.parse().map_err(|_| ApiError::not_found())?;
    let project_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT p.id FROM projects p JOIN project_members pm ON pm.project_id = p.id AND pm.member_id = $3
         AND pm.is_active AND pm.deleted_at IS NULL
         WHERE p.workspace_id = $1 AND p.identifier = $2 AND p.deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .bind(project_identifier)
    .bind(user.id)
    .fetch_optional(&st.db)
    .await?;
    let project_id = project_id.ok_or_else(ApiError::not_found)?;
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let mut sql = format!(
        "SELECT {ITEM_V1} FROM issues i WHERE i.workspace_id = $1 AND i.project_id = $2 AND i.sequence_id = $3 AND {ACTIVE}"
    );
    if p.restricted_guest() {
        sql.push_str(" AND i.created_by_id = $4");
    }
    let q = sqlx::query_scalar(&sql)
        .bind(ws.workspace_id)
        .bind(project_id)
        .bind(seq);
    let q = if p.restricted_guest() { q.bind(user.id) } else { q };
    json_row(q, &st.db).await
}

/// Plane's IssueSearchEndpoint: name ILIKE or `IDENTIFIER-SEQ` match, workspace or one project,
/// limit default 10, returns `{"issues": [...]}`. Guests see only their own items.
async fn search(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let Some(term) = q.get("search").map(|v| v.trim()).filter(|v| !v.is_empty()) else {
        return Ok(Json(json!({"issues": []})));
    };
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(10).clamp(1, 100);
    let project_id = q
        .get("project_id")
        .filter(|_| q.get("workspace_search").map(String::as_str) != Some("true"))
        .map(|v| Uuid::parse_str(v).map_err(|_| ApiError::bad("Invalid project_id")))
        .transpose()?;
    // Plane matches a sequence id only when the query has whole integers, else ILIKE on name and identifier.
    let seq: Option<i32> = term.parse().ok();
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT jsonb_build_object('name', i.name, 'id', i.id, 'sequence_id', i.sequence_id,
            'project_id', i.project_id, 'project__identifier', p.identifier, 'workspace__slug', w.slug)
         FROM issues i
         JOIN projects p ON p.id = i.project_id AND p.workspace_id = i.workspace_id AND p.deleted_at IS NULL AND p.archived_at IS NULL
         JOIN workspaces w ON w.id = i.workspace_id
         JOIN project_members pm ON pm.project_id = i.project_id AND pm.member_id = $2 AND pm.is_active AND pm.deleted_at IS NULL
         WHERE i.workspace_id = $1 AND {ACTIVE}
           AND ($3::uuid IS NULL OR i.project_id = $3)
           AND (pm.role > 5 OR p.guest_view_all_features OR i.created_by_id = $2)
           AND (i.name ILIKE '%' || $4 || '%' OR (p.identifier || '-' || i.sequence_id::text) ILIKE '%' || $4 || '%' OR i.sequence_id = $5)
         ORDER BY i.created_at DESC LIMIT $6"
    ))
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(project_id)
    .bind(term)
    .bind(seq)
    .bind(limit)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(json!({"issues": rows})))
}
