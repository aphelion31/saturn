//! Public API v1 workspace invitations and stickies (`X-Api-Key`).
//! Ports Plane's `api/views/invite.py` (WorkspaceInvitationsViewset, workspace admin only) and
//! `api/views/sticky.py` (StickyViewSet, scoped to the caller as owner).
//! Plane's BaseViewSet answers 405 for PUT here (no `update`), so no PUT route is registered.

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api_v1::paginate;
use crate::common::*;

const INVITE_JSON: &str = "jsonb_build_object('id', i.id, 'email', i.email, 'role', i.role, 'accepted', i.accepted,
    'responded_at', i.responded_at, 'created_at', i.created_at, 'updated_at', i.updated_at)";

// Same keys as Plane's StickySerializer (`fields = "__all__"`). Copy of notifications.rs STICKY_JSON
// because that constant is private to its module.
const STICKY_JSON: &str = "jsonb_build_object(
    'id', s.id, 'created_at', s.created_at, 'updated_at', s.updated_at, 'deleted_at', s.deleted_at,
    'name', s.name, 'description', s.description, 'description_html', s.description_html,
    'description_stripped', s.description_stripped, 'description_binary', s.description_binary,
    'logo_props', s.logo_props, 'color', s.color, 'background_color', s.background_color,
    'sort_order', s.sort_order, 'created_by', s.created_by_id, 'owner', s.owner_id,
    'updated_by', s.updated_by_id, 'workspace', s.workspace_id)";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/workspaces/{slug}/invitations/",
            get(invites_list).post(invite_create),
        )
        .route(
            "/api/v1/workspaces/{slug}/invitations/{pk}/",
            get(invite_get).patch(invite_patch).delete(invite_delete),
        )
        .route(
            "/api/v1/workspaces/{slug}/stickies/",
            get(stickies_list).post(sticky_create),
        )
        .route(
            "/api/v1/workspaces/{slug}/stickies/{pk}/",
            get(sticky_get).patch(sticky_patch).delete(sticky_delete),
        )
}

// ---- validation helpers (DRF-style field errors: {"field": ["message"]}) ----

fn field_err(field: &str, msg: &str) -> ApiError {
    ApiError::bad_body(Value::Object(
        std::iter::once((field.to_string(), json!([msg]))).collect(),
    ))
}

fn check_email(v: &Value) -> ApiResult<String> {
    let Some(e) = v.as_str() else {
        return Err(field_err("email", "Not a valid string."));
    };
    if e.is_empty() {
        return Err(field_err("email", "This field may not be blank."));
    }
    if e.chars().count() > 255 {
        return Err(field_err("email", "Ensure this field has no more than 255 characters."));
    }
    let ok = e.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && !e.contains(char::is_whitespace)
    });
    if !ok {
        return Err(field_err("email", "Invalid email address"));
    }
    Ok(e.to_string())
}

fn check_role(v: &Value) -> ApiResult<i16> {
    let n = v
        .as_i64()
        .ok_or_else(|| field_err("role", "A valid integer is required."))?;
    match i16::try_from(n).ok().filter(|r| [ADMIN, MEMBER, GUEST].contains(r)) {
        Some(r) => Ok(r),
        None => Err(field_err("role", &format!("\"{n}\" is not a valid choice."))),
    }
}

/// Absent = None, explicit null on a NOT NULL column = 400, otherwise the value.
fn opt_json(b: &Value, key: &str) -> ApiResult<Option<Value>> {
    match b.get(key) {
        None => Ok(None),
        Some(Value::Null) => Err(field_err(key, "This field may not be null.")),
        Some(v) => Ok(Some(v.clone())),
    }
}

/// Absent = None, null = Some(None) (nullable column), string = Some(Some).
fn opt_str(b: &Value, key: &str, max: Option<usize>) -> ApiResult<Option<Option<String>>> {
    match b.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            if max.is_some_and(|m| s.chars().count() > m) {
                return Err(field_err(key, "Ensure this field has no more than 255 characters."));
            }
            Ok(Some(Some(s.clone())))
        }
        Some(_) => Err(field_err(key, "Not a valid string.")),
    }
}

fn opt_f64(b: &Value, key: &str) -> ApiResult<Option<f64>> {
    match b.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_f64()
            .map(Some)
            .ok_or_else(|| field_err(key, "A valid number is required.")),
    }
}

/// Sanitized html plus `description_stripped` (None when empty), as Plane's Sticky.save does.
fn opt_html(b: &Value) -> ApiResult<Option<(String, Option<String>)>> {
    match b.get("description_html") {
        None => Ok(None),
        Some(Value::String(h)) => {
            let clean = sanitize_html(h)?;
            let stripped = (!clean.is_empty()).then(|| strip_tags(&clean));
            Ok(Some((clean, stripped)))
        }
        Some(_) => Err(field_err("description_html", "Not a valid string.")),
    }
}

