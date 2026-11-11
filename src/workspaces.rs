//! Workspaces, members, invitations, favorites, workspace-level user data.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::*;
use crate::instance::{RESTRICTED_SLUGS, slug_ok};
use crate::users::{USER_LITE_JSON, patch_sets};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/users/me/workspaces/", get(user_workspaces))
        .route("/api/workspaces/", get(user_workspaces).post(create))
        .route("/api/workspace-slug-check/", get(slug_check))
        .route("/api/workspaces/{slug}/", get(retrieve).patch(update).delete(destroy))
        .route("/api/workspaces/{slug}/members/", get(members_list))
        .route("/api/workspaces/{slug}/members/leave/", post(leave))
        .route(
            "/api/workspaces/{slug}/members/{pk}/",
            get(member_retrieve).patch(member_update).delete(member_remove),
        )
        .route("/api/workspaces/{slug}/workspace-members/me/", get(member_me))
        .route("/api/workspaces/{slug}/project-members/", get(project_members_map))
        .route("/api/users/me/workspaces/{slug}/project-roles/", get(project_roles))
        .route(
            "/api/workspaces/{slug}/user-properties/",
            get(user_properties_get).patch(user_properties_patch),
        )
        .route("/api/users/last-visited-workspace/", get(last_visited_workspace))
        .route(
            "/api/workspaces/{slug}/invitations/",
            get(invites_list).post(invites_create),
        )
        .route(
            "/api/workspaces/{slug}/invitations/{pk}/",
            get(invite_public).patch(invite_update).delete(invite_delete),
        )
        .route("/api/workspaces/{slug}/invitations/{pk}/join/", post(invite_join))
        .route(
            "/api/users/me/workspaces/invitations/",
            get(my_invites_list).post(my_invites_accept),
        )
        .route(
            "/api/workspaces/{slug}/user-favorites/",
            get(favorites_list).post(favorites_create),
        )
        .route(
            "/api/workspaces/{slug}/user-favorites/{pk}/",
            patch(favorites_update).delete(favorites_delete),
        )
        .route("/api/workspaces/{slug}/user-favorites/{pk}/group/", get(favorite_group))
        .route("/api/workspaces/{slug}/user-stats/{user_id}/", get(user_stats))
        .route("/api/workspaces/{slug}/user-profile/{user_id}/", get(user_profile))
        .route("/api/workspaces/{slug}/user-activity/{user_id}/", get(user_activity))
        .route(
            "/api/workspaces/{slug}/sidebar-preferences/",
            get(sidebar_prefs_list).patch(sidebar_prefs_bulk),
        )
        .route(
            "/api/workspaces/{slug}/sidebar-preferences/{key}/",
            patch(sidebar_pref_update),
        )
        .route("/api/workspaces/{slug}/home-preferences/", get(home_prefs_list))
        .route(
            "/api/workspaces/{slug}/home-preferences/{key}/",
            patch(home_pref_update),
        )
        .route(
            "/api/workspaces/{slug}/webhooks/",
            get(webhooks_list).post(webhook_create),
        )
        .route(
            "/api/workspaces/{slug}/webhooks/{pk}/",
            get(webhook_retrieve).patch(webhook_update).delete(webhook_delete),
        )
        .route(
            "/api/workspaces/{slug}/webhooks/{pk}/regenerate/",
            post(webhook_regenerate),
        )
        .route("/api/workspaces/{slug}/webhook-logs/{webhook_id}/", get(webhook_logs))
        .route(
            "/api/workspaces/{slug}/workspace-themes/",
            get(themes_list).post(theme_create),
        )
        .route(
            "/api/workspaces/{slug}/workspace-themes/{pk}/",
            get(theme_retrieve).patch(theme_update).delete(theme_delete),
        )
}

/// WorkSpaceSerializer fields, with `total_members` (active, non-bot) and `logo_url`.
/// `role` is added by the caller when the query knows it.
const WS_JSON: &str = "to_jsonb(w) - 'owner_id' - 'created_by_id' - 'updated_by_id' - 'logo_asset_id'
    || jsonb_build_object('owner', w.owner_id, 'created_by', w.created_by_id, 'updated_by', w.updated_by_id,
       'logo_asset', w.logo_asset_id, 'logo_url', NULLIF(w.logo, ''),
       'total_members', (SELECT count(*) FROM workspace_members m JOIN users u ON u.id = m.member_id
                         WHERE m.workspace_id = w.id AND m.is_active AND m.deleted_at IS NULL AND NOT u.is_bot))";

/// Plane's `WorkspaceUserSerializer` path, the caller's own workspaces with role and member count.
async fn user_workspaces(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {WS_JSON} || jsonb_build_object('role', m.role) FROM workspaces w
             JOIN workspace_members m ON m.workspace_id = w.id AND m.member_id = $1 AND m.is_active AND m.deleted_at IS NULL
             WHERE w.deleted_at IS NULL ORDER BY w.name"
        ))
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn slug_check(
    State(st): State<AppState>,
    _user: CurrentUser,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let slug = q.get("slug").map(String::as_str).unwrap_or_default();
    if slug.is_empty() {
        return Err(ApiError::bad("Workspace Slug is required"));
    }
    let taken: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM workspaces WHERE slug = $1)")
        .bind(slug)
        .fetch_one(&st.db)
        .await?;
    Ok(Json(json!({"status": !taken.0 && !RESTRICTED_SLUGS.contains(&slug)})))
}

async fn retrieve(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {WS_JSON} || jsonb_build_object('role', $2::smallint) FROM workspaces w
             WHERE w.id = $1 AND w.deleted_at IS NULL"
        ))
        .bind(ws.workspace_id)
        .bind(ws.role),
        &st.db,
    )
    .await
}

async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let disabled: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT value FROM instance_configurations WHERE key = 'DISABLE_WORKSPACE_CREATION' AND deleted_at IS NULL",
    )
    .fetch_optional(&st.db)
    .await?;
    if disabled.and_then(|(v,)| v).is_some_and(|v| v == "1") {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            json!({"error": "Workspace creation is not allowed"}),
        ));
    }
    let name = s(&b, "name").unwrap_or("");
    let slug = s(&b, "slug").unwrap_or("");
    if name.is_empty() || slug.is_empty() {
        return Err(ApiError::bad("Both name and slug are required"));
    }
    if name.chars().count() > 80 || slug.len() > 48 {
        return Err(ApiError::bad("The maximum length for name is 80 and for slug is 48"));
    }
    if !slug_ok(slug) {
        return Err(ApiError::bad_body(json!({"slug": ["Slug is not valid"]})));
    }
    let mut tx = st.db.begin().await?;
    let id = create_workspace(&mut tx, user.id, name, slug, s(&b, "organization_size").unwrap_or(""))
        .await
        .map_err(|e| {
            if e.0 == StatusCode::CONFLICT {
                ApiError(
                    StatusCode::CONFLICT,
                    json!({"slug": "The workspace with the slug already exists"}),
                )
            } else {
                e
            }
        })?;
    tx.commit().await?;
    let v: Value = sqlx::query_scalar(&format!(
        "SELECT {WS_JSON} || jsonb_build_object('role', 20::smallint) FROM workspaces w WHERE w.id = $1"
    ))
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    created(v)
}

