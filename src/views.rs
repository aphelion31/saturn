//! Issue views (project and workspace), user favorite views, and the workspace-wide work-item list.
//! Port of Plane's `app/urls/views.py` and `app/views/view/base.py` (`IssueViewViewSet`,
//! `WorkspaceViewViewSet`, `WorkspaceViewIssuesViewSet`, `IssueViewFavoriteViewSet`).
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get},
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
        .route("/api/workspaces/{slug}/views/", get(ws_list).post(ws_create))
        .route(
            "/api/workspaces/{slug}/views/{pk}/",
            get(ws_retrieve).patch(ws_update).delete(ws_destroy),
        )
        .route("/api/workspaces/{slug}/issues/", get(ws_issues))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/user-favorite-views/",
            get(fav_list).post(fav_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/user-favorite-views/{view_id}/",
            delete(fav_destroy),
        )
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
        Some(Value::String(n)) if n.trim().is_empty() => {
            Err(ApiError::bad_body(json!({"name": ["This field may not be blank."]})))
        }
        Some(Value::String(n)) if n.chars().count() > 255 => Err(ApiError::bad_body(
            json!({"name": ["Ensure this field has no more than 255 characters."]}),
        )),
        Some(Value::String(n)) => Ok(n.trim().to_string()),
        Some(_) => Err(ApiError::bad_body(json!({"name": ["Not a valid string."]}))),
    }
}

/// One view with `is_favorite` for `user`. `project` None selects workspace views (project_id NULL).
async fn view_by_id(db: &PgPool, pk: Uuid, project: Option<Uuid>, workspace_id: Uuid, user_id: Uuid) -> ApiResult {
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {VIEW_JSON} || jsonb_build_object('is_favorite', EXISTS (
                SELECT 1 FROM user_favorites f WHERE f.user_id = $4 AND f.entity_identifier = v.id
                  AND f.entity_type = 'view' AND f.project_id IS NOT DISTINCT FROM v.project_id AND f.deleted_at IS NULL))
             FROM issue_views v
             WHERE v.id = $1 AND v.workspace_id = $3 AND v.project_id IS NOT DISTINCT FROM $2 AND v.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(project)
        .bind(workspace_id)
        .bind(user_id),
        db,
    )
    .await
}

/// Insert a view owned by `user_id`. Shared by project and workspace create.
async fn insert_view(
    db: &PgPool,
    b: &Value,
    workspace_id: Uuid,
    project: Option<Uuid>,
    user_id: Uuid,
) -> ApiResult<Uuid> {
    let name = view_name(b)?;
    let filters = b.get("filters").cloned().unwrap_or_else(|| json!({}));
    // Plane derives `query` from `filters` through issue_filters. Stored as an empty object here.
    let id = sqlx::query_scalar(
        "INSERT INTO issue_views (id, name, description, query, filters, access, workspace_id, project_id,
                 display_filters, display_properties, rich_filters, logo_props, sort_order, is_locked,
                 owned_by_id, created_by_id, updated_by_id, created_at, updated_at)
             VALUES (gen_random_uuid(), $1, COALESCE($2::text, ''), '{}'::jsonb, $3::jsonb, 0, $4, $5,
                 COALESCE($6::jsonb, '{}'::jsonb), COALESCE($7::jsonb, '{}'::jsonb), COALESCE($8::jsonb, '{}'::jsonb),
                 COALESCE($9::jsonb, '{}'::jsonb), COALESCE($10::float8, 65535), false, $11, $11, $11, now(), now())
             RETURNING id",
    )
    .bind(name)
    .bind(s(b, "description"))
    .bind(filters)
    .bind(workspace_id)
    .bind(project)
    .bind(b.get("display_filters").cloned())
    .bind(b.get("display_properties").cloned())
    .bind(b.get("rich_filters").cloned())
    .bind(b.get("logo_props").cloned())
    .bind(b.get("sort_order").and_then(Value::as_f64))
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(id)
}

