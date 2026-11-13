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
            "/api/workspaces/{slug}/projects/{project_id}/project-views/",
            get(project_list).post(project_user_views_or_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/views/{pk}/",
            get(project_retrieve).patch(project_update).delete(project_destroy),
        )
        .route("/api/workspaces/{slug}/views/", get(ws_list).post(ws_create))
        .route(
            "/api/workspaces/{slug}/workspace-views/",
            get(ws_list).post(ws_create_or_props),
        )
        .route(
            "/api/workspaces/{slug}/views/{pk}/",
            get(ws_retrieve).patch(ws_update).delete(ws_destroy),
        )
        .route(
            "/api/workspaces/{slug}/workspace-views/{pk}/",
            get(ws_retrieve).patch(ws_update).delete(ws_destroy),
        )
        .route("/api/workspaces/{slug}/issues/", get(ws_issues))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/intakes/",
            get(intake_list).post(intake_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/inboxes/",
            get(intake_list).post(intake_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/intakes/{pk}/",
            get(intake_retrieve).delete(intake_destroy),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/inboxes/{pk}/",
            get(intake_retrieve).delete(intake_destroy),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/intake-issues/",
            get(intake_issues_list).post(intake_issues_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/inbox-issues/",
            get(intake_issues_list).post(intake_issues_create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/intake-issues/{pk}/",
            get(intake_issue_retrieve)
                .patch(intake_issue_patch)
                .delete(intake_issue_destroy),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/inbox-issues/{pk}/",
            get(intake_issue_retrieve)
                .patch(intake_issue_patch)
                .delete(intake_issue_destroy),
        )
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

async fn ws_create_or_props(
    st: State<AppState>,
    user: CurrentUser,
    path: Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    if b.get("view_props").is_some() && b.get("name").is_none() {
        let ws = require_workspace(&st.db, &user, &path.0).await?;
        ws.require(&[ADMIN, MEMBER, GUEST])?;
        let view_props = b.get("view_props").cloned().unwrap_or_else(|| json!({}));
        sqlx::query(
            "UPDATE workspace_members SET view_props = $3, updated_at = now(), updated_by_id = $2
             WHERE workspace_id = $1 AND member_id = $2 AND is_active AND deleted_at IS NULL",
        )
        .bind(ws.workspace_id)
        .bind(user.id)
        .bind(view_props)
        .execute(&st.db)
        .await?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let (status, json) = ws_create(st, user, path, Json(b)).await?;
    Ok((status, json).into_response())
}

async fn project_user_views_or_create(
    st: State<AppState>,
    user: CurrentUser,
    path: Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    if b.get("name").is_some() && b.get("view_props").is_none() && b.get("preferences").is_none() {
        let (status, json) = project_create(st, user, path, Json(b)).await?;
        return Ok((status, json).into_response());
    }
    let p = require_project(&st.db, &user, &path.0.0, path.0.1).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let view_props = b.get("view_props");
    let default_props = b.get("default_props");
    let preferences = b.get("preferences");
    let sort_order = b.get("sort_order").and_then(Value::as_f64);

    sqlx::query(
        "UPDATE project_members SET
            view_props = COALESCE($3, view_props),
            default_props = COALESCE($4, default_props),
            preferences = COALESCE($5, preferences),
            sort_order = COALESCE($6, sort_order),
            updated_at = now(),
            updated_by_id = $2
         WHERE project_id = $1 AND member_id = $2 AND workspace_id = $7 AND is_active AND deleted_at IS NULL",
    )
    .bind(path.0.1)
    .bind(user.id)
    .bind(view_props)
    .bind(default_props)
    .bind(preferences)
    .bind(sort_order)
    .bind(p.workspace_id())
    .execute(&st.db)
    .await?;

    Ok(StatusCode::NO_CONTENT.into_response())
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
    let mut q = q;
    if let Some(view_id_str) = q.get("view_id").or_else(|| q.get("view")).cloned() {
        if let Ok(view_id) = Uuid::parse_str(&view_id_str) {
            if let Some(filters) = sqlx::query_scalar::<_, Value>(
                "SELECT filters FROM issue_views WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
            )
            .bind(view_id)
            .bind(ws.workspace_id)
            .fetch_optional(&st.db)
            .await?
            {
                if let Some(obj) = filters.as_object() {
                    for (k, v) in obj {
                        if !q.contains_key(k) {
                            if let Some(s) = v.as_str() {
                                q.insert(k.clone(), s.to_string());
                            } else if let Some(arr) = v.as_array() {
                                let items: Vec<String> =
                                    arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect();
                                if !items.is_empty() {
                                    q.insert(k.clone(), items.join(","));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
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

// ---------------------------------------------------------------------------------------------------
// Intake (Plane's `app/urls/intake.py`, `app/views/intake/base.py`)
// ---------------------------------------------------------------------------------------------------

/// `IntakeSerializer` fields plus `pending_issue_count` (intake_issues with status -2).
const INTAKE_JSON: &str = "jsonb_build_object('id', n.id, 'name', n.name, 'description', n.description,
    'is_default', n.is_default, 'view_props', n.view_props, 'logo_props', n.logo_props,
    'project', n.project_id, 'project_id', n.project_id, 'workspace', n.workspace_id, 'workspace_id', n.workspace_id,
    'created_at', n.created_at, 'updated_at', n.updated_at, 'created_by', n.created_by_id,
    'updated_by', n.updated_by_id, 'pending_issue_count', (
        SELECT count(*) FROM intake_issues ii WHERE ii.intake_id = n.id AND ii.status = -2 AND ii.deleted_at IS NULL))";

/// `IntakeIssueSerializer` row. `issue` is `IssueIntakeSerializer` (id, name, priority, sequence_id,
/// project_id, created_at, label_ids, created_by) and the issue's `label_ids`.
const INTAKE_ISSUE_JSON: &str =
    "jsonb_build_object('id', ii.id, 'status', ii.status, 'duplicate_to', ii.duplicate_to_id,
    'snoozed_till', ii.snoozed_till, 'source', ii.source, 'created_by', ii.created_by_id,
    'issue', jsonb_build_object('id', i.id, 'name', i.name, 'priority', i.priority, 'sequence_id', i.sequence_id,
        'project_id', i.project_id, 'created_at', i.created_at, 'created_by', i.created_by_id,
        'label_ids', COALESCE((SELECT jsonb_agg(il.label_id) FROM issue_labels il WHERE il.issue_id = i.id
                               AND il.deleted_at IS NULL), '[]'::jsonb)))";

/// Plane's `IntakeIssueDetailSerializer` adds the full issue. Built on the work-item row shape.
const INTAKE_ISSUE_DETAIL_JSON: &str = "jsonb_build_object('id', ii.id, 'status', ii.status,
    'duplicate_to', ii.duplicate_to_id, 'snoozed_till', ii.snoozed_till, 'source', ii.source,
    'duplicate_issue_detail', (SELECT jsonb_build_object('id', d.id, 'name', d.name, 'priority', d.priority,
        'sequence_id', d.sequence_id, 'project_id', d.project_id, 'created_at', d.created_at, 'created_by', d.created_by_id)
        FROM issues d WHERE d.id = ii.duplicate_to_id AND d.deleted_at IS NULL),
    'issue', jsonb_build_object('id', i.id, 'name', i.name, 'description_html', i.description_html,
        'description_stripped', i.description_stripped, 'priority', i.priority, 'state_id', i.state_id,
        'sequence_id', i.sequence_id, 'project_id', i.project_id, 'parent_id', i.parent_id,
        'start_date', i.start_date, 'target_date', i.target_date, 'completed_at', i.completed_at,
        'sort_order', i.sort_order, 'is_draft', i.is_draft, 'archived_at', i.archived_at,
        'created_at', i.created_at, 'updated_at', i.updated_at, 'created_by', i.created_by_id,
        'updated_by', i.updated_by_id, 'estimate_point', i.estimate_point_id, 'type_id', i.type_id,
        'label_ids', COALESCE((SELECT jsonb_agg(il.label_id) FROM issue_labels il WHERE il.issue_id = i.id
                               AND il.deleted_at IS NULL), '[]'::jsonb),
        'assignee_ids', COALESCE((SELECT jsonb_agg(a.assignee_id) FROM issue_assignees a
                                  WHERE a.issue_id = i.id AND a.deleted_at IS NULL), '[]'::jsonb),
        'module_ids', COALESCE((SELECT jsonb_agg(m.module_id) FROM module_issues m
                                WHERE m.issue_id = i.id AND m.deleted_at IS NULL), '[]'::jsonb)))";

/// Plane `IntakeViewSet` shared `Intake` row. Each project gets a default intake on demand, as
/// Plane creates one with the project.
async fn ensure_default_intake(db: &PgPool, workspace_id: Uuid, project_id: Uuid, user_id: Uuid) -> ApiResult<Uuid> {
    let id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO intakes (id, name, description, is_default, view_props, logo_props, project_id, workspace_id,
             created_by_id, updated_by_id, created_at, updated_at)
         SELECT gen_random_uuid(), 'Intake', '', true, '{}'::jsonb, '{}'::jsonb, $1, $2, $3, $3, now(), now()
         WHERE NOT EXISTS (SELECT 1 FROM intakes WHERE project_id = $1 AND deleted_at IS NULL)
         ON CONFLICT DO NOTHING
         RETURNING id",
    )
    .bind(project_id)
    .bind(workspace_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    match id {
        Some(id) => Ok(id),
        None => sqlx::query_scalar(
            "SELECT id FROM intakes WHERE project_id = $1 AND deleted_at IS NULL ORDER BY created_at LIMIT 1",
        )
        .bind(project_id)
        .fetch_one(db)
        .await
        .map_err(Into::into),
    }
}

/// Plane `IntakeViewSet.list`: a single object, the project's first intake.
// ponytail: Plane creates the default intake with the project. Here it is created on first read.
async fn intake_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let id = ensure_default_intake(&st.db, p.workspace_id(), project_id, user.id).await?;
    json_row(
        sqlx::query_scalar(&format!("SELECT {INTAKE_JSON} FROM intakes n WHERE n.id = $1")).bind(id),
        &st.db,
    )
    .await
}

async fn intake_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {INTAKE_JSON} FROM intakes n WHERE n.id = $1 AND n.project_id = $2 AND n.workspace_id = $3 AND n.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

/// Plane `IntakeViewSet.perform_create` (admin or member). Plane has no partial_update/update on this
/// viewset, so PATCH/PUT on `intakes/{pk}/` answer 405 there and are deliberately not routed here.
async fn intake_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let name = b.get("name").and_then(Value::as_str).unwrap_or("Intake").trim();
    if name.is_empty() {
        return Err(ApiError::bad("Name is required"));
    }
    let description = b.get("description").and_then(Value::as_str).unwrap_or("");
    let is_default = b.get("is_default").and_then(Value::as_bool).unwrap_or(false);
    let view_props = b.get("view_props").cloned().unwrap_or_else(|| json!({}));
    let logo_props = b.get("logo_props").cloned().unwrap_or_else(|| json!({}));

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO intakes (id, name, description, is_default, view_props, logo_props, project_id, workspace_id,
             created_by_id, updated_by_id, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9, now(), now())",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(is_default)
    .bind(view_props)
    .bind(logo_props)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .execute(&st.db)
    .await
    .map_err(|e| match &e {
        // DRF's UniqueTogetherValidator on (name, project, deleted_at) answers 400 before the insert.
        sqlx::Error::Database(d) if d.code().as_deref() == Some("23505") => ApiError::bad_body(
            json!({"non_field_errors": ["The fields name, project, deleted_at must make a unique set."]}),
        ),
        _ => e.into(),
    })?;

    let row = sqlx::query_scalar(&format!(
        "SELECT {INTAKE_JSON} FROM intakes n WHERE n.id = $1 AND n.project_id = $2 AND n.workspace_id = $3"
    ))
    .bind(id)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;

    created(row)
}

/// Plane `IntakeViewSet.destroy`: the default intake cannot be deleted.
async fn intake_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let row: Option<(bool,)> = sqlx::query_as(
        "SELECT is_default FROM intakes WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    let (is_default,) = row.ok_or_else(ApiError::not_found)?;
    if is_default {
        return Err(ApiError::bad("You cannot delete the default intake"));
    }
    sqlx::query("UPDATE intakes SET deleted_at = now() WHERE id = $1 AND project_id = $2 AND workspace_id = $3")
        .bind(pk)
        .bind(project_id)
        .bind(p.workspace_id())
        .execute(&st.db)
        .await?;
    no_content()
}

/// Plane `IntakeIssueViewSet.list`: intake issues filtered by status (default -2, pending).
/// Restricted guests see the intake issues they created.
async fn intake_issues_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let statuses: Vec<i32> = q
        .get("status")
        .map(String::as_str)
        .unwrap_or("-2")
        .split(',')
        .filter_map(|x| x.trim().parse().ok())
        .collect();
    let order = match q.get("order_by").map(String::as_str).unwrap_or("-issue__created_at") {
        "issue__created_at" => "i.created_at ASC",
        "-issue__created_at" => "i.created_at DESC",
        "issue__updated_at" => "i.updated_at ASC",
        "-issue__updated_at" => "i.updated_at DESC",
        "created_at" => "ii.created_at ASC",
        "-created_at" => "ii.created_at DESC",
        "updated_at" => "ii.updated_at ASC",
        "-updated_at" => "ii.updated_at DESC",
        "status" => "ii.status ASC",
        "-status" => "ii.status DESC",
        _ => "i.created_at DESC",
    };
    let per_page: i64 = q
        .get("per_page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let page: i64 = q
        .get("cursor")
        .and_then(|c| c.split(':').nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .max(0);
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {INTAKE_ISSUE_JSON} FROM intake_issues ii
         JOIN issues i ON i.id = ii.issue_id AND i.deleted_at IS NULL
         JOIN intakes n ON n.id = ii.intake_id
         WHERE ii.project_id = $1 AND ii.workspace_id = $2 AND n.project_id = $1 AND n.deleted_at IS NULL
           AND ii.deleted_at IS NULL AND ($3::int[] = '{{}}' OR ii.status = ANY($3))
           AND (NOT $4 OR ii.created_by_id = $5)
         ORDER BY {order} LIMIT {} OFFSET {}",
        per_page + 1,
        page * per_page
    ))
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(&statuses)
    .bind(p.restricted_guest())
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;
    let has_next = rows.len() as i64 > per_page;
    let results: Vec<Value> = rows.into_iter().take(per_page as usize).collect();
    let count = results.len() as i64;
    Ok(Json(json!({
        "count": count,
        "next": if has_next { Some(format!("{per_page}:{}:0", page + 1)) } else { None },
        "prev": if page > 0 { Some(format!("{per_page}:{}:1", page - 1)) } else { None },
        "next_cursor": format!("{per_page}:{}:0", page + 1),
        "prev_cursor": format!("{per_page}:{}:1", page - 1),
        "next_page_results": has_next,
        "prev_page_results": page > 0,
        "results": results,
    })))
}

/// Plane `IntakeIssueViewSet.create`: create a triage work item and its intake row (status -2).
async fn intake_issues_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let issue = b.get("issue").cloned().unwrap_or_else(|| json!({}));
    let name = match issue.get("name") {
        Some(Value::String(n)) if !n.trim().is_empty() => n.trim().to_string(),
        _ => return Err(ApiError::bad("Name is required")),
    };
    let priority = s(&issue, "priority").unwrap_or("none");
    if !["low", "medium", "high", "urgent", "none"].contains(&priority) {
        return Err(ApiError::bad("Invalid priority"));
    }
    let description = match s(&issue, "description_html") {
        Some(h) => sanitize_html(h)?,
        None => "<p></p>".to_string(),
    };
    let stripped = strip_tags(&description);
    let intake_id = ensure_default_intake(&st.db, p.workspace_id(), project_id, user.id).await?;
    let mut tx = st.db.begin().await?;

    // Triage state: Plane creates one if the project has none.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1::text))")
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
    let triage: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM states WHERE project_id = $1 AND is_triage AND deleted_at IS NULL LIMIT 1")
            .bind(project_id)
            .fetch_optional(&mut *tx)
            .await?;
    let triage = match triage {
        Some(t) => t,
        None => {
            sqlx::query_scalar(
                "INSERT INTO states (name, color, \"group\", \"default\", is_triage, sequence, slug, project_id, workspace_id,
                     created_by_id, updated_by_id)
                 VALUES ('Triage', '#4E5355', 'triage', false, true, 65000, 'triage', $1, $2, $3, $3) RETURNING id",
            )
            .bind(project_id)
            .bind(p.workspace_id())
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?
        }
    };

    // Sequence: next number after the highest one ever issued in this project, per Plane's IssueSequence.
    let seq: i64 =
        sqlx::query_scalar("SELECT COALESCE(max(sequence), 0) + 1 FROM issue_sequences WHERE project_id = $1")
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
    let issue_id: Uuid = sqlx::query_scalar(
        "INSERT INTO issues (id, name, description_html, description_stripped, description_json, priority, sequence_id,
             project_id, workspace_id, state_id, sort_order, is_draft, created_by_id, updated_by_id, created_at, updated_at)
         VALUES (gen_random_uuid(), $1, $2, $3, '{}'::jsonb, $4, $5, $6, $7, $8, 65535, false, $9, $9, now(), now())
         RETURNING id",
    )
    .bind(&name)
    .bind(&description)
    .bind(&stripped)
    .bind(priority)
    .bind(seq as i32)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(triage)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO issue_sequences (id, sequence, deleted, issue_id, project_id, workspace_id, created_by_id, updated_by_id, created_at, updated_at)
         VALUES (gen_random_uuid(), $1, false, $2, $3, $4, $5, $5, now(), now())",
    )
    .bind(seq)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    // ponytail: labels, assignees, and activity rows from the create body are not written yet.
    let ii_id: Uuid = sqlx::query_scalar(
        "INSERT INTO intake_issues (id, status, source, intake_id, issue_id, project_id, workspace_id, created_by_id,
             updated_by_id, created_at, updated_at)
         VALUES (gen_random_uuid(), -2, 'IN_APP', $1, $2, $3, $4, $5, $5, now(), now())
         RETURNING id",
    )
    .bind(intake_id)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    crate::activity::record(&mut *tx, crate::activity::ActivityEvent::new(issue_id, project_id, p.workspace_id(), user.id, "created", "created the issue")).await?;
    let activities = crate::notify::snapshots(&mut tx, issue_id, project_id, p.workspace_id()).await?;
    tx.commit().await?;
    crate::notify::after_activity(&st, issue_id, project_id, p.workspace_id(), user.id, activities);
    let v = intake_issue_detail(&st.db, ii_id, project_id, p.workspace_id()).await?;
    created(v)
}

/// Single intake issue row in the detail shape (Plane `IntakeIssueDetailSerializer`).
async fn intake_issue_detail(db: &PgPool, ii_id: Uuid, project_id: Uuid, workspace_id: Uuid) -> ApiResult<Value> {
    let v: Option<Value> = sqlx::query_scalar(&format!(
        "SELECT {INTAKE_ISSUE_DETAIL_JSON} FROM intake_issues ii JOIN issues i ON i.id = ii.issue_id
         WHERE ii.id = $1 AND ii.project_id = $2 AND ii.workspace_id = $3 AND ii.deleted_at IS NULL"
    ))
    .bind(ii_id)
    .bind(project_id)
    .bind(workspace_id)
    .fetch_optional(db)
    .await?;
    v.ok_or_else(ApiError::not_found)
}

/// Plane `IntakeIssueViewSet.retrieve`. `pk` is the work item id, as the frontend sends it.
/// A restricted guest sees only the intake issues they created.
async fn intake_issue_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let row: Option<(Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT ii.id, ii.created_by_id FROM intake_issues ii
         WHERE ii.issue_id = $1 AND ii.project_id = $2 AND ii.workspace_id = $3 AND ii.deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    let (ii_id, creator) = row.ok_or_else(ApiError::not_found)?;
    if p.restricted_guest() && creator != Some(user.id) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            json!({"error": "You are not allowed to view this issue"}),
        ));
    }
    Ok(Json(
        intake_issue_detail(&st.db, ii_id, project_id, p.workspace_id()).await?,
    ))
}