/// Plane: PATCH needs admin. Only writable workspace columns; slug and owner are not editable here.
async fn update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    crate::users::validate_timezone(&st.db, &b, "timezone").await?;
    if let Some(n) = s(&b, "name")
        && (n.trim().is_empty() || n.chars().count() > 80)
    {
        return Err(ApiError::bad_body(json!({"name": ["Name is not valid"]})));
    }
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE workspaces w SET name = COALESCE($2, name), logo = COALESCE($3, logo),
                organization_size = COALESCE($4, organization_size), timezone = COALESCE($5, timezone),
                background_color = COALESCE($6, background_color), updated_at = now(), updated_by_id = $7
             WHERE w.id = $1 AND w.deleted_at IS NULL
             RETURNING {WS_JSON} || jsonb_build_object('role', $8::smallint)"
        ))
        .bind(ws.workspace_id)
        .bind(s(&b, "name").map(str::trim))
        .bind(s(&b, "logo"))
        .bind(s(&b, "organization_size"))
        .bind(s(&b, "timezone"))
        .bind(s(&b, "background_color"))
        .bind(user.id)
        .bind(ws.role),
        &st.db,
    )
    .await
}

/// Plane: DELETE needs admin. Soft delete; clear `last_workspace_id` pointers like Plane does.
async fn destroy(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let mut tx = st.db.begin().await?;
    sqlx::query("UPDATE profiles SET last_workspace_id = NULL WHERE last_workspace_id = $1")
        .bind(ws.workspace_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE workspaces SET deleted_at = now(), updated_at = now(), updated_by_id = $2 WHERE id = $1")
        .bind(ws.workspace_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    no_content()
}

/// Insert a workspace and its owner as admin member. Shared by the create endpoint and `saturn seed`.
/// Roles a workspace admin may hand out. Plane: guests can be demoted only when they hold no other project role.
const WS_ROLE_VALUES: &[i16] = &[ADMIN, MEMBER, GUEST];

/// WorkspaceMemberAdminSerializer for admins and members, WorkSpaceMemberSerializer for guests.
const MEMBER_JSON: &str =
    "jsonb_build_object('id', m.id, 'role', m.role, 'member', {USER}, 'workspace', m.workspace_id,
    'is_active', m.is_active, 'created_at', m.created_at, 'updated_at', m.updated_at, 'view_props', m.view_props,
    'default_props', m.default_props, 'issue_props', m.issue_props, 'company_role', m.company_role,
    'created_by', m.created_by_id, 'updated_by', m.updated_by_id, 'deleted_at', m.deleted_at)";

async fn members_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {} FROM workspace_members m JOIN users u ON u.id = m.member_id
             WHERE m.workspace_id = $1 AND m.is_active AND m.deleted_at IS NULL AND NOT u.is_bot ORDER BY m.created_at",
            MEMBER_JSON.replace(
                "{USER}",
                &format!("{USER_LITE_JSON} || jsonb_build_object('email', CASE WHEN $2 THEN u.email END)")
            )
        ))
        .bind(ws.workspace_id)
        .bind(ws.is_admin()),
        &st.db,
    )
    .await
}

async fn member_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {} FROM workspace_members m JOIN users u ON u.id = m.member_id
             WHERE m.id = $1 AND m.workspace_id = $2 AND m.is_active AND m.deleted_at IS NULL",
            MEMBER_JSON.replace(
                "{USER}",
                &format!("{USER_LITE_JSON} || jsonb_build_object('email', CASE WHEN $3 THEN u.email END)")
            )
        ))
        .bind(pk)
        .bind(ws.workspace_id)
        .bind(ws.is_admin()),
        &st.db,
    )
    .await
}

/// Plane: role change (admin only). Demoting to guest also drops project roles to guest.
async fn member_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let Some(obj) = b.as_object().filter(|o| o.len() == 1 && o.contains_key("role")) else {
        return Err(ApiError::bad("Only the role of a workspace member can be updated"));
    };
    let role = obj["role"]
        .as_i64()
        .filter(|r| WS_ROLE_VALUES.iter().any(|v| i64::from(*v) == *r));
    let Some(role) = role.map(|r| r as i16) else {
        return Err(ApiError::bad_body(
            json!({"role": ["\"Invalid role\" is not a valid choice."]}),
        ));
    };
    let target: (Uuid, bool) = sqlx::query_as(
        "SELECT m.member_id, u.is_bot FROM workspace_members m JOIN users u ON u.id = m.member_id
         WHERE m.id = $1 AND m.workspace_id = $2 AND m.is_active AND m.deleted_at IS NULL",
    )
    .bind(pk)
    .bind(ws.workspace_id)
    .fetch_one(&st.db)
    .await?;
    if target.0 == user.id {
        return Err(ApiError::bad("You cannot update your own role"));
    }
    if role == GUEST && is_sole_admin_of_any_project(&st.db, ws.workspace_id, target.0).await? {
        return Err(ApiError::bad(
            "User is a part of some projects where they are the only admin, they should either leave that project or promote another user to admin.",
        ));
    }
    let mut tx = st.db.begin().await?;
    if role == GUEST {
        sqlx::query(
            "UPDATE project_members SET role = $3, updated_at = now() WHERE workspace_id = $1 AND member_id = $2",
        )
        .bind(ws.workspace_id)
        .bind(target.0)
        .bind(GUEST)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE workspace_members SET role = $2, updated_at = now(), updated_by_id = $3 WHERE id = $1")
        .bind(pk)
        .bind(role)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {} FROM workspace_members m JOIN users u ON u.id = m.member_id WHERE m.id = $1",
            MEMBER_JSON.replace(
                "{USER}",
                &format!("{USER_LITE_JSON} || jsonb_build_object('email', u.email)")
            )
        ))
        .bind(pk),
        &st.db,
    )
    .await
}

/// True when `member` is the only admin of a project in the workspace (Plane `is_sole_admin_of_any_project`).
async fn is_sole_admin_of_any_project(db: &sqlx::PgPool, workspace_id: Uuid, member: Uuid) -> Result<bool, ApiError> {
    let r: (bool,) = sqlx::query_as(
        "SELECT EXISTS (
            SELECT 1 FROM projects p
            WHERE p.workspace_id = $1 AND p.deleted_at IS NULL
              AND (SELECT count(*) FROM project_members pm WHERE pm.project_id = p.id AND pm.is_active AND pm.deleted_at IS NULL) = 1
              AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = p.id AND pm.member_id = $2
                          AND pm.role = 20 AND pm.is_active AND pm.deleted_at IS NULL))",
    )
    .bind(workspace_id)
    .bind(member)
    .fetch_one(db)
    .await?;
    Ok(r.0)
}

/// Plane: remove a member (admin only). Deactivates the member and their project memberships.
async fn member_remove(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let target: (Uuid, Uuid, i16) = sqlx::query_as(
        "SELECT m.id, m.member_id, m.role FROM workspace_members m JOIN users u ON u.id = m.member_id
         WHERE m.id = $1 AND m.workspace_id = $2 AND NOT u.is_bot AND m.is_active AND m.deleted_at IS NULL",
    )
    .bind(pk)
    .bind(ws.workspace_id)
    .fetch_one(&st.db)
    .await?;
    if target.1 == user.id {
        return Err(ApiError::bad(
            "You cannot remove yourself from the workspace. Please use leave workspace",
        ));
    }
    if ws.role < target.2 {
        return Err(ApiError::bad("You cannot remove a user having role higher than you"));
    }
    if is_sole_admin_of_any_project(&st.db, ws.workspace_id, target.1).await? {
        return Err(ApiError::bad(
            "User is a part of some projects where they are the only admin, they should either leave that project or promote another user to admin.",
        ));
    }
    deactivate_member(&st.db, ws.workspace_id, target.0, target.1).await?;
    no_content()
}

