//! Notifications, preferences, stickies, quick-links, recent-visits, and timezones.
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
        // Notifications
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
        // User Notification Preferences
        .route(
            "/api/users/me/notification-preferences/",
            get(get_notification_preferences).patch(patch_notification_preferences),
        )
        // Workspace Stickies
        .route(
            "/api/workspaces/{slug}/stickies/",
            get(list_stickies).post(create_sticky),
        )
        .route(
            "/api/workspaces/{slug}/stickies/{id}/",
            get(get_sticky).patch(update_sticky).delete(delete_sticky),
        )
        // Quick Links
        .route(
            "/api/workspaces/{slug}/quick-links/",
            get(list_quick_links).post(create_quick_link),
        )
        .route(
            "/api/workspaces/{slug}/quick-links/{id}/",
            get(get_quick_link).patch(update_quick_link).delete(delete_quick_link),
        )
        // Recent Visits
        .route(
            "/api/workspaces/{slug}/recent-visits/",
            get(list_recent_visits),
        )
        // Timezones
        .route("/api/timezones/", get(list_timezones))
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

/// Create an in-app notification without creating duplicates (Plane #9600).
#[allow(dead_code)]
pub async fn create_notification(
    db: &sqlx::PgPool,
    workspace_id: Uuid,
    receiver_id: Uuid,
    entity_name: &str,
    entity_identifier: Uuid,
    title: &str,
    message_html: &str,
    sender: &str,
    triggered_by_id: Option<Uuid>,
) -> Result<Option<Uuid>, ApiError> {
    let id = Uuid::new_v4();
    let row: Option<(Uuid,)> = sqlx::query_as(
        "INSERT INTO notifications (id, created_at, updated_at, workspace_id, receiver_id, entity_name,
                                    entity_identifier, title, message_html, sender, triggered_by_id)
         SELECT $1, now(), now(), $2, $3, $4, $5, $6, $7, $8, $9
         WHERE NOT EXISTS (
             SELECT 1 FROM notifications
             WHERE workspace_id = $2 AND receiver_id = $3 AND entity_name = $4
               AND entity_identifier = $5 AND sender = $8 AND deleted_at IS NULL
         )
         RETURNING id",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(receiver_id)
    .bind(entity_name)
    .bind(entity_identifier)
    .bind(title)
    .bind(message_html)
    .bind(sender)
    .bind(triggered_by_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(id,)| id))
}

// =========================================================================
// 2. User Notification Preferences (/api/users/me/notification-preferences/)
// =========================================================================

const PREF_JSON: &str = "jsonb_build_object(
    'id', p.id,
    'created_at', p.created_at,
    'updated_at', p.updated_at,
    'created_by', p.created_by_id,
    'updated_by', p.updated_by_id,
    'deleted_at', p.deleted_at,
    'property_change', p.property_change,
    'state_change', p.state_change,
    'comment', p.comment,
    'mention', p.mention,
    'issue_completed', p.issue_completed,
    'user', p.user_id,
    'workspace', p.workspace_id,
    'project', p.project_id
)";

async fn ensure_user_notification_preference(db: &sqlx::PgPool, user_id: Uuid) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO user_notification_preferences (user_id, property_change, state_change, comment, mention, issue_completed)
         SELECT $1, true, true, true, true, true
         WHERE NOT EXISTS (SELECT 1 FROM user_notification_preferences WHERE user_id = $1 AND deleted_at IS NULL)"
    )
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn get_notification_preferences(
    State(st): State<AppState>,
    user: CurrentUser,
) -> ApiResult {
    ensure_user_notification_preference(&st.db, user.id).await?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {PREF_JSON} FROM user_notification_preferences p WHERE p.user_id = $1 AND p.deleted_at IS NULL LIMIT 1"
        ))
        .bind(user.id),
        &st.db,
    )
    .await
}

#[derive(Deserialize)]
struct NotificationPreferencesPatch {
    property_change: Option<bool>,
    state_change: Option<bool>,
    comment: Option<bool>,
    mention: Option<bool>,
    issue_completed: Option<bool>,
}

