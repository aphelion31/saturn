//! Notifications and user workspace home endpoints.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::*;
use crate::users::USER_LITE_JSON;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/{slug}/users/notifications/",
            get(list_notifications),
        )
        .route(
            "/api/workspaces/{slug}/users/notifications/unread/",
            get(unread_count),
        )
        .route(
            "/api/workspaces/{slug}/users/notifications/mark-all-read/",
            post(mark_all_read),
        )
        .route(
            "/api/workspaces/{slug}/users/notifications/{id}/",
            get(get_notification).patch(update_notification).delete(delete_notification),
        )
        .route(
            "/api/workspaces/{slug}/users/notifications/{id}/read/",
            patch(mark_read).post(mark_read).delete(mark_unread),
        )
        .route(
            "/api/workspaces/{slug}/users/notifications/{id}/archive/",
            post(archive_notification).delete(unarchive_notification),
        )
}

fn notification_json() -> String {
    format!(
        "jsonb_build_object(
            'id', n.id,
            'created_at', n.created_at,
            'updated_at', n.updated_at,
            'created_by', n.created_by_id,
            'updated_by', n.updated_by_id,
            'deleted_at', n.deleted_at,
            'data', n.data,
            'entity_identifier', n.entity_identifier,
            'entity_name', n.entity_name,
            'title', n.title,
            'message', n.message,
            'message_html', n.message_html,
            'message_stripped', n.message_stripped,
            'sender', n.sender,
            'read_at', n.read_at,
            'snoozed_till', n.snoozed_till,
            'archived_at', n.archived_at,
            'receiver', n.receiver_id,
            'triggered_by', n.triggered_by_id,
            'project', n.project_id,
            'workspace', n.workspace_id,
            'triggered_by_details', CASE WHEN u.id IS NULL THEN NULL ELSE {USER_LITE_JSON} END,
            'is_inbox_issue', (n.entity_name = 'issue' AND EXISTS (
                SELECT 1 FROM intake_issues ii
                WHERE ii.issue_id = n.entity_identifier AND ii.workspace_id = n.workspace_id
                  AND ii.status IN (0, 2, -2) AND ii.deleted_at IS NULL
            )),
            'is_intake_issue', (n.entity_name = 'issue' AND EXISTS (
                SELECT 1 FROM intake_issues ii
                WHERE ii.issue_id = n.entity_identifier AND ii.workspace_id = n.workspace_id
                  AND ii.status IN (0, 2, -2) AND ii.deleted_at IS NULL
            )),
            'is_mentioned_notification', (n.sender ILIKE '%mentioned%')
        )"
    )
}

#[derive(Deserialize, Default)]
struct NotificationListQ {
    #[serde(rename = "type")]
    filter_type: Option<String>,
    read: Option<String>,
    archived: Option<String>,
    snoozed: Option<String>,
    mentioned: Option<String>,
    per_page: Option<i64>,
    cursor: Option<String>,
}