async fn deactivate_member(db: &sqlx::PgPool, workspace_id: Uuid, wm_id: Uuid, member: Uuid) -> Result<(), ApiError> {
    let mut tx = db.begin().await?;
    sqlx::query("UPDATE project_members SET is_active = false, updated_at = now() WHERE workspace_id = $1 AND member_id = $2 AND is_active")
        .bind(workspace_id)
        .bind(member)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE workspace_members SET is_active = false, updated_at = now() WHERE id = $1")
        .bind(wm_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Plane: the caller leaves a workspace. Blocked when they are its only admin or sole project admin.
async fn leave(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    if ws.role == ADMIN {
        let admins: (i64,) = sqlx::query_as(
            "SELECT count(*) FROM workspace_members WHERE workspace_id = $1 AND role = 20 AND is_active AND deleted_at IS NULL",
        )
        .bind(ws.workspace_id)
        .fetch_one(&st.db)
        .await?;
        if admins.0 <= 1 {
            return Err(ApiError::bad(
                "You cannot leave the workspace as you are the only admin of the workspace you will have to either delete the workspace or promote another user to admin.",
            ));
        }
    }
    if is_sole_admin_of_any_project(&st.db, ws.workspace_id, user.id).await? {
        return Err(ApiError::bad(
            "You are a part of some projects where you are the only admin, you should either leave the project or promote another user to admin.",
        ));
    }
    let wm: (Uuid,) = sqlx::query_as(
        "SELECT id FROM workspace_members WHERE workspace_id = $1 AND member_id = $2 AND is_active AND deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    deactivate_member(&st.db, ws.workspace_id, wm.0, user.id).await?;
    no_content()
}

/// Plane: the caller's own membership row (`WorkspaceMemberMeSerializer`, `__all__` plus draft count).
async fn member_me(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    json_row(
        sqlx::query_scalar(
            "SELECT to_jsonb(m) || jsonb_build_object('draft_issue_count', (
                SELECT count(*) FROM draft_issues d WHERE d.created_by_id = $2 AND d.workspace_id = $1 AND d.deleted_at IS NULL))
             FROM workspace_members m WHERE m.workspace_id = $1 AND m.member_id = $2 AND m.is_active AND m.deleted_at IS NULL",
        )
        .bind(ws.workspace_id)
        .bind(user.id),
        &st.db,
    )
    .await
}

/// Plane: `{project_id: [{id, role, member, original_role, created_at}]}` for projects the caller is in.
async fn project_members_map(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT pm.project_id::text, jsonb_build_object('id', pm.id, 'role', pm.role, 'member', pm.member_id,
                'original_role', pm.role, 'created_at', pm.created_at)
         FROM project_members pm
         WHERE pm.workspace_id = $1 AND pm.is_active AND pm.deleted_at IS NULL
           AND pm.project_id IN (SELECT project_id FROM project_members WHERE member_id = $2 AND is_active AND deleted_at IS NULL)
         ORDER BY pm.created_at",
    )
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;
    let mut out = serde_json::Map::new();
    for (pid, v) in rows {
        out.entry(pid)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(v);
    }
    Ok(Json(Value::Object(out)))
}

/// Plane: `{project_id: role}` for the caller across all workspace projects.
async fn project_roles(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let rows: Vec<(String, i16)> = sqlx::query_as(
        "SELECT project_id::text, role FROM project_members
         WHERE workspace_id = $1 AND member_id = $2 AND is_active AND deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(Value::Object(
        rows.into_iter().map(|(p, r)| (p, json!(r))).collect(),
    )))
}

const USER_PROPERTIES_FIELDS: &[(&str, &str)] = &[
    ("filters", "jsonb"),
    ("display_filters", "jsonb"),
    ("display_properties", "jsonb"),
    ("rich_filters", "jsonb"),
    ("navigation_project_limit", "int4"),
    ("navigation_control_preference", "text"),
];

const USER_PROPERTIES_JSON: &str = "to_jsonb(p) - 'user_id' - 'workspace_id' - 'created_by_id' - 'updated_by_id'
    || jsonb_build_object('user', p.user_id, 'workspace', p.workspace_id)";

/// Get-or-create the caller's properties row for the workspace, then return it.
async fn user_properties_row(st: &AppState, user: &CurrentUser, ws_id: Uuid) -> Result<Value, ApiError> {
    sqlx::query(
        "INSERT INTO workspace_user_properties (id, user_id, workspace_id, created_by_id, updated_by_id)
         SELECT gen_random_uuid(), $1, $2, $1, $1 WHERE NOT EXISTS (
            SELECT 1 FROM workspace_user_properties WHERE user_id = $1 AND workspace_id = $2 AND deleted_at IS NULL)",
    )
    .bind(user.id)
    .bind(ws_id)
    .execute(&st.db)
    .await?;
    Ok(sqlx::query_scalar(&format!(
        "SELECT {USER_PROPERTIES_JSON} FROM workspace_user_properties p
         WHERE p.user_id = $1 AND p.workspace_id = $2 AND p.deleted_at IS NULL"
    ))
    .bind(user.id)
    .bind(ws_id)
    .fetch_one(&st.db)
    .await?)
}

async fn user_properties_get(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    Ok(Json(user_properties_row(&st, &user, ws.workspace_id).await?))
}

async fn user_properties_patch(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    user_properties_row(&st, &user, ws.workspace_id).await?;
    let sets = patch_sets(&b, USER_PROPERTIES_FIELDS, 3);
    if sets.is_empty() {
        return Ok(Json(user_properties_row(&st, &user, ws.workspace_id).await?));
    }
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE workspace_user_properties p SET {}, updated_at = now(), updated_by_id = $1
             WHERE p.user_id = $1 AND p.workspace_id = $2 AND p.deleted_at IS NULL
             RETURNING {USER_PROPERTIES_JSON}",
            sets.join(", ")
        ))
        .bind(user.id)
        .bind(ws.workspace_id)
        .bind(&b),
        &st.db,
    )
    .await
}

/// Plane: the caller's last workspace with its project memberships (`profiles.last_workspace_id`).
async fn last_visited_workspace(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    let last: Option<(Option<Uuid>,)> = sqlx::query_as("SELECT last_workspace_id FROM profiles WHERE user_id = $1")
        .bind(user.id)
        .fetch_optional(&st.db)
        .await?;
    let Some(ws_id) = last.and_then(|(l,)| l) else {
        return Ok(Json(json!({"project_details": [], "workspace_details": {}})));
    };
    let ws: Option<Value> = sqlx::query_scalar(&format!(
        "SELECT {WS_JSON} || jsonb_build_object('role', m.role) FROM workspaces w
         JOIN workspace_members m ON m.workspace_id = w.id AND m.member_id = $2 AND m.is_active AND m.deleted_at IS NULL
         WHERE w.id = $1 AND w.deleted_at IS NULL"
    ))
    .bind(ws_id)
    .bind(user.id)
    .fetch_optional(&st.db)
    .await?;
    let projects: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(pm) FROM project_members pm WHERE pm.workspace_id = $1 AND pm.member_id = $2 AND pm.deleted_at IS NULL",
    )
    .bind(ws_id)
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(
        json!({"workspace_details": ws.unwrap_or(json!({})), "project_details": projects}),
    ))
}