async fn patch_notification_preferences(
    State(st): State<AppState>,
    user: CurrentUser,
    Json(body): Json<NotificationPreferencesPatch>,
) -> ApiResult {
    ensure_user_notification_preference(&st.db, user.id).await?;
    json_row(
        sqlx::query_scalar(&format!(
            "WITH updated AS (
                UPDATE user_notification_preferences
                SET property_change = COALESCE($2, property_change),
                    state_change = COALESCE($3, state_change),
                    comment = COALESCE($4, comment),
                    mention = COALESCE($5, mention),
                    issue_completed = COALESCE($6, issue_completed),
                    updated_at = now()
                WHERE user_id = $1 AND deleted_at IS NULL
                RETURNING *
            )
            SELECT {PREF_JSON} FROM updated p"
        ))
        .bind(user.id)
        .bind(body.property_change)
        .bind(body.state_change)
        .bind(body.comment)
        .bind(body.mention)
        .bind(body.issue_completed),
        &st.db,
    )
    .await
}

// =========================================================================
// 3. Stickies, Quick Links, Recent Visits, Timezones
// =========================================================================

const STICKY_JSON: &str = "jsonb_build_object(
    'id', s.id,
    'created_at', s.created_at,
    'updated_at', s.updated_at,
    'deleted_at', s.deleted_at,
    'name', s.name,
    'description', s.description,
    'description_html', s.description_html,
    'description_stripped', s.description_stripped,
    'description_binary', s.description_binary,
    'logo_props', s.logo_props,
    'color', s.color,
    'background_color', s.background_color,
    'sort_order', s.sort_order,
    'created_by', s.created_by_id,
    'owner', s.owner_id,
    'updated_by', s.updated_by_id,
    'workspace', s.workspace_id
)";

#[derive(Deserialize)]
struct StickyListQ {
    query: Option<String>,
    cursor: Option<String>,
    per_page: Option<i64>,
}

async fn list_stickies(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<StickyListQ>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

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

    let search = q.query.as_deref().unwrap_or("").trim().to_lowercase();
    let has_search = !search.is_empty();

    let total_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM stickies s
         WHERE s.workspace_id = $1 AND s.owner_id = $2 AND s.deleted_at IS NULL
           AND (NOT $3 OR (s.description_stripped IS NOT NULL AND LOWER(s.description_stripped) LIKE '%' || $4 || '%'))"
    )
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(has_search)
    .bind(&search)
    .fetch_one(&st.db)
    .await?;

    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {STICKY_JSON} FROM stickies s
         WHERE s.workspace_id = $1 AND s.owner_id = $2 AND s.deleted_at IS NULL
           AND (NOT $3 OR (s.description_stripped IS NOT NULL AND LOWER(s.description_stripped) LIKE '%' || $4 || '%'))
         ORDER BY s.sort_order DESC
         LIMIT $5 OFFSET $6"
    ))
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(has_search)
    .bind(&search)
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
        "results": rows,
    })))
}

#[derive(Deserialize)]
struct StickyCreate {
    name: Option<String>,
    description: Option<Value>,
    description_html: Option<String>,
    logo_props: Option<Value>,
    color: Option<String>,
    background_color: Option<String>,
}

async fn create_sticky(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(body): Json<StickyCreate>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let html = sanitize_html(body.description_html.as_deref().unwrap_or("<p></p>"))?;
    let stripped = if html.is_empty() {
        None
    } else {
        Some(strip_tags(&html))
    };
    let desc = body.description.unwrap_or_else(|| json!({}));
    let logo = body.logo_props.unwrap_or_else(|| json!({}));

    let max_sort: Option<f64> = sqlx::query_scalar(
        "SELECT max(sort_order) FROM stickies WHERE workspace_id = $1 AND deleted_at IS NULL"
    )
    .bind(ws.workspace_id)
    .fetch_one(&st.db)
    .await?;
    let sort_order = max_sort.map(|m| m + 10000.0).unwrap_or(65535.0);

    let row: Value = sqlx::query_scalar(&format!(
        "WITH inserted AS (
            INSERT INTO stickies (
                id, workspace_id, owner_id, created_by_id, updated_by_id,
                name, description, description_html, description_stripped,
                logo_props, color, background_color, sort_order, created_at, updated_at
            )
            VALUES (
                gen_random_uuid(), $1, $2, $2, $2,
                $3, $4, $5, $6,
                $7, $8, $9, $10, now(), now()
            )
            RETURNING *
        )
        SELECT {STICKY_JSON} FROM inserted s"
    ))
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(body.name)
    .bind(desc)
    .bind(html)
    .bind(stripped)
    .bind(logo)
    .bind(body.color)
    .bind(body.background_color)
    .bind(sort_order)
    .fetch_one(&st.db)
    .await?;

    created(row)
}