/// Plane `IntakeIssueViewSet.partial_update`. Editing the work item: any member, guests only their own
/// issues. Changing the intake status (accept, decline, snooze, duplicate) needs project admin or
/// workspace admin; other callers have `status` silently ignored, as in Plane.
async fn intake_issue_patch(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let ws_admin = p.ws.is_admin();
    let row: Option<(Uuid, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT ii.id, ii.created_by_id, st.group::text FROM intake_issues ii
         JOIN issues i ON i.id = ii.issue_id AND i.deleted_at IS NULL
         LEFT JOIN states st ON st.id = i.state_id
         WHERE ii.issue_id = $1 AND ii.project_id = $2 AND ii.workspace_id = $3 AND ii.deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    let (ii_id, creator, issue_group) = row.ok_or_else(ApiError::not_found)?;
    if p.role <= GUEST && !ws_admin && creator != Some(user.id) {
        return Err(ApiError::bad("You cannot edit intake issues"));
    }
    let can_triage = p.role > MEMBER || ws_admin;

    // Validate everything before the first write.
    let issue = b.get("issue").filter(|v| v.is_object());
    let name = match issue.and_then(|i| i.get("name")) {
        None | Some(Value::Null) => None,
        Some(Value::String(n)) if !n.trim().is_empty() => Some(n.trim().to_string()),
        Some(_) => return Err(ApiError::bad_body(json!({"name": ["This field may not be blank."]}))),
    };
    let priority = match issue.and_then(|i| s(i, "priority")) {
        Some(pr) if p.role > GUEST && ["low", "medium", "high", "urgent", "none"].contains(&pr) => Some(pr.to_string()),
        Some(pr) if p.role > GUEST => {
            return Err(ApiError::bad_body(
                json!({"priority": [format!("\"{pr}\" is not a valid choice.")]}),
            ));
        }
        _ => None,
    };
    let description = match issue.and_then(|i| s(i, "description_html")) {
        Some(h) => Some(sanitize_html(h)?),
        None => None,
    };
    let stripped = description.as_deref().map(strip_tags);

    let status = if can_triage {
        b.get("status").and_then(Value::as_i64)
    } else {
        None
    };
    if let Some(code) = status
        && ![-2, -1, 0, 1, 2].contains(&code)
    {
        return Err(ApiError::bad_body(
            json!({"status": [format!("\"{code}\" is not a valid choice.")]}),
        ));
    }
    let snooze_set = can_triage && b.get("snoozed_till").is_some();
    let snoozed_till = if snooze_set {
        s(&b, "snoozed_till").map(String::from)
    } else {
        None
    };
    // ponytail: Plane snaps snoozed_till to the start of the user's day (snooze_day_start). Stored as sent.
    let duplicate_to = if can_triage { uuid_of(&b, "duplicate_to") } else { None };

    let mut tx = st.db.begin().await?;
    if name.is_some() || priority.is_some() || description.is_some() {
        sqlx::query(
            "UPDATE issues SET name = COALESCE($2, name), priority = COALESCE($3, priority),
                 description_html = COALESCE($4, description_html), description_stripped = COALESCE($5, description_stripped),
                 updated_by_id = $6, updated_at = now()
             WHERE id = $1 AND project_id = $7 AND workspace_id = $8",
        )
        .bind(pk)
        .bind(name)
        .bind(priority)
        .bind(description)
        .bind(stripped)
        .bind(user.id)
        .bind(project_id)
        .bind(p.workspace_id())
        .execute(&mut *tx)
        .await?;
    }
    if let Some(code) = status {
        let old_status: i32 = sqlx::query_scalar("SELECT status FROM intake_issues WHERE id=$1 AND project_id=$2 AND workspace_id=$3 AND deleted_at IS NULL FOR UPDATE")
            .bind(ii_id).bind(project_id).bind(p.workspace_id()).fetch_one(&mut *tx).await?;
        let name = |status| match status {
            -2 => Some("Pending"),
            -1 => Some("Rejected"),
            0 => Some("Snoozed"),
            1 => Some("Accepted"),
            2 => Some("Duplicate"),
            _ => None,
        };
        let verb = code.to_string();
        let mut event = crate::activity::ActivityEvent::new(
            pk,
            project_id,
            p.workspace_id(),
            user.id,
            &verb,
            "updated the intake status",
        );
        event.field = Some("intake");
        event.old_value = name(i64::from(old_status));
        event.new_value = name(code);
        crate::activity::record(&mut *tx, event).await?;
        // Accepting a triage item moves it to the project's default state, as Plane does.
        if code == 1 && issue_group.as_deref() == Some("triage") {
            let default: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM states WHERE project_id = $1 AND workspace_id = $2 AND \"default\" AND NOT is_triage
                   AND deleted_at IS NULL LIMIT 1",
            )
            .bind(project_id)
            .bind(p.workspace_id())
            .fetch_optional(&mut *tx)
            .await?;
            let default = default.ok_or_else(|| {
                ApiError::bad_body(
                    json!({"status": "Cannot accept intake issue: No default state found for the project"}),
                )
            })?;
            sqlx::query("UPDATE issues SET state_id = $2, updated_at = now() WHERE id = $1")
                .bind(pk)
                .bind(default)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "UPDATE intake_issues SET status = $2,
                 snoozed_till = CASE WHEN $3 THEN $4::timestamptz ELSE snoozed_till END,
                 duplicate_to_id = COALESCE($5, duplicate_to_id),
                 updated_by_id = $6, updated_at = now()
             WHERE id = $1",
        )
        .bind(ii_id)
        .bind(code as i32)
        .bind(snooze_set)
        .bind(snoozed_till)
        .bind(duplicate_to)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    } else if snooze_set || duplicate_to.is_some() {
        sqlx::query(
            "UPDATE intake_issues SET snoozed_till = CASE WHEN $2 THEN $3::timestamptz ELSE snoozed_till END,
                 duplicate_to_id = COALESCE($4, duplicate_to_id), updated_by_id = $5, updated_at = now()
             WHERE id = $1",
        )
        .bind(ii_id)
        .bind(snooze_set)
        .bind(snoozed_till)
        .bind(duplicate_to)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(
        intake_issue_detail(&st.db, ii_id, project_id, p.workspace_id()).await?,
    ))
}