pub async fn create_workspace(
    tx: &mut sqlx::PgConnection,
    owner: Uuid,
    name: &str,
    slug: &str,
    organization_size: &str,
) -> Result<Uuid, ApiError> {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO workspaces (name, slug, owner_id, created_by_id, updated_by_id, organization_size)
         VALUES ($1, $2, $3, $3, $3, $4) RETURNING id",
    )
    .bind(name)
    .bind(slug)
    .bind(owner)
    .bind(organization_size)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO workspace_members (workspace_id, member_id, role, created_by_id) VALUES ($1, $2, 20, $2)")
        .bind(id)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    Ok(id)
}

// ---- invitations ----

const INVITE_JSON: &str = "jsonb_build_object('id', i.id, 'email', i.email, 'role', i.role, 'accepted', i.accepted,
    'responded_at', i.responded_at, 'message', i.message, 'created_at', i.created_at, 'updated_at', i.updated_at,
    'created_by', i.created_by_id, 'updated_by', i.updated_by_id, 'deleted_at', i.deleted_at,
    'workspace', jsonb_build_object('id', w.id, 'name', w.name, 'slug', w.slug, 'logo', w.logo,
        'owner', w.owner_id, 'logo_url', NULLIF(w.logo, '')))";

async fn invites_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {INVITE_JSON} FROM workspace_member_invites i JOIN workspaces w ON w.id = i.workspace_id
             WHERE i.workspace_id = $1 AND i.deleted_at IS NULL ORDER BY i.created_at"
        ))
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

/// Plane: create invites (admin). Invited role is capped at the caller's role.
// ponytail: no SMTP in Saturn, so invite emails are not sent. The invite row is stored and the
// recipient accepts it from the invitations list.
async fn invites_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let emails = b.get("emails").and_then(Value::as_array).cloned().unwrap_or_default();
    if emails.is_empty() {
        return Err(ApiError::bad("Emails are required"));
    }
    let mut rows: Vec<(String, i16)> = Vec::new();
    for e in &emails {
        let addr = s(e, "email").unwrap_or("").trim().to_lowercase();
        if !valid_email(&addr) {
            return Err(ApiError::bad(format!(
                "Invalid email - {addr} provided a valid email address is required to send the invite"
            )));
        }
        let role = e.get("role").and_then(Value::as_i64).unwrap_or(i64::from(GUEST));
        if !(0..=i64::from(ws.role)).contains(&role) {
            return Err(ApiError::bad("You cannot invite a user with higher role"));
        }
        rows.push((addr, role as i16));
    }
    let addrs: Vec<String> = rows.iter().map(|(e, _)| e.clone()).collect();
    let existing: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM workspace_members m JOIN users u ON u.id = m.member_id
         WHERE m.workspace_id = $1 AND m.is_active AND m.deleted_at IS NULL AND lower(u.email) = ANY($2)",
    )
    .bind(ws.workspace_id)
    .bind(&addrs)
    .fetch_one(&st.db)
    .await?;
    if existing.0 > 0 {
        return Err(ApiError::bad("Some users are already member of workspace"));
    }
    let mut tx = st.db.begin().await?;
    for (addr, role) in &rows {
        sqlx::query(
            "INSERT INTO workspace_member_invites (id, email, token, role, accepted, workspace_id, created_by_id)
             VALUES (gen_random_uuid(), $1, $2, $3, false, $4, $5)",
        )
        .bind(addr)
        .bind(invite_token())
        .bind(role)
        .bind(ws.workspace_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"message": "Emails sent successfully"})))
}

/// 32 random base62 chars, same alphabet as session keys.
fn invite_token() -> String {
    use rand::Rng;
    const B62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut r = rand::thread_rng();
    (0..32).map(|_| B62[r.gen_range(0..62)] as char).collect()
}

fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !e.contains(char::is_whitespace)
}

/// Public invite view: no token, matches Plane's `WorkSpaceMemberInvitePublicSerializer`.
async fn invite_public(State(st): State<AppState>, Path((slug, pk)): Path<(String, Uuid)>) -> ApiResult {
    json_row(
        sqlx::query_scalar(
            "SELECT jsonb_build_object('id', i.id, 'email', i.email, 'role', i.role, 'accepted', i.accepted,
                'responded_at', i.responded_at, 'message', i.message, 'created_at', i.created_at, 'updated_at', i.updated_at,
                'workspace', jsonb_build_object('id', w.id, 'name', w.name, 'slug', w.slug, 'logo', w.logo))
             FROM workspace_member_invites i JOIN workspaces w ON w.id = i.workspace_id
             WHERE i.id = $1 AND w.slug = $2 AND i.deleted_at IS NULL",
        )
        .bind(pk)
        .bind(slug),
        &st.db,
    )
    .await
}

/// Plane: change the role on a pending invite, capped at the caller's role.
async fn invite_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let Some(obj) = b.as_object().filter(|o| o.len() == 1 && o.contains_key("role")) else {
        return Err(ApiError::bad("Only the role of an invitation can be updated"));
    };
    let Some(role) = obj["role"].as_i64().filter(|r| (0..=i64::from(ws.role)).contains(r)) else {
        return Err(ApiError::bad("You cannot invite a user with higher role"));
    };
    json_row(
        sqlx::query_scalar(&format!(
            "WITH i AS (UPDATE workspace_member_invites SET role = $3, updated_at = now(), updated_by_id = $4
                        WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL RETURNING *)
             SELECT {INVITE_JSON} FROM i JOIN workspaces w ON w.id = i.workspace_id"
        ))
        .bind(pk)
        .bind(ws.workspace_id)
        .bind(role as i16)
        .bind(user.id),
        &st.db,
    )
    .await
}

/// Plane: delete a pending invite.
async fn invite_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let r = sqlx::query("DELETE FROM workspace_member_invites WHERE id = $1 AND workspace_id = $2")
        .bind(pk)
        .bind(ws.workspace_id)
        .execute(&st.db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

/// Plane: accept or reject an invite. The caller must be the invited email and present the token.
async fn invite_join(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let inv: (Uuid, String, String, i16, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT i.workspace_id, i.email, i.token, i.role, i.responded_at
         FROM workspace_member_invites i JOIN workspaces w ON w.id = i.workspace_id
         WHERE i.id = $1 AND w.slug = $2 AND i.deleted_at IS NULL",
    )
    .bind(pk)
    .bind(&slug)
    .fetch_one(&st.db)
    .await?;
    let token = s(&b, "token").unwrap_or("");
    if token.is_empty() || token != inv.2 {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            json!({"error": "You do not have permission to join the workspace"}),
        ));
    }
    if !user.email.eq_ignore_ascii_case(&inv.1) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            json!({"error": "You do not have permission to accept this invitation"}),
        ));
    }
    if inv.4.is_some() {
        return Err(ApiError::bad("You have already responded to the invitation request"));
    }
    let accepted = b.get("accepted").and_then(Value::as_bool).unwrap_or(false);
    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE workspace_member_invites SET accepted = $2, responded_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(pk)
    .bind(accepted)
    .execute(&mut *tx)
    .await?;
    if !accepted {
        tx.commit().await?;
        return Ok(Json(json!({"message": "Workspace Invitation was not accepted"})));
    }
    add_or_reactivate_member(&mut tx, inv.0, user.id, inv.3).await?;
    sqlx::query("UPDATE profiles SET last_workspace_id = $2, updated_at = now() WHERE user_id = $1")
        .bind(user.id)
        .bind(inv.0)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE workspace_member_invites SET deleted_at = now() WHERE id = $1")
        .bind(pk)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"message": "Workspace Invitation Accepted"})))
}