async fn get_sticky(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {STICKY_JSON} FROM stickies s
             WHERE s.id = $1 AND s.workspace_id = $2 AND s.owner_id = $3 AND s.deleted_at IS NULL"
        ))
        .bind(id)
        .bind(ws.workspace_id)
        .bind(user.id),
        &st.db,
    )
    .await
}

#[derive(Deserialize)]
struct StickyPatch {
    name: Option<String>,
    description: Option<Value>,
    description_html: Option<String>,
    logo_props: Option<Value>,
    color: Option<String>,
    background_color: Option<String>,
    sort_order: Option<f64>,
}

async fn update_sticky(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(body): Json<StickyPatch>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let html_res = match body.description_html.as_deref() {
        Some(h) => {
            let s = sanitize_html(h)?;
            let st = if s.is_empty() { None } else { Some(strip_tags(&s)) };
            Some((s, st))
        }
        None => None,
    };
    let html = html_res.as_ref().map(|(h, _)| h.as_str());
    let stripped = html_res.as_ref().and_then(|(_, st)| st.as_deref());

    json_row(
        sqlx::query_scalar(&format!(
            "WITH updated AS (
                UPDATE stickies
                SET name = COALESCE($4, name),
                    description = COALESCE($5, description),
                    description_html = COALESCE($6, description_html),
                    description_stripped = CASE WHEN $6 IS NOT NULL THEN $7 ELSE description_stripped END,
                    logo_props = COALESCE($8, logo_props),
                    color = COALESCE($9, color),
                    background_color = COALESCE($10, background_color),
                    sort_order = COALESCE($11, sort_order),
                    updated_at = now(),
                    updated_by_id = $3
                WHERE id = $1 AND workspace_id = $2 AND owner_id = $3 AND deleted_at IS NULL
                RETURNING *
            )
            SELECT {STICKY_JSON} FROM updated s"
        ))
        .bind(id)
        .bind(ws.workspace_id)
        .bind(user.id)
        .bind(body.name)
        .bind(body.description)
        .bind(html)
        .bind(stripped)
        .bind(body.logo_props)
        .bind(body.color)
        .bind(body.background_color)
        .bind(body.sort_order),
        &st.db,
    )
    .await
}

async fn delete_sticky(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let res = sqlx::query(
        "UPDATE stickies SET deleted_at = now()
         WHERE id = $1 AND workspace_id = $2 AND owner_id = $3 AND deleted_at IS NULL"
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

// Quick Links

const LINK_JSON: &str = "jsonb_build_object(
    'id', l.id,
    'created_at', l.created_at,
    'updated_at', l.updated_at,
    'deleted_at', l.deleted_at,
    'title', l.title,
    'url', l.url,
    'metadata', l.metadata,
    'created_by', l.created_by_id,
    'owner', l.owner_id,
    'project', l.project_id,
    'updated_by', l.updated_by_id,
    'workspace', l.workspace_id
)";

fn normalize_url(raw: &str) -> String {
    let t = raw.trim();
    if t.starts_with("http://") || t.starts_with("https://") {
        t.to_string()
    } else {
        format!("http://{t}")
    }
}

async fn list_quick_links(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {LINK_JSON} FROM workspace_user_links l
             WHERE l.workspace_id = $1 AND l.owner_id = $2 AND l.deleted_at IS NULL
             ORDER BY l.created_at DESC"
        ))
        .bind(ws.workspace_id)
        .bind(user.id),
        &st.db,
    )
    .await
}