async fn list_notifications(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<NotificationListQ>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    // Parse cursor and pagination
    let mut limit: i64 = q.per_page.unwrap_or(20);
    let mut page: i64 = 0;
    if let Some(cursor_str) = q.cursor.as_deref() {
        let parts: Vec<&str> = cursor_str.split(':').collect();
        if parts.len() == 3 {
            if let Ok(c_limit) = parts[0].parse::<i64>() {
                if q.per_page.is_none() {
                    limit = c_limit;
                }
            }
            if let Ok(c_page) = parts[1].parse::<i64>() {
                page = c_page;
            }
        }
    }
    limit = limit.clamp(1, 1000);
    page = page.max(0);
    let offset = page * limit;

    let mut clauses = vec![
        "n.workspace_id = $1".to_string(),
        "n.receiver_id = $2".to_string(),
        "n.entity_name = 'issue'".to_string(),
        "n.deleted_at IS NULL".to_string(),
    ];

    if q.snoozed.as_deref() == Some("true") {
        clauses.push("(n.snoozed_till < now() OR n.snoozed_till IS NOT NULL)".to_string());
    } else {
        clauses.push("(n.snoozed_till IS NULL OR n.snoozed_till >= now())".to_string());
    }

    if q.archived.as_deref() == Some("true") {
        clauses.push("n.archived_at IS NOT NULL".to_string());
    } else {
        clauses.push("n.archived_at IS NULL".to_string());
    }

    match q.read.as_deref() {
        Some("true") => clauses.push("n.read_at IS NOT NULL".to_string()),
        Some("false") => clauses.push("n.read_at IS NULL".to_string()),
        _ => {}
    }

    if q.mentioned.as_deref() == Some("true") {
        clauses.push("n.sender ILIKE '%mentioned%'".to_string());
    } else {
        clauses.push("n.sender NOT ILIKE '%mentioned%'".to_string());
    }

    if let Some(ftype) = q.filter_type.as_deref() {
        let types: Vec<&str> = ftype.split(',').map(|s| s.trim()).collect();
        let mut type_clauses = Vec::new();

        if types.contains(&"subscribed") {
            type_clauses.push(
                "EXISTS (
                    SELECT 1 FROM issue_subscribers isub
                    WHERE isub.issue_id = n.entity_identifier
                      AND isub.workspace_id = $1
                      AND isub.subscriber_id = $2
                      AND isub.deleted_at IS NULL
                      AND NOT EXISTS (
                          SELECT 1 FROM issues i
                          WHERE i.id = isub.issue_id AND i.created_by_id = $2 AND i.deleted_at IS NULL
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM issue_assignees ia
                          WHERE ia.issue_id = isub.issue_id AND ia.assignee_id = $2 AND ia.deleted_at IS NULL
                      )
                )"
            );
        }

        if types.contains(&"assigned") {
            type_clauses.push(
                "EXISTS (
                    SELECT 1 FROM issue_assignees ia
                    WHERE ia.issue_id = n.entity_identifier
                      AND ia.workspace_id = $1
                      AND ia.assignee_id = $2
                      AND ia.deleted_at IS NULL
                )"
            );
        }

        if types.contains(&"created") {
            if ws.role < MEMBER {
                clauses.push("FALSE".to_string());
            } else {
                type_clauses.push(
                    "EXISTS (
                        SELECT 1 FROM issues i
                        WHERE i.id = n.entity_identifier
                          AND i.workspace_id = $1
                          AND i.created_by_id = $2
                          AND i.deleted_at IS NULL
                    )"
                );
            }
        }

        if !type_clauses.is_empty() {
            clauses.push(format!("({})", type_clauses.join(" OR ")));
        }
    }

    let where_sql = clauses.join(" AND ");

    let total_count: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM notifications n WHERE {where_sql}"
    ))
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;

    let notif_json = notification_json();
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {notif_json}
         FROM notifications n
         LEFT JOIN users u ON u.id = n.triggered_by_id
         WHERE {where_sql}
         ORDER BY n.created_at DESC
         LIMIT $3 OFFSET $4"
    ))
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&st.db)
    .await?;

    let total_pages = if total_count == 0 {
        0
    } else {
        (total_count + limit - 1) / limit
    };
    let has_next = (page + 1) * limit < total_count;
    let has_prev = page > 0;
    let next_cursor = format!("{limit}:{}:0", page + 1);
    let prev_cursor = format!("{limit}:{}:1", if page > 0 { page - 1 } else { 0 });

    Ok(Json(json!({
        "next_cursor": next_cursor,
        "prev_cursor": prev_cursor,
        "next_page_results": has_next,
        "prev_page_results": has_prev,
        "count": rows.len(),
        "total_pages": total_pages,
        "total_results": total_count,
        "total_count": total_count,
        "extra_stats": null,
        "grouped_by": null,
        "sub_grouped_by": null,
        "results": rows,
    })))
}

async fn unread_count(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let row: (i64, i64) = sqlx::query_as(
        "SELECT
            COUNT(*) FILTER (WHERE sender NOT ILIKE '%mentioned%')::int8,
            COUNT(*) FILTER (WHERE sender ILIKE '%mentioned%')::int8
         FROM notifications
         WHERE workspace_id = $1
           AND receiver_id = $2
           AND read_at IS NULL
           AND archived_at IS NULL
           AND snoozed_till IS NULL
           AND deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;

    Ok(Json(json!({
        "total_unread_notifications_count": row.0,
        "mention_unread_notifications_count": row.1,
    })))
}

async fn get_notification(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let notif_json = notification_json();
    let row: Option<Value> = sqlx::query_scalar(&format!(
        "SELECT {notif_json}
         FROM notifications n
         LEFT JOIN users u ON u.id = n.triggered_by_id
         WHERE n.id = $1 AND n.workspace_id = $2 AND n.receiver_id = $3 AND n.deleted_at IS NULL"
    ))
    .bind(id)
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_optional(&st.db)
    .await?;

    let row = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(row))
}

#[derive(Deserialize)]
struct NotificationUpdateBody {
    snoozed_till: Option<chrono::DateTime<chrono::Utc>>,
}

async fn update_notification(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(body): Json<NotificationUpdateBody>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let notif_json = notification_json();
    let row: Option<Value> = sqlx::query_scalar(&format!(
        "WITH updated AS (
            UPDATE notifications
            SET snoozed_till = $4, updated_at = now()
            WHERE id = $1 AND workspace_id = $2 AND receiver_id = $3 AND deleted_at IS NULL
            RETURNING *
         )
         SELECT {notif_json}
         FROM updated n
         LEFT JOIN users u ON u.id = n.triggered_by_id"
    ))
    .bind(id)
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(body.snoozed_till)
    .fetch_optional(&st.db)
    .await?;

    let row = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(row))
}