/// ILIKE pattern for Plane's `description_stripped__icontains`, with % and _ escaped.
fn like_pattern(q: &str) -> String {
    format!(
        "%{}%",
        q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
    )
}

// ---- workspace invitations (admin only, Plane WorkspaceOwnerPermission) ----

async fn invites_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {INVITE_JSON} FROM workspace_member_invites i
             WHERE i.workspace_id = $1 AND i.deleted_at IS NULL ORDER BY i.created_at"
        ))
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

async fn invite_get(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {INVITE_JSON} FROM workspace_member_invites i
             WHERE i.id = $1 AND i.workspace_id = $2 AND i.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

async fn invite_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let email = match b.get("email") {
        None | Some(Value::Null) => return Err(field_err("email", "This field is required.")),
        Some(v) => check_email(v)?,
    };
    let role = match b.get("role") {
        None => GUEST,
        Some(v) => check_role(v)?,
    };
    // Plane validate(): duplicate check on live invites, exact match, reported as non_field_errors.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM workspace_member_invites
         WHERE email = $1 AND workspace_id = $2 AND deleted_at IS NULL)",
    )
    .bind(&email)
    .bind(ws.workspace_id)
    .fetch_one(&st.db)
    .await?;
    if exists {
        return Err(ApiError::bad_body(
            json!({"non_field_errors": ["Email already invited"]}),
        ));
    }
    let row: Value = sqlx::query_scalar(&format!(
        "WITH i AS (
            INSERT INTO workspace_member_invites
                (id, email, token, role, accepted, workspace_id, created_by_id, created_at, updated_at)
            VALUES (gen_random_uuid(), $1, $2, $3, false, $4, $5, now(), now())
            RETURNING *)
         SELECT {INVITE_JSON} FROM i"
    ))
    .bind(&email)
    .bind(Uuid::new_v4().simple().to_string())
    .bind(role)
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    crate::workspaces::send_workspace_invitation(&st, ws.workspace_id, user.id, &email).await;
    created(row)
}

async fn invite_patch(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    if let Some(v) = b.get("email") {
        let set = match v {
            Value::Null => false,
            Value::String(e) => !e.is_empty(),
            _ => true,
        };
        if set {
            return Err(ApiError::bad_body(json!({
                "error": "Email cannot be updated after invite is created.",
                "code": "EMAIL_CANNOT_BE_UPDATED"
            })));
        }
        check_email(v)?;
    }
    let role: Option<i16> = b.get("role").map(check_role).transpose()?;
    json_row(
        sqlx::query_scalar(&format!(
            "WITH i AS (
                UPDATE workspace_member_invites
                SET role = COALESCE($3, role), updated_at = now(), updated_by_id = $4
                WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL
                RETURNING *)
             SELECT {INVITE_JSON} FROM i"
        ))
        .bind(pk)
        .bind(ws.workspace_id)
        .bind(role)
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn invite_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let (accepted, responded_at): (bool, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT accepted, responded_at FROM workspace_member_invites
         WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(ws.workspace_id)
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if accepted {
        return Err(ApiError::bad_body(
            json!({"error": "Invite already accepted", "code": "INVITE_ALREADY_ACCEPTED"}),
        ));
    }
    if responded_at.is_some() {
        return Err(ApiError::bad_body(
            json!({"error": "Invite already responded", "code": "INVITE_ALREADY_RESPONDED"}),
        ));
    }
    let r = sqlx::query(
        "UPDATE workspace_member_invites SET deleted_at = now(), updated_at = now(), updated_by_id = $3
         WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL AND NOT accepted AND responded_at IS NULL",
    )
    .bind(pk)
    .bind(ws.workspace_id)
    .bind(user.id)
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

// ---- stickies (any workspace member, owner-scoped, Plane WorkspaceUserPermission) ----

async fn stickies_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(mut q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let pattern = q.get("query").filter(|v| !v.is_empty()).map(|v| like_pattern(v));
    // Plane's StickyViewSet paginates with default_per_page=20.
    q.entry("per_page".to_string()).or_insert_with(|| "20".to_string());
    // ponytail: loads all of the caller's stickies, then paginates in memory (same as the archived lists).
    // Add SQL LIMIT/OFFSET when a user has thousands of stickies.
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {STICKY_JSON} FROM stickies s
         WHERE s.workspace_id = $1 AND s.owner_id = $2 AND s.deleted_at IS NULL
           AND ($3::text IS NULL OR s.description_stripped ILIKE $3)
         ORDER BY s.created_at DESC"
    ))
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(pattern)
    .fetch_all(&st.db)
    .await?;
    paginate(rows, &q)
}