/// Reactivate an old membership row, or insert a new one. Same as Plane's `WorkspaceJoinEndpoint`.
async fn add_or_reactivate_member(
    tx: &mut sqlx::PgConnection,
    workspace_id: Uuid,
    member: Uuid,
    role: i16,
) -> Result<(), ApiError> {
    let r = sqlx::query(
        "UPDATE workspace_members SET is_active = true, role = $3, updated_at = now()
         WHERE workspace_id = $1 AND member_id = $2 AND deleted_at IS NULL",
    )
    .bind(workspace_id)
    .bind(member)
    .bind(role)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        sqlx::query(
            "INSERT INTO workspace_members (id, workspace_id, member_id, role, created_by_id, view_props, default_props, issue_props,
                is_active, explored_features, getting_started_checklist, tips)
             VALUES (gen_random_uuid(), $1, $2, $3, $2, '{}', '{}', '{}', true, '{}', '{}', '{}')",
        )
        .bind(workspace_id)
        .bind(member)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Plane: pending invites addressed to the caller's email.
async fn my_invites_list(State(st): State<AppState>, user: CurrentUser) -> ApiResult {
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {INVITE_JSON} FROM workspace_member_invites i JOIN workspaces w ON w.id = i.workspace_id
             WHERE lower(i.email) = lower($1) AND i.deleted_at IS NULL AND i.responded_at IS NULL ORDER BY i.created_at"
        ))
        .bind(&user.email),
        &st.db,
    )
    .await
}

/// Plane: accept the listed invites addressed to the caller's own email (no token, self-service).
async fn my_invites_accept(
    State(st): State<AppState>,
    user: CurrentUser,
    Json(b): Json<Value>,
) -> ApiResult<StatusCode> {
    let ids: Vec<Uuid> = b
        .get("invitations")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().and_then(|x| Uuid::parse_str(x).ok()))
                .collect()
        })
        .unwrap_or_default();
    let mut tx = st.db.begin().await?;
    let invites: Vec<(Uuid, i16)> = sqlx::query_as(
        "SELECT workspace_id, role FROM workspace_member_invites
         WHERE id = ANY($1) AND lower(email) = lower($2) AND deleted_at IS NULL ORDER BY created_at DESC",
    )
    .bind(&ids)
    .bind(&user.email)
    .fetch_all(&mut *tx)
    .await?;
    for (ws_id, role) in &invites {
        add_or_reactivate_member(&mut tx, *ws_id, user.id, *role).await?;
    }
    sqlx::query(
        "UPDATE workspace_member_invites SET deleted_at = now() WHERE id = ANY($1) AND lower(email) = lower($2)",
    )
    .bind(&ids)
    .bind(&user.email)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    no_content()
}

// ---- favorites ----

const FAV_JSON: &str =
    "jsonb_build_object('id', f.id, 'entity_type', f.entity_type, 'entity_identifier', f.entity_identifier,
    'name', f.name, 'is_folder', f.is_folder, 'sequence', f.sequence, 'parent', f.parent_id,
    'workspace_id', f.workspace_id, 'project_id', f.project_id)";

async fn favorites_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {FAV_JSON} FROM user_favorites f
             WHERE f.user_id = $1 AND f.workspace_id = $2 AND f.parent_id IS NULL AND f.deleted_at IS NULL
               AND (f.project_id IS NULL OR EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = f.project_id
                    AND pm.member_id = $1 AND pm.is_active AND pm.deleted_at IS NULL))
             ORDER BY f.sequence"
        ))
        .bind(user.id)
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

/// Entity tables a favorite can point at. Fixed whitelist, never request input.
fn favorite_table(entity_type: &str) -> Option<&'static str> {
    match entity_type {
        "project" => Some("projects"),
        "cycle" => Some("cycles"),
        "module" => Some("modules"),
        "view" => Some("issue_views"),
        "page" => Some("pages"),
        "issue" => Some("issues"),
        _ => None,
    }
}

/// Plane: add a favorite. Idempotent on (entity_type, entity_identifier). The entity must be in this workspace.
async fn favorites_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let entity_type = s(&b, "entity_type").unwrap_or("");
    if entity_type.is_empty() {
        return Err(ApiError::bad_body(json!({"entity_type": ["This field is required."]})));
    }
    let entity_id = uuid_of(&b, "entity_identifier");
    let project_id = uuid_of(&b, "project_id");
    if let Some(eid) = entity_id {
        let existing: Option<Value> = sqlx::query_scalar(&format!(
            "SELECT {FAV_JSON} FROM user_favorites f WHERE f.workspace_id = $1 AND f.user_id = $2
             AND f.entity_type = $3 AND f.entity_identifier = $4 AND f.deleted_at IS NULL LIMIT 1"
        ))
        .bind(ws.workspace_id)
        .bind(user.id)
        .bind(entity_type)
        .bind(eid)
        .fetch_optional(&st.db)
        .await?;
        if let Some(v) = existing {
            return Ok((StatusCode::OK, Json(v)));
        }
        let Some(table) = favorite_table(entity_type) else {
            return Err(ApiError::bad(format!("\"{entity_type}\" is not a valid favorite type")));
        };
        // Projects: caller must be a member. Other entities: caller must be a member of the project.
        let visible: (bool,) = if table == "projects" {
            sqlx::query_as(
                "SELECT EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = $1 AND pm.workspace_id = $2
                 AND pm.member_id = $3 AND pm.is_active AND pm.deleted_at IS NULL)",
            )
            .bind(eid)
            .bind(ws.workspace_id)
            .bind(user.id)
            .fetch_one(&st.db)
            .await?
        } else {
            let Some(pid) = project_id else {
                return Err(ApiError::bad("project_id is required for this entity"));
            };
            sqlx::query_as(&format!(
                "SELECT EXISTS (SELECT 1 FROM {table} e JOIN project_members pm ON pm.project_id = $2
                 AND pm.member_id = $3 AND pm.is_active AND pm.deleted_at IS NULL
                 WHERE e.id = $1 AND e.workspace_id = $4 AND e.deleted_at IS NULL)"
            ))
            .bind(eid)
            .bind(pid)
            .bind(user.id)
            .bind(ws.workspace_id)
            .fetch_one(&st.db)
            .await?
        };
        if !visible.0 {
            return Err(ApiError::not_found());
        }
    }
    let v: Value = sqlx::query_scalar(&format!(
        "INSERT INTO user_favorites AS f (id, entity_type, entity_identifier, name, is_folder, sequence, user_id, workspace_id,
            project_id, parent_id, created_by_id, updated_by_id)
         VALUES (gen_random_uuid(), $1, $2, $3, $4,
            COALESCE((SELECT max(sequence) + 10000 FROM user_favorites WHERE user_id = $5 AND workspace_id = $6 AND deleted_at IS NULL), 65535),
            $5, $6, $7, $8, $5, $5)
         RETURNING {FAV_JSON}"
    ))
    .bind(entity_type)
    .bind(entity_id)
    .bind(s(&b, "name"))
    .bind(b.get("is_folder").and_then(Value::as_bool).unwrap_or(false))
    .bind(user.id)
    .bind(ws.workspace_id)
    .bind(project_id)
    .bind(uuid_of(&b, "parent"))
    .fetch_one(&st.db)
    .await?;
    created(v)
}