async fn delete_notification(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let res = sqlx::query(
        "UPDATE notifications SET deleted_at = now()
         WHERE id = $1 AND workspace_id = $2 AND receiver_id = $3 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(ws.workspace_id)
    .bind(user.id)
    .execute(&st.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn mark_read(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let notif_json = notification_json();
    let row: Option<Value> = sqlx::query_scalar(&format!(
        "WITH updated AS (
            UPDATE notifications
            SET read_at = now(), updated_at = now()
            WHERE id = $1 AND workspace_id = $2 AND receiver_id = $3 AND deleted_at IS NULL
            RETURNING *
         )
         SELECT {notif_json}
         FROM updated n
         LEFT JOIN users u ON u.id = n.triggered_by_id"
    ))
    .bind(id)
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_optional(&st.db)
    .await?;

    let row = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(row))
}

async fn mark_unread(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let notif_json = notification_json();
    let row: Option<Value> = sqlx::query_scalar(&format!(
        "WITH updated AS (
            UPDATE notifications
            SET read_at = NULL, updated_at = now()
            WHERE id = $1 AND workspace_id = $2 AND receiver_id = $3 AND deleted_at IS NULL
            RETURNING *
         )
         SELECT {notif_json}
         FROM updated n
         LEFT JOIN users u ON u.id = n.triggered_by_id"
    ))
    .bind(id)
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_optional(&st.db)
    .await?;

    let row = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(row))
}

async fn archive_notification(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let notif_json = notification_json();
    let row: Option<Value> = sqlx::query_scalar(&format!(
        "WITH updated AS (
            UPDATE notifications
            SET archived_at = now(), updated_at = now()
            WHERE id = $1 AND workspace_id = $2 AND receiver_id = $3 AND deleted_at IS NULL
            RETURNING *
         )
         SELECT {notif_json}
         FROM updated n
         LEFT JOIN users u ON u.id = n.triggered_by_id"
    ))
    .bind(id)
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_optional(&st.db)
    .await?;

    let row = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(row))
}

async fn unarchive_notification(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let notif_json = notification_json();
    let row: Option<Value> = sqlx::query_scalar(&format!(
        "WITH updated AS (
            UPDATE notifications
            SET archived_at = NULL, updated_at = now()
            WHERE id = $1 AND workspace_id = $2 AND receiver_id = $3 AND deleted_at IS NULL
            RETURNING *
         )
         SELECT {notif_json}
         FROM updated n
         LEFT JOIN users u ON u.id = n.triggered_by_id"
    ))
    .bind(id)
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_optional(&st.db)
    .await?;

    let row = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(row))
}

#[derive(Deserialize, Default)]
struct MarkAllReadBody {
    snoozed: Option<bool>,
    archived: Option<bool>,
    #[serde(rename = "type")]
    filter_type: Option<String>,
}

async fn mark_all_read(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(body): Json<MarkAllReadBody>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let mut clauses = vec![
        "workspace_id = $1".to_string(),
        "receiver_id = $2".to_string(),
        "read_at IS NULL".to_string(),
        "deleted_at IS NULL".to_string(),
    ];

    if body.snoozed == Some(true) {
        clauses.push("(snoozed_till < now() OR snoozed_till IS NOT NULL)".to_string());
    } else {
        clauses.push("(snoozed_till IS NULL OR snoozed_till >= now())".to_string());
    }

    if body.archived == Some(true) {
        clauses.push("archived_at IS NOT NULL".to_string());
    } else {
        clauses.push("archived_at IS NULL".to_string());
    }

    if let Some(ftype) = body.filter_type.as_deref() {
        if ftype == "watching" || ftype == "subscribed" {
            clauses.push(
                "EXISTS (
                    SELECT 1 FROM issue_subscribers isub
                    WHERE isub.issue_id = notifications.entity_identifier
                      AND isub.workspace_id = $1
                      AND isub.subscriber_id = $2
                      AND isub.deleted_at IS NULL
                )".to_string()
            );
        } else if ftype == "assigned" {
            clauses.push(
                "EXISTS (
                    SELECT 1 FROM issue_assignees ia
                    WHERE ia.issue_id = notifications.entity_identifier
                      AND ia.workspace_id = $1
                      AND ia.assignee_id = $2
                      AND ia.deleted_at IS NULL
                )".to_string()
            );
        } else if ftype == "created" {
            if ws.role < MEMBER {
                clauses.push("FALSE".to_string());
            } else {
                clauses.push(
                    "EXISTS (
                        SELECT 1 FROM issues i
                        WHERE i.id = notifications.entity_identifier
                          AND i.workspace_id = $1
                          AND i.created_by_id = $2
                          AND i.deleted_at IS NULL
                    )".to_string()
                );
            }
        }
    }

    let where_sql = clauses.join(" AND ");
    sqlx::query(&format!(
        "UPDATE notifications
         SET read_at = now(), updated_at = now()
         WHERE {where_sql}"
    ))
    .bind(ws.workspace_id)
    .bind(user.id)
    .execute(&st.db)
    .await?;

    Ok(Json(json!({"message": "Successful"})))
}