#[derive(Deserialize)]
struct QuickLinkCreate {
    title: Option<String>,
    url: String,
    metadata: Option<Value>,
}

async fn create_quick_link(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(body): Json<QuickLinkCreate>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let url = normalize_url(&body.url);
    let metadata = body.metadata.unwrap_or_else(|| json!({}));

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM workspace_user_links
            WHERE workspace_id = $1 AND owner_id = $2 AND url = $3 AND deleted_at IS NULL
        )"
    )
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(&url)
    .fetch_one(&st.db)
    .await?;

    if exists {
        return Err(ApiError::bad("URL already exists for this workspace and owner"));
    }

    let row: Value = sqlx::query_scalar(&format!(
        "WITH inserted AS (
            INSERT INTO workspace_user_links (
                id, workspace_id, owner_id, created_by_id, updated_by_id,
                title, url, metadata, created_at, updated_at
            )
            VALUES (
                gen_random_uuid(), $1, $2, $2, $2,
                $3, $4, $5, now(), now()
            )
            RETURNING *
        )
        SELECT {LINK_JSON} FROM inserted l"
    ))
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(body.title)
    .bind(url)
    .bind(metadata)
    .fetch_one(&st.db)
    .await?;

    created(row)
}

async fn get_quick_link(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {LINK_JSON} FROM workspace_user_links l
             WHERE l.id = $1 AND l.workspace_id = $2 AND l.owner_id = $3 AND l.deleted_at IS NULL"
        ))
        .bind(id)
        .bind(ws.workspace_id)
        .bind(user.id),
        &st.db,
    )
    .await
}

#[derive(Deserialize)]
struct QuickLinkPatch {
    title: Option<String>,
    url: Option<String>,
    metadata: Option<Value>,
}

async fn update_quick_link(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(body): Json<QuickLinkPatch>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let norm_url = body.url.as_deref().map(normalize_url);

    if let Some(ref u) = norm_url {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM workspace_user_links
                WHERE workspace_id = $1 AND owner_id = $2 AND url = $3 AND id <> $4 AND deleted_at IS NULL
            )"
        )
        .bind(ws.workspace_id)
        .bind(user.id)
        .bind(u)
        .bind(id)
        .fetch_one(&st.db)
        .await?;

        if exists {
            return Err(ApiError::bad("URL already exists for this workspace and owner"));
        }
    }

    json_row(
        sqlx::query_scalar(&format!(
            "WITH updated AS (
                UPDATE workspace_user_links
                SET title = COALESCE($4, title),
                    url = COALESCE($5, url),
                    metadata = COALESCE($6, metadata),
                    updated_at = now(),
                    updated_by_id = $3
                WHERE id = $1 AND workspace_id = $2 AND owner_id = $3 AND deleted_at IS NULL
                RETURNING *
            )
            SELECT {LINK_JSON} FROM updated l"
        ))
        .bind(id)
        .bind(ws.workspace_id)
        .bind(user.id)
        .bind(body.title)
        .bind(norm_url)
        .bind(body.metadata),
        &st.db,
    )
    .await
}

