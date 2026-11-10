//! Issue views (project views) and the workspace-wide work-item list. Port of Plane's
//! `app/urls/views.py`, `app/views/view/base.py` (`IssueViewViewSet`, `WorkspaceViewIssuesViewSet`).
//! Workspace views, favorites and intake follow in the same module.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::*;
use crate::users::patch_sets;
use crate::workitems::{self, ACTIVE, Arg, Base, Scope};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/views/",
            get(project_list).post(project_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/views/{pk}/",
            get(project_retrieve).patch(project_update).delete(project_destroy),
        )
        .route("/api/workspaces/{slug}/issues/", get(ws_issues))
}

/// IssueViewSerializer fields (`fields = "__all__"`) plus `is_favorite`.
/// `query_data` mirrors the stored `filters`, which the frontend type reads.
const VIEW_JSON: &str = "jsonb_build_object(
    'id', v.id, 'name', v.name, 'description', v.description, 'query', v.query, 'query_data', v.filters,
    'filters', v.filters, 'access', v.access, 'project', v.project_id, 'project_id', v.project_id,
    'workspace', v.workspace_id, 'workspace_id', v.workspace_id, 'display_filters', v.display_filters,
    'display_properties', v.display_properties, 'rich_filters', v.rich_filters, 'sort_order', v.sort_order,
    'logo_props', v.logo_props, 'is_locked', v.is_locked, 'owned_by', v.owned_by_id,
    'created_by', v.created_by_id, 'updated_by', v.updated_by_id, 'created_at', v.created_at,
    'updated_at', v.updated_at, 'archived_at', v.archived_at)";

/// Client-writable columns. `query`, `owned_by`, `access`, `is_locked` are read-only in Plane.
const VIEW_FIELDS: &[(&str, &str)] = &[
    ("name", "text"),
    ("description", "text"),
    ("filters", "jsonb"),
    ("display_filters", "jsonb"),
    ("display_properties", "jsonb"),
    ("rich_filters", "jsonb"),
    ("logo_props", "jsonb"),
    ("sort_order", "float8"),
];

/// Plane's `CharField` validation for `name`: required, not blank, at most 255 characters.
fn view_name(b: &Value) -> ApiResult<String> {
    match b.get("name") {
        None | Some(Value::Null) => Err(ApiError::bad_body(json!({"name": ["This field is required."]}))),
        Some(Value::String(n)) if !n.trim().is_empty() && n.chars().count() <= 255 => Ok(n.trim().to_string()),
        Some(Value::String(n)) if n.chars().count() > 255 => Err(ApiError::bad_body(
            json!({"name": ["Ensure this field has no more than 255 characters."]}),
        )),
        Some(_) => Err(ApiError::bad_body(json!({"name": ["This field may not be blank."]}))),
    }
}

/// Plane's `IssueView` query (`create` stores `query = issue_filters(filters)`; here the posted
/// filters are kept as-is, as the task asks).
async fn view_by_id(db: &PgPool, pk: Uuid, project_id: Uuid, workspace_id: Uuid, user_id: Uuid) -> ApiResult {
    let sql = format!(
        "SELECT {VIEW_JSON} || jsonb_build_object('is_favorite', EXISTS (
            SELECT 1 FROM user_favorites f WHERE f.user_id = $4 AND f.entity_identifier = v.id
              AND f.entity_type = 'view' AND f.project_id = v.project_id AND f.deleted_at IS NULL))
         FROM issue_views v
         WHERE v.id = $1 AND v.project_id = $2 AND v.workspace_id = $3 AND v.deleted_at IS NULL"
    );
    json_row(
        sqlx::query_scalar(&sql)
            .bind(pk)
            .bind(project_id)
            .bind(workspace_id)
            .bind(user_id),
        db,
    )
    .await
}

/// Plane `IssueViewViewSet.list`: views the user owns, plus public views. A project guest without
/// guest_view_all_features sees only the views they own.
async fn project_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {VIEW_JSON} || jsonb_build_object('is_favorite', EXISTS (
                SELECT 1 FROM user_favorites f WHERE f.user_id = $3 AND f.entity_identifier = v.id
                  AND f.entity_type = 'view' AND f.project_id = $1 AND f.deleted_at IS NULL))
             FROM issue_views v
             WHERE v.project_id = $1 AND v.workspace_id = $2 AND v.deleted_at IS NULL AND v.archived_at IS NULL
               AND (v.owned_by_id = $3 OR (v.access = 1 AND $4 = false))
             ORDER BY v.name"
        ))
        .bind(project_id)
        .bind(p.workspace_id())
        .bind(user.id)
        // Restricted guest: own views only.
        .bind(p.restricted_guest()),
        &st.db,
    )
    .await
}