/// Plane: rename, reorder, or move a favorite into a folder. The favorited entity stays fixed.
async fn favorites_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let mut sets = Vec::new();
    if b.get("name").is_some() {
        sets.push("name = ($2::jsonb ->> 'name')");
    }
    if b.get("sequence").is_some() {
        sets.push("sequence = ($2::jsonb ->> 'sequence')::float8");
    }
    if b.get("parent").is_some() {
        sets.push("parent_id = ($2::jsonb ->> 'parent')::uuid");
    }
    if sets.is_empty() {
        return json_row(
            sqlx::query_scalar(&format!(
                "SELECT {FAV_JSON} FROM user_favorites f WHERE f.id = $1 AND f.user_id = $3 AND f.workspace_id = $4 AND f.deleted_at IS NULL"
            ))
            .bind(pk)
            .bind(&b)
            .bind(user.id)
            .bind(ws.workspace_id),
            &st.db,
        )
        .await;
    }
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE user_favorites f SET {}, updated_at = now(), updated_by_id = $3
             WHERE f.id = $1 AND f.user_id = $3 AND f.workspace_id = $4 AND f.deleted_at IS NULL
             RETURNING {FAV_JSON}",
            sets.join(", ")
        ))
        .bind(pk)
        .bind(&b)
        .bind(user.id)
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

/// Plane: remove a favorite. Soft delete here.
async fn favorites_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let r = sqlx::query(
        "UPDATE user_favorites SET deleted_at = now(), updated_at = now()
         WHERE id = $1 AND user_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(user.id)
    .bind(ws.workspace_id)
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

/// Plane: the children of a favorite folder.
async fn favorite_group(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {FAV_JSON} FROM user_favorites f
             WHERE f.parent_id = $1 AND f.user_id = $2 AND f.workspace_id = $3 AND f.deleted_at IS NULL ORDER BY f.sequence"
        ))
        .bind(pk)
        .bind(user.id)
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

// ---- user profile (step 5): stats, profile page, activity ----

/// Plane `_visible_issues_q`, written for alias `i` on issues: caller must be an active project member,
/// and guests see only issues they created unless the project enables guest_view_all_features.
const VISIBLE_ISSUE: &str = "EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = i.project_id AND pm.member_id = $1
    AND pm.is_active AND pm.deleted_at IS NULL
    AND (pm.role > 5 OR EXISTS (SELECT 1 FROM projects p WHERE p.id = i.project_id AND p.guest_view_all_features) OR i.created_by_id = $1))";

/// Plane: issue counts for `user_id` as seen by the caller (WorkspaceUserProfileStatsEndpoint, core counters).
async fn user_stats(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, target)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(&format!(
        "SELECT
            count(*) FILTER (WHERE i.created_by_id = $2),
            count(*) FILTER (WHERE EXISTS (SELECT 1 FROM issue_assignees a WHERE a.issue_id = i.id AND a.assignee_id = $2)),
            count(*) FILTER (WHERE i.completed_at IS NOT NULL AND EXISTS (SELECT 1 FROM issue_assignees a WHERE a.issue_id = i.id AND a.assignee_id = $2)),
            count(*) FILTER (WHERE s.\"group\" IN ('backlog','unstarted','started') AND EXISTS (SELECT 1 FROM issue_assignees a WHERE a.issue_id = i.id AND a.assignee_id = $2)),
            count(*) FILTER (WHERE EXISTS (SELECT 1 FROM issue_subscribers sb WHERE sb.issue_id = i.id AND sb.subscriber_id = $2))
         FROM issues i LEFT JOIN states s ON s.id = i.state_id
         WHERE i.workspace_id = $3 AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft AND {VISIBLE_ISSUE}"
    ))
    .bind(user.id)
    .bind(target)
    .bind(ws.workspace_id)
    .fetch_one(&st.db)
    .await?;
    Ok(Json(json!({
        "created_issues": counts.0,
        "assigned_issues": counts.1,
        "completed_issues": counts.2,
        "pending_issues": counts.3,
        "subscribed_issues": counts.4,
    })))
}

/// Plane: profile page data for a workspace member. Requires the target to be an active member.
async fn user_profile(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, target)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let row: Option<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('email', u.email, 'first_name', u.first_name, 'last_name', u.last_name,
            'avatar_url', NULLIF(u.avatar, ''), 'cover_image_url', u.cover_image, 'date_joined', u.date_joined)
         FROM workspace_members m JOIN users u ON u.id = m.member_id
         WHERE m.workspace_id = $1 AND m.member_id = $2 AND m.is_active AND m.deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .bind(target)
    .fetch_optional(&st.db)
    .await?;
    let Some(user_data) = row else {
        return Err(ApiError::not_found());
    };
    Ok(Json(json!({"project_data": [], "user_data": user_data})))
}

/// Plane: activity feed. Returned empty until the activity writer lands.
// ponytail: issue_activities is not read here yet. Empty page is valid for the UI.
async fn user_activity(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, _target)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;
    Ok(Json(json!({"results": [], "count": 0, "next": null, "previous": null})))
}

// --- sidebar and home preferences (WorkspaceUserPreference, WorkspaceHomePreference) ---

const SIDEBAR_KEYS: &[&str] = &[
    "views",
    "active_cycles",
    "analytics",
    "drafts",
    "your_work",
    "archives",
    "stickies",
];
const SIDEBAR_PINNED: &[&str] = &["drafts", "your_work", "stickies"];
/// Plane creates every home widget except these two by default.
const HOME_KEYS: &[&str] = &["quick_links", "recents", "my_stickies"];

/// Insert the default sidebar rows this user does not have yet (Plane does this on GET).
async fn ensure_sidebar_prefs(st: &AppState, ws: &WorkspaceCtx) -> Result<(), ApiError> {
    for (i, key) in SIDEBAR_KEYS.iter().enumerate() {
        sqlx::query(
            "INSERT INTO workspace_user_preferences (key, is_pinned, sort_order, user_id, workspace_id, created_by_id, updated_by_id)
             SELECT $1, $2, $3, $4, $5, $4, $4
             WHERE NOT EXISTS (SELECT 1 FROM workspace_user_preferences
                               WHERE workspace_id = $5 AND user_id = $4 AND key = $1 AND deleted_at IS NULL)",
        )
        .bind(key)
        .bind(SIDEBAR_PINNED.contains(key))
        .bind(65535.0 + i as f64 * 10000.0)
        .bind(ws.user_id)
        .bind(ws.workspace_id)
        .execute(&st.db)
        .await?;
    }
    Ok(())
}