async fn sticky_get(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {STICKY_JSON} FROM stickies s
             WHERE s.id = $1 AND s.workspace_id = $2 AND s.owner_id = $3 AND s.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(ws.workspace_id)
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn sticky_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let name = opt_str(&b, "name", None)?.flatten();
    let color = opt_str(&b, "color", Some(255))?.flatten();
    let background = opt_str(&b, "background_color", Some(255))?.flatten();
    let description = opt_json(&b, "description")?.unwrap_or_else(|| json!({}));
    let logo = opt_json(&b, "logo_props")?.unwrap_or_else(|| json!({}));
    let (html, stripped) = opt_html(&b)?.unwrap_or_else(|| ("<p></p>".to_string(), Some(String::new())));
    // Plane Sticky.save: next sort_order is last + 10000 across the workspace, else the request or 65535.
    let max_sort: Option<f64> = sqlx::query_scalar(
        "SELECT max(sort_order) FROM stickies WHERE workspace_id = $1 AND deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .fetch_one(&st.db)
    .await?;
    let sort = match max_sort {
        Some(m) => m + 10000.0,
        None => opt_f64(&b, "sort_order")?.unwrap_or(65535.0),
    };
    // ponytail: description_binary input is not accepted (the web UI never sends it).
    json_row(
        sqlx::query_scalar(&format!(
            "WITH i AS (
                INSERT INTO stickies (
                    id, workspace_id, owner_id, created_by_id, updated_by_id,
                    name, description, description_html, description_stripped,
                    logo_props, color, background_color, sort_order, created_at, updated_at)
                VALUES (gen_random_uuid(), $1, $2, $2, $2, $3, $4, $5, $6, $7, $8, $9, $10, now(), now())
                RETURNING *)
             SELECT {STICKY_JSON} FROM i s"
        ))
        .bind(ws.workspace_id)
        .bind(user.id)
        .bind(name)
        .bind(description)
        .bind(html)
        .bind(stripped)
        .bind(logo)
        .bind(color)
        .bind(background)
        .bind(sort),
        &st.db,
    )
    .await
    .map(|Json(v)| (StatusCode::CREATED, Json(v)))
}

async fn sticky_patch(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    // Each column gets a "present" flag so an explicit null clears nullable columns.
    let name = opt_str(&b, "name", None)?;
    let color = opt_str(&b, "color", Some(255))?;
    let background = opt_str(&b, "background_color", Some(255))?;
    let description = opt_json(&b, "description")?;
    let logo = opt_json(&b, "logo_props")?;
    let sort = opt_f64(&b, "sort_order")?;
    let html = opt_html(&b)?;

    let (has_name, name) = (name.is_some(), name.flatten());
    let (has_color, color) = (color.is_some(), color.flatten());
    let (has_bg, background) = (background.is_some(), background.flatten());
    let (has_desc, description) = (description.is_some(), description.unwrap_or_else(|| json!({})));
    let (has_logo, logo) = (logo.is_some(), logo.unwrap_or_else(|| json!({})));
    let (has_sort, sort) = (sort.is_some(), sort.unwrap_or(0.0));
    let (has_html, html, stripped) = match html {
        Some((h, s)) => (true, h, s),
        None => (false, String::new(), None),
    };
    json_row(
        sqlx::query_scalar(&format!(
            "WITH u AS (
                UPDATE stickies SET
                    name = CASE WHEN $4 THEN $5 ELSE name END,
                    description = CASE WHEN $6 THEN $7::jsonb ELSE description END,
                    description_html = CASE WHEN $8 THEN $9 ELSE description_html END,
                    description_stripped = CASE WHEN $8 THEN $10 ELSE description_stripped END,
                    logo_props = CASE WHEN $11 THEN $12::jsonb ELSE logo_props END,
                    color = CASE WHEN $13 THEN $14 ELSE color END,
                    background_color = CASE WHEN $15 THEN $16 ELSE background_color END,
                    sort_order = CASE WHEN $17 THEN $18 ELSE sort_order END,
                    updated_at = now(), updated_by_id = $19
                WHERE id = $1 AND workspace_id = $2 AND owner_id = $3 AND deleted_at IS NULL
                RETURNING *)
             SELECT {STICKY_JSON} FROM u s"
        ))
        .bind(pk)
        .bind(ws.workspace_id)
        .bind(user.id)
        .bind(has_name)
        .bind(name)
        .bind(has_desc)
        .bind(description)
        .bind(has_html)
        .bind(html)
        .bind(stripped)
        .bind(has_logo)
        .bind(logo)
        .bind(has_color)
        .bind(color)
        .bind(has_bg)
        .bind(background)
        .bind(has_sort)
        .bind(sort)
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn sticky_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let r = sqlx::query(
        "UPDATE stickies SET deleted_at = now(), updated_at = now(), updated_by_id = $3
         WHERE id = $1 AND workspace_id = $2 AND owner_id = $3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(ws.workspace_id)
    .bind(user.id)
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rules() {
        assert!(check_email(&json!("a@b.co")).is_ok());
        assert!(check_email(&json!("a b@c.co")).is_err());
        assert!(check_email(&json!("nodomain@x")).is_err());
        assert!(check_role(&json!(15)).is_ok());
        assert!(check_role(&json!(7)).is_err());
        assert_eq!(like_pattern("50%_x"), "%50\\%\\_x%");
    }
}