/// Plane `IntakeIssueViewSet.destroy`: project admin, workspace admin, or the creator. Declined,
/// snoozed, pending and duplicate items also soft delete their work item. Accepted ones keep it.
async fn intake_issue_destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN])?;
    let row: Option<(Uuid, i32, Option<Uuid>)> = sqlx::query_as(
        "SELECT ii.id, ii.status, ii.created_by_id FROM intake_issues ii
         WHERE ii.issue_id = $1 AND ii.project_id = $2 AND ii.workspace_id = $3 AND ii.deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?;
    let (ii_id, status, creator) = row.ok_or_else(ApiError::not_found)?;
    if !(p.is_admin() || p.ws.is_admin() || creator == Some(user.id)) {
        return Err(ApiError::forbidden());
    }
    let mut tx = st.db.begin().await?;
    sqlx::query("UPDATE intake_issues SET deleted_at = now() WHERE id = $1")
        .bind(ii_id)
        .execute(&mut *tx)
        .await?;
    if [-2, -1, 0, 2].contains(&status) {
        sqlx::query("UPDATE issues SET deleted_at = now() WHERE id = $1 AND project_id = $2 AND workspace_id = $3")
            .bind(pk)
            .bind(project_id)
            .bind(p.workspace_id())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    no_content()
}
