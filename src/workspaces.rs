//! Workspaces, members, invitations, favorites, workspace-level user data.
use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
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