/// Shared PATCH: the view must be unlocked, and only its creator may change it.
/// Plane answers a non-owner with 403 on project views and 400 on workspace views.
async fn update_view(
    st: &AppState,
    user: &CurrentUser,
    workspace_id: Uuid,
    project: Option<Uuid>,
    pk: Uuid,
    b: &Value,
) -> ApiResult {
    let mut tx = st.db.begin().await?;
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT owned_by_id, is_locked FROM issue_views
         WHERE id = $1 AND workspace_id = $2 AND project_id IS NOT DISTINCT FROM $3 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(pk)
    .bind(workspace_id)
    .bind(project)
    .fetch_optional(&mut *tx)
    .await?;
    let (owner, locked) = row.ok_or_else(ApiError::not_found)?;
    if locked {
        return Err(ApiError::bad("view is locked"));
    }
    if owner != user.id {
        return Err(match project {
            Some(_) => ApiError::forbidden(),
            None => ApiError::bad("Only the owner of the view can update the view"),
        });
    }
    if b.get("name").is_some() {
        view_name(b)?;
    }
    let mut sets = patch_sets(b, VIEW_FIELDS, 2);
    sets.push("updated_by_id = $1".into());
    sets.push("updated_at = now()".into());
    sqlx::query(&format!(
        "UPDATE issue_views SET {} WHERE id = $3 AND workspace_id = $4 AND project_id IS NOT DISTINCT FROM $5",
        sets.join(", ")
    ))
    .bind(user.id)
    .bind(b)
    .bind(pk)
    .bind(workspace_id)
    .bind(project)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    view_by_id(&st.db, pk, project, workspace_id, user.id).await
}

/// Soft delete a view and its favorites. Returns Err when the view is missing or the caller may not delete.
async fn delete_view(
    db: &PgPool,
    user_id: Uuid,
    may_delete_any: bool,
    workspace_id: Uuid,
    project: Option<Uuid>,
    pk: Uuid,
) -> ApiResult<StatusCode> {
    let owner: Option<(Uuid,)> = sqlx::query_as(
        "SELECT owned_by_id FROM issue_views WHERE id = $1 AND workspace_id = $2
         AND project_id IS NOT DISTINCT FROM $3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(workspace_id)
    .bind(project)
    .fetch_optional(db)
    .await?;
    let (owner,) = owner.ok_or_else(ApiError::not_found)?;
    if !(may_delete_any || owner == user_id) {
        return Err(match project {
            Some(_) => ApiError::forbidden(),
            None => ApiError::bad("Only admin or owner can delete the view"),
        });
    }
    let mut tx = db.begin().await?;
    sqlx::query("UPDATE issue_views SET deleted_at = now() WHERE id = $1 AND workspace_id = $2 AND project_id IS NOT DISTINCT FROM $3")
        .bind(pk)
        .bind(workspace_id)
        .bind(project)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE user_favorites SET deleted_at = now()
         WHERE entity_identifier = $1 AND entity_type = 'view' AND workspace_id = $2
           AND project_id IS NOT DISTINCT FROM $3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(workspace_id)
    .bind(project)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    no_content()
}

// ---------------------------------------------------------------------------------------------------
// Project views
// ---------------------------------------------------------------------------------------------------

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
        .bind(p.restricted_guest()),
        &st.db,
    )
    .await
}

/// Plane `IssueViewViewSet.create`: any project member creates a view they own.
async fn project_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let id = insert_view(&st.db, &b, p.workspace_id(), Some(project_id), user.id).await?;
    created(
        view_by_id(&st.db, id, Some(project_id), p.workspace_id(), user.id)
            .await?
            .0,
    )
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
    view_by_id(&st.db, pk, Some(project_id), p.workspace_id(), user.id).await
}

async fn project_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    update_view(&st, &user, p.workspace_id(), Some(project_id), pk, &b).await
}

/// Plane `IssueViewViewSet.destroy`: project admin or the creator.
async fn project_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    delete_view(&st.db, user.id, p.is_admin(), p.workspace_id(), Some(project_id), pk).await
}

// ---------------------------------------------------------------------------------------------------
// Workspace views (project_id NULL)
// ---------------------------------------------------------------------------------------------------

/// Plane `WorkspaceViewViewSet.list`: own views plus public ones. Workspace guests see only their own.
async fn ws_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {VIEW_JSON} || jsonb_build_object('is_favorite', EXISTS (
                SELECT 1 FROM user_favorites f WHERE f.user_id = $2 AND f.entity_identifier = v.id
                  AND f.entity_type = 'view' AND f.project_id IS NULL AND f.deleted_at IS NULL))
             FROM issue_views v
             WHERE v.workspace_id = $1 AND v.project_id IS NULL AND v.deleted_at IS NULL AND v.archived_at IS NULL
               AND (v.owned_by_id = $2 OR (v.access = 1 AND $3 = false))
             ORDER BY v.created_at DESC"
        ))
        .bind(ws.workspace_id)
        .bind(user.id)
        .bind(ws.role == GUEST),
        &st.db,
    )
    .await
}