/// GET: `{key: {is_pinned, sort_order}}`, ordered by sort_order.
async fn sidebar_prefs_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ensure_sidebar_prefs(&st, &ws).await?;
    let rows: Vec<(String, bool, f64)> = sqlx::query_as(
        "SELECT key, is_pinned, sort_order FROM workspace_user_preferences
         WHERE workspace_id = $1 AND user_id = $2 AND deleted_at IS NULL ORDER BY sort_order",
    )
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;
    let mut out = serde_json::Map::new();
    for (key, is_pinned, sort_order) in rows {
        out.insert(key, json!({"is_pinned": is_pinned, "sort_order": sort_order}));
    }
    Ok(Json(Value::Object(out)))
}

async fn update_sidebar_pref(
    st: &AppState,
    ws: &WorkspaceCtx,
    key: &str,
    b: &Value,
) -> Result<Option<Value>, ApiError> {
    Ok(sqlx::query_scalar(
        "UPDATE workspace_user_preferences SET is_pinned = COALESCE($4, is_pinned), sort_order = COALESCE($5, sort_order),
                updated_at = now(), updated_by_id = $2
         WHERE workspace_id = $1 AND user_id = $2 AND key = $3 AND deleted_at IS NULL
         RETURNING jsonb_build_object('key', key, 'is_pinned', is_pinned, 'sort_order', sort_order)",
    )
    .bind(ws.workspace_id)
    .bind(ws.user_id)
    .bind(key)
    .bind(b.get("is_pinned").and_then(Value::as_bool))
    .bind(b.get("sort_order").and_then(Value::as_f64))
    .fetch_optional(&st.db)
    .await?)
}

/// PATCH with a list of `{key, is_pinned?, sort_order?}`; unknown keys are skipped, as in Plane.
async fn sidebar_prefs_bulk(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    for item in b.as_array().cloned().unwrap_or_default() {
        if let Some(key) = s(&item, "key") {
            update_sidebar_pref(&st, &ws, key, &item).await?;
        }
    }
    Ok(Json(json!({"message": "Successfully updated"})))
}

/// PATCH one key. Plane's web app calls this path although Plane only routes the bulk form.
async fn sidebar_pref_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, key)): Path<(String, String)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ensure_sidebar_prefs(&st, &ws).await?;
    update_sidebar_pref(&st, &ws, &key, &b)
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

/// GET: list of `{key, is_enabled, config, sort_order}`, creating defaults first.
async fn home_prefs_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    for (i, key) in HOME_KEYS.iter().enumerate() {
        sqlx::query(
            "INSERT INTO workspace_home_preferences (key, is_enabled, config, sort_order, user_id, workspace_id,
                                                    created_by_id, updated_by_id)
             SELECT $1, true, '{}'::jsonb, $2, $3, $4, $3, $3
             WHERE NOT EXISTS (SELECT 1 FROM workspace_home_preferences
                               WHERE workspace_id = $4 AND user_id = $3 AND key = $1 AND deleted_at IS NULL)",
        )
        .bind(key)
        .bind(1000.0 - (i as f64 + 1.0))
        .bind(user.id)
        .bind(ws.workspace_id)
        .execute(&st.db)
        .await?;
    }
    json_rows(
        sqlx::query_scalar(
            "SELECT jsonb_build_object('key', key, 'is_enabled', is_enabled, 'config', config, 'sort_order', sort_order)
             FROM workspace_home_preferences WHERE workspace_id = $1 AND user_id = $2 AND deleted_at IS NULL",
        )
        .bind(ws.workspace_id)
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn home_pref_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, key)): Path<(String, String)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    let v: Option<Value> = sqlx::query_scalar(
        "UPDATE workspace_home_preferences SET is_enabled = COALESCE($4, is_enabled), sort_order = COALESCE($5, sort_order),
                config = COALESCE($6, config), updated_at = now(), updated_by_id = $2
         WHERE workspace_id = $1 AND user_id = $2 AND key = $3 AND deleted_at IS NULL
         RETURNING jsonb_build_object('key', key, 'is_enabled', is_enabled, 'sort_order', sort_order)",
    )
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(&key)
    .bind(b.get("is_enabled").and_then(Value::as_bool))
    .bind(b.get("sort_order").and_then(Value::as_f64))
    .bind(b.get("config"))
    .fetch_optional(&st.db)
    .await?;
    v.map(Json)
        .ok_or_else(|| ApiError::bad_body(json!({"detail": "Preference not found"})))
}

// --- webhooks (WebhookEndpoint, WebhookSecretRegenerateEndpoint, WebhookLogsEndpoint). Workspace ADMIN only ---

/// WebhookSerializer fields. `secret_key` is added only on create and regenerate.
const WEBHOOK_JSON: &str = "jsonb_build_object('id', w.id, 'url', w.url, 'is_active', w.is_active, 'project', w.project,
    'issue', w.issue, 'module', w.module, 'cycle', w.cycle, 'issue_comment', w.issue_comment, 'is_internal', w.is_internal,
    'version', w.version, 'created_at', w.created_at, 'updated_at', w.updated_at, 'workspace', w.workspace_id,
    'created_by', w.created_by_id, 'updated_by', w.updated_by_id)";
const WEBHOOK_FLAGS: &[&str] = &["is_active", "project", "issue", "module", "cycle", "issue_comment"];
const WEBHOOK_PATCH_FIELDS: &[(&str, &str)] = &[
    ("url", "text"),
    ("is_active", "bool"),
    ("project", "bool"),
    ("issue", "bool"),
    ("module", "bool"),
    ("cycle", "bool"),
    ("issue_comment", "bool"),
];

/// Plane: `"plane_wh_" + uuid4().hex`.
fn new_webhook_secret() -> String {
    format!("plane_wh_{}", Uuid::new_v4().simple())
}

fn flag(b: &Value, k: &str) -> Option<bool> {
    b.get(k).and_then(Value::as_bool)
}

fn check_flags(b: &Value) -> Result<(), ApiError> {
    let mut errs = serde_json::Map::new();
    for k in WEBHOOK_FLAGS {
        if let Some(v) = b.get(*k)
            && !v.is_boolean()
        {
            errs.insert((*k).into(), json!(["Must be a valid boolean."]));
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(ApiError::bad_body(Value::Object(errs)))
    }
}

/// Stand-in for Plane's `validate_url` (SSRF guard): http/https only, and the host must resolve
/// to public addresses. Loopback, private, link-local and unspecified targets are refused.
// ponytail: DNS is resolved once here; a rebinding race before delivery is not covered. Add when delivery is ported.
async fn check_webhook_url(raw: &str) -> Result<(), ApiError> {
    let bad = || ApiError::bad_body(json!({"url": ["Invalid or disallowed webhook URL."]}));
    let u = url::Url::parse(raw).map_err(|_| bad())?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(bad());
    }
    let port = u.port_or_known_default().unwrap_or(443);
    let ips: Vec<std::net::IpAddr> = match u.host().ok_or_else(bad)? {
        url::Host::Ipv4(ip) => vec![ip.into()],
        url::Host::Ipv6(ip) => vec![ip.into()],
        url::Host::Domain(d) => tokio::net::lookup_host((d, port))
            .await
            .map_err(|_| bad())?
            .map(|a| a.ip())
            .collect(),
    };
    if ips.is_empty() || ips.iter().any(|ip| blocked_ip(*ip)) {
        return Err(bad());
    }
    Ok(())
}