/// Plane `IssueViewViewSet.create`: any project member (admin, member, guest) creates a view they own.
async fn project_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let name = view_name(&b)?;
    let filters = b.get("filters").cloned().unwrap_or_else(|| json!({}));
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO issue_views (id, name, description, query, filters, access, workspace_id, project_id,
             display_filters, display_properties, rich_filters, logo_props, sort_order, is_locked,
             owned_by_id, created_by_id, updated_by_id, created_at, updated_at)
         VALUES (gen_random_uuid(), $1, COALESCE($2::text, ''), '{}'::jsonb, $3::jsonb, 0, $4, $5,
             COALESCE($6::jsonb, '{}'::jsonb), COALESCE($7::jsonb, '{}'::jsonb), COALESCE($8::jsonb, '{}'::jsonb),
             COALESCE($9::jsonb, '{}'::jsonb), COALESCE($10::float8, 65535), false, $11, $11, $11, now(), now())
         RETURNING id",
    )
    .bind(&name)
    .bind(s(&b, "description"))
    .bind(filters)
    .bind(p.workspace_id())
    .bind(project_id)
    .bind(b.get("display_filters").cloned())
    .bind(b.get("display_properties").cloned())
    .bind(b.get("rich_filters").cloned())
    .bind(b.get("logo_props").cloned())
    .bind(b.get("sort_order").and_then(Value::as_f64))
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    created(view_by_id(&st.db, id, project_id, p.workspace_id(), user.id).await?.0)
}

/// Plane `IssueViewViewSet.retrieve`. A restricted guest gets 403 for a view they do not own.
async fn project_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT owned_by_id, access = 1 FROM issue_views
         WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL AND archived_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    let (owner, public) = row.ok_or_else(ApiError::not_found)?;
    if owner != user.id && !public {
        return Err(ApiError::not_found());
    }
    if p.restricted_guest() && owner != user.id {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            json!({"error": "You are not allowed to view this issue"}),
        ));
    }
    view_by_id(&st.db, pk, project_id, p.workspace_id(), user.id).await
}

/// Plane `IssueViewViewSet.partial_update`: only the creator may update; locked views refuse.
async fn project_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    let mut tx = st.db.begin().await?;
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT owned_by_id, is_locked FROM issue_views
         WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&mut *tx)
    .await?;
    let (owner, locked) = row.ok_or_else(ApiError::not_found)?;
    if owner != user.id {
        return Err(ApiError::forbidden());
    }
    if locked {
        return Err(ApiError::bad("view is locked"));
    }
    if b.get("name").is_some() {
        view_name(&b)?;
    }
    let mut sets = patch_sets(&b, VIEW_FIELDS, 2);
    sets.push("updated_by_id = $1".into());
    sets.push("updated_at = now()".into());
    sqlx::query(&format!(
        "UPDATE issue_views SET {} WHERE id = $3 AND project_id = $4 AND workspace_id = $5",
        sets.join(", ")
    ))
    .bind(user.id)
    .bind(&b)
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    view_by_id(&st.db, pk, project_id, p.workspace_id(), user.id).await
}

/// Plane `IssueViewViewSet.destroy`: project admin or the creator. Soft delete, and the view's
/// favorites go with it.
async fn project_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    let owner: Option<(Uuid,)> = sqlx::query_as(
        "SELECT owned_by_id FROM issue_views WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    let (owner,) = owner.ok_or_else(ApiError::not_found)?;
    if !(p.is_admin() || owner == user.id) {
        return Err(ApiError::forbidden());
    }
    let mut tx = st.db.begin().await?;
    sqlx::query("UPDATE issue_views SET deleted_at = now() WHERE id = $1 AND project_id = $2 AND workspace_id = $3")
        .bind(pk)
        .bind(project_id)
        .bind(p.workspace_id())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE user_favorites SET deleted_at = now()
         WHERE entity_identifier = $1 AND entity_type = 'view' AND project_id = $2 AND workspace_id = $3",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    no_content()
}

/// Plane `WorkspaceViewIssuesViewSet.list`: work items across every project the user is an active
/// member of. A project guest without guest_view_all_features sees only the items they created.
async fn ws_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let base = Base::new(
        format!(
            "i.workspace_id = $1 AND {ACTIVE} AND EXISTS (
               SELECT 1 FROM project_members pm JOIN projects gp ON gp.id = pm.project_id
               WHERE pm.project_id = i.project_id AND pm.member_id = $2 AND pm.is_active AND pm.deleted_at IS NULL
                 AND (pm.role > 5 OR gp.guest_view_all_features OR i.created_by_id = $2))"
        ),
        vec![Arg::Uuid(ws.workspace_id), Arg::Uuid(user.id)],
    );
    workitems::list(
        &st.db,
        base,
        &q,
        Scope::Workspace {
            workspace_id: ws.workspace_id,
            user_id: user.id,
        },
    )
    .await
}