async fn ws_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let id = insert_view(&st.db, &b, ws.workspace_id, None, user.id).await?;
    created(view_by_id(&st.db, id, None, ws.workspace_id, user.id).await?.0)
}

/// Plane `WorkspaceViewViewSet.retrieve`: a view that is not visible to the caller is a 404.
async fn ws_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT owned_by_id, access = 1 FROM issue_views
         WHERE id = $1 AND workspace_id = $2 AND project_id IS NULL AND deleted_at IS NULL AND archived_at IS NULL",
    )
    .bind(pk)
    .bind(ws.workspace_id)
    .fetch_optional(&st.db)
    .await?;
    let (owner, public) = row.ok_or_else(ApiError::not_found)?;
    if owner != user.id && (!public || ws.role == GUEST) {
        return Err(ApiError::not_found());
    }
    view_by_id(&st.db, pk, None, ws.workspace_id, user.id).await
}

async fn ws_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    update_view(&st, &user, ws.workspace_id, None, pk, &b).await
}

/// Plane `WorkspaceViewViewSet.destroy`: workspace admin or the creator.
async fn ws_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    delete_view(&st.db, user.id, ws.is_admin(), ws.workspace_id, None, pk).await
}

// ---------------------------------------------------------------------------------------------------
// Workspace-wide work items (views' issue list)
// ---------------------------------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------------------------------
// User favorite views
// ---------------------------------------------------------------------------------------------------

/// Plane `IssueViewFavoriteViewSet.list`: the caller's view favorites in the workspace.
async fn fav_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(
        sqlx::query_scalar(
            "SELECT jsonb_build_object('id', f.id, 'entity_type', f.entity_type, 'entity_identifier', f.entity_identifier,
                 'name', v.name, 'is_folder', f.is_folder, 'sequence', f.sequence, 'parent', f.parent_id,
                 'project_id', f.project_id, 'workspace_id', f.workspace_id, 'user_id', f.user_id,
                 'logo_props', v.logo_props, 'created_at', f.created_at, 'updated_at', f.updated_at)
             FROM user_favorites f JOIN issue_views v ON v.id = f.entity_identifier AND v.deleted_at IS NULL
             WHERE f.user_id = $1 AND f.workspace_id = $2 AND f.entity_type = 'view' AND f.deleted_at IS NULL
             ORDER BY f.sequence",
        )
        .bind(user.id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

/// Plane `IssueViewFavoriteViewSet.create`: the view must be visible to the caller in this project,
/// else 404. Favoriting the same view twice is a no-op. Returns 204.
async fn fav_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let view = uuid_of(&b, "view").ok_or_else(ApiError::not_found)?;
    let visible: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM issue_views v WHERE v.id = $1 AND v.project_id = $2 AND v.workspace_id = $3
           AND v.deleted_at IS NULL AND v.archived_at IS NULL
           AND (v.owned_by_id = $4 OR (v.access = 1 AND NOT $5)))",
    )
    .bind(view)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .bind(p.restricted_guest())
    .fetch_one(&st.db)
    .await?;
    if !visible.0 {
        return Err(ApiError::not_found());
    }
    sqlx::query(
        "INSERT INTO user_favorites (id, entity_type, entity_identifier, is_folder, sequence, user_id, workspace_id,
             project_id, created_by_id, updated_by_id, created_at, updated_at)
         SELECT gen_random_uuid(), 'view', $1, false, 65535, $2, $3, $4, $2, $2, now(), now()
         WHERE NOT EXISTS (SELECT 1 FROM user_favorites WHERE user_id = $2 AND entity_type = 'view'
             AND entity_identifier = $1 AND project_id = $4 AND deleted_at IS NULL)",
    )
    .bind(view)
    .bind(user.id)
    .bind(p.workspace_id())
    .bind(project_id)
    .execute(&st.db)
    .await?;
    no_content()
}

/// Plane `IssueViewFavoriteViewSet.destroy`: missing favorite is a 404. Returns 204.
async fn fav_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, view_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let r = sqlx::query(
        "UPDATE user_favorites SET deleted_at = now() WHERE user_id = $1 AND workspace_id = $2 AND project_id = $3
         AND entity_type = 'view' AND entity_identifier = $4 AND deleted_at IS NULL",
    )
    .bind(user.id)
    .bind(p.workspace_id())
    .bind(project_id)
    .bind(view_id)
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}