async fn delete_quick_link(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let res = sqlx::query(
        "UPDATE workspace_user_links SET deleted_at = now()
         WHERE id = $1 AND workspace_id = $2 AND owner_id = $3 AND deleted_at IS NULL"
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

// Recent Visits

#[derive(Deserialize)]
struct RecentVisitsQ {
    entity_name: Option<String>,
}

async fn list_recent_visits(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<RecentVisitsQ>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    json_rows(
        sqlx::query_scalar(
            "SELECT jsonb_build_object(
                'id', rv.id,
                'entity_name', rv.entity_name,
                'entity_identifier', rv.entity_identifier,
                'visited_at', rv.visited_at,
                'entity_data', CASE
                    WHEN rv.entity_name = 'issue' THEN (
                        SELECT jsonb_build_object(
                            'id', i.id,
                            'name', i.name,
                            'state', i.state_id,
                            'priority', i.priority,
                            'assignees', COALESCE((SELECT jsonb_agg(ia.assignee_id) FROM issue_assignees ia WHERE ia.issue_id = i.id AND ia.deleted_at IS NULL), '[]'::jsonb),
                            'type', i.type_id,
                            'sequence_id', i.sequence_id,
                            'project_id', i.project_id,
                            'project_identifier', p.identifier
                        )
                        FROM issues i
                        JOIN projects p ON p.id = i.project_id AND p.deleted_at IS NULL
                        WHERE i.id = rv.entity_identifier AND i.deleted_at IS NULL
                    )
                    WHEN rv.entity_name = 'project' THEN (
                        SELECT jsonb_build_object(
                            'id', p.id,
                            'name', p.name,
                            'logo_props', p.logo_props,
                            'identifier', p.identifier,
                            'project_members', COALESCE((SELECT jsonb_agg(pm.member_id) FROM project_members pm JOIN users u ON u.id = pm.member_id WHERE pm.project_id = p.id AND NOT u.is_bot AND pm.is_active AND pm.deleted_at IS NULL), '[]'::jsonb)
                        )
                        FROM projects p
                        WHERE p.id = rv.entity_identifier AND p.deleted_at IS NULL
                    )
                    WHEN rv.entity_name = 'page' THEN (
                        SELECT jsonb_build_object(
                            'id', pg.id,
                            'name', pg.name,
                            'logo_props', pg.logo_props,
                            'project_id', pp.project_id,
                            'owned_by', pg.owned_by_id,
                            'project_identifier', p.identifier
                        )
                        FROM pages pg
                        LEFT JOIN project_pages pp ON pp.page_id = pg.id AND pp.deleted_at IS NULL
                        LEFT JOIN projects p ON p.id = pp.project_id AND p.deleted_at IS NULL
                        WHERE pg.id = rv.entity_identifier AND pg.deleted_at IS NULL
                        LIMIT 1
                    )
                    ELSE NULL
                END
            )
            FROM user_recent_visits rv
            WHERE rv.workspace_id = $1
              AND rv.user_id = $2
              AND rv.entity_name IN ('issue', 'page', 'project')
              AND ($3::text IS NULL OR rv.entity_name = $3)
              AND rv.deleted_at IS NULL
            ORDER BY rv.visited_at DESC
            LIMIT 20"
        )
        .bind(ws.workspace_id)
        .bind(user.id)
        .bind(q.entity_name),
        &st.db,
    )
    .await
}

// Timezones

async fn list_timezones() -> ApiResult {
    // Static curated IANA timezones in Plane's shape
    static TIMEZONES: &[(&str, &str, &str, &str)] = &[
        ("Pacific/Midway", "Midway Island", "UTC-11:00", "GMT-11:00"),
        ("Pacific/Honolulu", "Hawaii", "UTC-10:00", "GMT-10:00"),
        ("America/Anchorage", "Alaska", "UTC-09:00", "GMT-09:00"),
        ("America/Los_Angeles", "Pacific Time (US and Canada)", "UTC-08:00", "GMT-08:00"),
        ("America/Tijuana", "Baja California", "UTC-08:00", "GMT-08:00"),
        ("America/Denver", "Mountain Time (US and Canada)", "UTC-07:00", "GMT-07:00"),
        ("America/Phoenix", "Arizona", "UTC-07:00", "GMT-07:00"),
        ("America/Chicago", "Central Time (US and Canada)", "UTC-06:00", "GMT-06:00"),
        ("America/Mexico_City", "Guadalajara, Mexico City, Monterrey", "UTC-06:00", "GMT-06:00"),
        ("America/New_York", "Eastern Time (US and Canada)", "UTC-05:00", "GMT-05:00"),
        ("America/Bogota", "Bogota", "UTC-05:00", "GMT-05:00"),
        ("America/Lima", "Lima", "UTC-05:00", "GMT-05:00"),
        ("America/Halifax", "Atlantic Time (Canada)", "UTC-04:00", "GMT-04:00"),
        ("America/Santiago", "Santiago", "UTC-04:00", "GMT-04:00"),
        ("America/Buenos_Aires", "Buenos Aires", "UTC-03:00", "GMT-03:00"),
        ("America/Sao_Paulo", "Brasilia", "UTC-03:00", "GMT-03:00"),
        ("Atlantic/South_Georgia", "South Georgia and the South Sandwich Islands", "UTC-02:00", "GMT-02:00"),
        ("Atlantic/Azores", "Azores", "UTC-01:00", "GMT-01:00"),
        ("Atlantic/Cape_Verde", "Cape Verde Islands", "UTC-01:00", "GMT-01:00"),
        ("Europe/London", "London, Edinburgh, Dublin", "UTC+00:00", "GMT+00:00"),
        ("UTC", "UTC", "UTC+00:00", "GMT+00:00"),
        ("Europe/Paris", "Central European Time (Berlin, Rome, Paris)", "UTC+01:00", "GMT+01:00"),
        ("Africa/Lagos", "West Central Africa, Lagos", "UTC+01:00", "GMT+01:00"),
        ("Europe/Athens", "Athens, Bucharest, Istanbul", "UTC+02:00", "GMT+02:00"),
        ("Africa/Cairo", "Cairo", "UTC+02:00", "GMT+02:00"),
        ("Africa/Johannesburg", "Harare, Pretoria", "UTC+02:00", "GMT+02:00"),
        ("Europe/Moscow", "Moscow, St. Petersburg", "UTC+03:00", "GMT+03:00"),
        ("Asia/Riyadh", "Kuwait, Riyadh", "UTC+03:00", "GMT+03:00"),
        ("Asia/Tehran", "Tehran", "UTC+03:30", "GMT+03:30"),
        ("Asia/Dubai", "Abu Dhabi, Muscat", "UTC+04:00", "GMT+04:00"),
        ("Asia/Kabul", "Kabul", "UTC+04:30", "GMT+04:30"),
        ("Asia/Karachi", "Islamabad, Karachi", "UTC+05:00", "GMT+05:00"),
        ("Asia/Kolkata", "Chennai, Kolkata, Mumbai, New Delhi", "UTC+05:30", "GMT+05:30"),
        ("Asia/Kathmandu", "Kathmandu", "UTC+05:45", "GMT+05:45"),
        ("Asia/Dhaka", "Astana, Dhaka", "UTC+06:00", "GMT+06:00"),
        ("Asia/Bangkok", "Bangkok, Hanoi, Jakarta", "UTC+07:00", "GMT+07:00"),
        ("Asia/Singapore", "Kuala Lumpur, Singapore", "UTC+08:00", "GMT+08:00"),
        ("Asia/Shanghai", "Beijing, Chongqing, Hong Kong, Urumqi", "UTC+08:00", "GMT+08:00"),
        ("Asia/Tokyo", "Osaka, Sapporo, Tokyo", "UTC+09:00", "GMT+09:00"),
        ("Asia/Seoul", "Seoul", "UTC+09:00", "GMT+09:00"),
        ("Australia/Sydney", "Canberra, Melbourne, Sydney", "UTC+10:00", "GMT+10:00"),
        ("Pacific/Guam", "Guam, Port Moresby", "UTC+10:00", "GMT+10:00"),
        ("Pacific/Noumea", "New Caledonia", "UTC+11:00", "GMT+11:00"),
        ("Pacific/Auckland", "Auckland, Wellington", "UTC+12:00", "GMT+12:00"),
        ("Pacific/Fiji", "Fiji", "UTC+12:00", "GMT+12:00"),
        ("Pacific/Tongatapu", "Nuku'alofa", "UTC+13:00", "GMT+13:00"),
    ];

    let list: Vec<Value> = TIMEZONES
        .iter()
        .map(|(val, label, utc, gmt)| {
            json!({
                "value": val,
                "label": label,
                "utc_offset": utc,
                "gmt_offset": gmt,
            })
        })
        .collect();

    Ok(Json(json!({ "timezones": list })))
}