fn blocked_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.octets()[0] == 0
        }
        std::net::IpAddr::V6(v6) => {
            let seg = v6.segments()[0];
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || (seg & 0xfe00) == 0xfc00
                || (seg & 0xffc0) == 0xfe80
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| blocked_ip(std::net::IpAddr::V4(v4)))
        }
    }
}

async fn webhooks_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {WEBHOOK_JSON} FROM webhooks w WHERE w.workspace_id = $1 AND w.deleted_at IS NULL ORDER BY w.created_at"
        ))
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

async fn webhook_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let url = s(&b, "url").ok_or_else(|| ApiError::bad_body(json!({"url": ["This field is required."]})))?;
    check_webhook_url(url).await?;
    check_flags(&b)?;
    let secret = new_webhook_secret();
    let mut v: Value = sqlx::query_scalar(&format!(
        "INSERT INTO webhooks AS w (workspace_id, url, secret_key, is_active, project, issue, module, cycle, issue_comment,
            created_by_id, updated_by_id)
         VALUES ($1, $2, $3, COALESCE($4, true), COALESCE($5, false), COALESCE($6, false), COALESCE($7, false),
                 COALESCE($8, false), COALESCE($9, false), $10, $10)
         RETURNING {WEBHOOK_JSON}"
    ))
    .bind(ws.workspace_id)
    .bind(url)
    .bind(&secret)
    .bind(flag(&b, "is_active"))
    .bind(flag(&b, "project"))
    .bind(flag(&b, "issue"))
    .bind(flag(&b, "module"))
    .bind(flag(&b, "cycle"))
    .bind(flag(&b, "issue_comment"))
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    v["secret_key"] = json!(secret);
    created(v)
}

async fn webhook_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {WEBHOOK_JSON} FROM webhooks w WHERE w.id = $1 AND w.workspace_id = $2 AND w.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

async fn webhook_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    if b.get("url").is_some() {
        let url = s(&b, "url").ok_or_else(|| ApiError::bad_body(json!({"url": ["This field may not be null."]})))?;
        check_webhook_url(url).await?;
    }
    check_flags(&b)?;
    let mut sets = crate::users::patch_sets(&b, WEBHOOK_PATCH_FIELDS, 2);
    sets.push("updated_at = now()".into());
    sets.push("updated_by_id = $4".into());
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE webhooks AS w SET {} WHERE w.id = $1 AND w.workspace_id = $3 AND w.deleted_at IS NULL
             RETURNING {WEBHOOK_JSON}",
            sets.join(", ")
        ))
        .bind(pk)
        .bind(&b)
        .bind(ws.workspace_id)
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn webhook_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let r = sqlx::query(
        "UPDATE webhooks SET deleted_at = now(), updated_at = now(), updated_by_id = $3
         WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
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

async fn webhook_regenerate(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    let secret = new_webhook_secret();
    let v: Option<Value> = sqlx::query_scalar(&format!(
        "UPDATE webhooks AS w SET secret_key = $3, updated_at = now(), updated_by_id = $4
         WHERE w.id = $1 AND w.workspace_id = $2 AND w.deleted_at IS NULL RETURNING {WEBHOOK_JSON}"
    ))
    .bind(pk)
    .bind(ws.workspace_id)
    .bind(&secret)
    .bind(user.id)
    .fetch_optional(&st.db)
    .await?;
    let mut v = v.ok_or_else(ApiError::not_found)?;
    v["secret_key"] = json!(secret);
    Ok(Json(v))
}

async fn webhook_logs(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, webhook_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN])?;
    // ponytail: delivery not ported, so this table stays empty until webhook delivery exists.
    json_rows(
        sqlx::query_scalar(
            "SELECT jsonb_build_object('id', l.id, 'created_at', l.created_at, 'updated_at', l.updated_at,
                'event_type', l.event_type, 'request_method', l.request_method, 'request_headers', l.request_headers,
                'request_body', l.request_body, 'response_status', l.response_status, 'response_headers', l.response_headers,
                'response_body', l.response_body, 'retry_count', l.retry_count, 'created_by', l.created_by_id,
                'updated_by', l.updated_by_id, 'webhook', l.webhook, 'workspace', l.workspace_id)
             FROM webhook_logs l
             WHERE l.workspace_id = $1 AND l.webhook = $2 AND l.deleted_at IS NULL ORDER BY l.created_at DESC",
        )
        .bind(ws.workspace_id)
        .bind(webhook_id),
        &st.db,
    )
    .await
}

// --- workspace themes (WorkspaceThemeViewSet). Plane's WorkSpaceAdminPermission admits Admin and Member ---

const THEME_JSON: &str = "jsonb_build_object('id', t.id, 'name', t.name, 'colors', t.colors, 'actor', t.actor_id,
    'workspace', t.workspace_id, 'created_at', t.created_at, 'updated_at', t.updated_at,
    'created_by', t.created_by_id, 'updated_by', t.updated_by_id)";

/// Plane's `name` is varchar(300) and required.
fn theme_name(b: &Value, required: bool) -> Result<Option<&str>, ApiError> {
    match s(b, "name").map(str::trim) {
        Some(n) if n.is_empty() => Err(ApiError::bad_body(json!({"name": ["This field may not be blank."]}))),
        Some(n) if n.chars().count() > 300 => Err(ApiError::bad_body(
            json!({"name": ["Ensure this field has no more than 300 characters."]}),
        )),
        Some(n) => Ok(Some(n)),
        None if required => Err(ApiError::bad_body(json!({"name": ["This field is required."]}))),
        None => Ok(None),
    }
}

async fn themes_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {THEME_JSON} FROM workspace_themes t WHERE t.workspace_id = $1 AND t.deleted_at IS NULL ORDER BY t.created_at"
        ))
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

async fn theme_create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let name = theme_name(&b, true)?.unwrap_or_default();
    let v: Value = sqlx::query_scalar(&format!(
        "INSERT INTO workspace_themes AS t (workspace_id, name, colors, actor_id, created_by_id, updated_by_id)
         VALUES ($1, $2, COALESCE($3::jsonb, '{{}}'::jsonb), $4, $4, $4) RETURNING {THEME_JSON}"
    ))
    .bind(ws.workspace_id)
    .bind(name)
    .bind(b.get("colors"))
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    created(v)
}

async fn theme_retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {THEME_JSON} FROM workspace_themes t WHERE t.id = $1 AND t.workspace_id = $2 AND t.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

async fn theme_update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let name = theme_name(&b, false)?;
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE workspace_themes AS t SET name = COALESCE($3, t.name), colors = COALESCE($4::jsonb, t.colors),
                updated_at = now(), updated_by_id = $2
             WHERE t.id = $1 AND t.workspace_id = $5 AND t.deleted_at IS NULL RETURNING {THEME_JSON}"
        ))
        .bind(pk)
        .bind(user.id)
        .bind(name)
        .bind(b.get("colors"))
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}

async fn theme_delete(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, pk)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;
    let r = sqlx::query(
        "UPDATE workspace_themes SET deleted_at = now(), updated_at = now(), updated_by_id = $3
         WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
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
