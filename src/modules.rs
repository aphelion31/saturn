//! Modules: CRUD, issue membership, links, favorites, archive and per-user view properties.
//! Port of Plane's `app/urls/module.py` and `app/views/module/{base,issue,archive}.py`.
//! Skipped (ponytail): burndown charts (`completion_chart` is `{}`), distribution rows (empty arrays),
//! activity fan-out and webhooks.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

use crate::common::*;
use crate::workitems::{self, ACTIVE, Arg, Base, Scope};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/workspaces/{slug}/modules/", get(workspace_list))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/modules/",
            get(list).post(create),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/modules/{pk}/",
            get(retrieve).put(update).patch(update).delete(destroy),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/modules/",
            post(add_issue_modules),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/modules/{module_id}/issues/",
            get(list_issues).post(add_module_issues),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/modules/{module_id}/issues/{issue_id}/",
            get(get_issue).delete(remove_issue),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/modules/{module_id}/module-links/",
            get(list_links).post(create_link),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/modules/{module_id}/module-links/{pk}/",
            get(get_link).put(update_link).patch(update_link).delete(destroy_link),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/user-favorite-modules/",
            get(list_favorites).post(add_favorite),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/user-favorite-modules/{module_id}/",
            axum::routing::delete(remove_favorite),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/modules/{module_id}/user-properties/",
            get(get_user_properties).patch(patch_user_properties),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/modules/{module_id}/archive/",
            post(archive).delete(unarchive),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/archived-modules/",
            get(list_archived),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/archived-modules/{pk}/",
            get(get_archived).delete(unarchive),
        )
}

const MODULE_STATUSES: &[&str] = &["backlog", "planned", "in-progress", "paused", "completed", "cancelled"];
const MODULE_FIELDS: &[(&str, &str)] = &[
    ("name", "text"),
    ("description", "text"),
    ("description_text", "jsonb"),
    ("description_html", "jsonb"),
    ("start_date", "date"),
    ("target_date", "date"),
    ("status", "text"),
    ("lead_id", "uuid"),
    ("view_props", "jsonb"),
    ("sort_order", "float8"),
    ("logo_props", "jsonb"),
];

/// ModuleSerializer fields plus computed counts (`*_issues`, `*_estimate_points`). `fav_expr` is the
/// `is_favorite` boolean SQL. Counts come from the `COUNTS` lateral join, so the caller must join it as `c`.
fn module_json(fav_expr: &str) -> String {
    format!(
        "jsonb_build_object(
        'id', m.id, 'workspace_id', m.workspace_id, 'project_id', m.project_id, 'name', m.name,
        'description', m.description, 'description_text', m.description_text, 'description_html', m.description_html,
        'start_date', m.start_date, 'target_date', m.target_date, 'status', m.status, 'lead_id', m.lead_id,
        'member_ids', COALESCE((SELECT jsonb_agg(mm.member_id) FROM module_members mm
                                WHERE mm.module_id = m.id AND mm.deleted_at IS NULL), '[]'::jsonb),
        'view_props', m.view_props, 'sort_order', m.sort_order, 'external_source', m.external_source,
        'external_id', m.external_id, 'logo_props', m.logo_props,
        'total_estimate_points', c.total_ep, 'completed_estimate_points', c.completed_ep,
        'is_favorite', {fav_expr},
        'total_issues', c.total_issues, 'cancelled_issues', c.cancelled_issues, 'completed_issues', c.completed_issues,
        'started_issues', c.started_issues, 'unstarted_issues', c.unstarted_issues, 'backlog_issues', c.backlog_issues,
        'created_at', m.created_at, 'updated_at', m.updated_at, 'archived_at', m.archived_at)"
    )
}

/// Per-module issue counts and estimate sums over active work items (Plane's `issue_objects`).
/// Joins as `LEFT JOIN LATERAL (...) c ON true`, one aggregate pass per module.
const COUNTS: &str = "LEFT JOIN LATERAL (
    SELECT count(*) AS total_issues,
           count(*) FILTER (WHERE s.group = 'cancelled') AS cancelled_issues,
           count(*) FILTER (WHERE s.group = 'completed') AS completed_issues,
           count(*) FILTER (WHERE s.group = 'started') AS started_issues,
           count(*) FILTER (WHERE s.group = 'unstarted') AS unstarted_issues,
           count(*) FILTER (WHERE s.group = 'backlog') AS backlog_issues,
           COALESCE(sum(ep.value::float8) FILTER (WHERE ep.id IS NOT NULL), 0)::float8 AS total_ep,
           COALESCE(sum(ep.value::float8) FILTER (WHERE s.group = 'completed'), 0)::float8 AS completed_ep
    FROM module_issues mi
    JOIN issues i ON i.id = mi.issue_id AND i.workspace_id = m.workspace_id AND i.project_id = m.project_id AND i.deleted_at IS NULL AND i.archived_at IS NULL AND NOT i.is_draft
    LEFT JOIN states s ON s.id = i.state_id AND s.workspace_id = m.workspace_id AND s.project_id = m.project_id AND s.deleted_at IS NULL
    LEFT JOIN estimate_points ep ON ep.id = i.estimate_point_id AND ep.workspace_id = m.workspace_id AND ep.project_id = m.project_id AND ep.deleted_at IS NULL
    WHERE mi.module_id = m.id AND mi.workspace_id = m.workspace_id AND mi.project_id = m.project_id AND mi.deleted_at IS NULL
) c ON true";

/// Favorite flag for `user` on module `m` (user_favorites rows are soft-deleted in Plane).
fn fav(user_param: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM user_favorites uf WHERE uf.entity_type = 'module' AND uf.entity_identifier = m.id
                 AND uf.user_id = {user_param} AND uf.deleted_at IS NULL)"
    )
}

fn module_select(user_param: &str) -> String {
    format!("SELECT {} FROM modules m {COUNTS}", module_json(&fav(user_param)))
}

/// 404 unless the module exists in this workspace and project and is not soft-deleted.
async fn module_in_project(db: &sqlx::PgPool, workspace_id: Uuid, project_id: Uuid, pk: Uuid) -> Result<(), ApiError> {
    let (ok,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM modules WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL)",
    )
    .bind(pk)
    .bind(project_id)
    .bind(workspace_id)
    .fetch_one(db)
    .await?;
    if ok { Ok(()) } else { Err(module_not_found()) }
}

fn module_not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, json!({"error": "Module not found"}))
}

/// Insert module members for `members`, ignoring duplicates (Plane's `ignore_conflicts=True`).
async fn insert_members(
    tx: &mut sqlx::PgConnection,
    module_id: Uuid,
    project_id: Uuid,
    workspace_id: Uuid,
    members: &[Uuid],
    user_id: Uuid,
) -> Result<(), ApiError> {
    for m in members {
        sqlx::query(
            "INSERT INTO module_members (module_id, member_id, project_id, workspace_id, created_by_id, updated_by_id)
             VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT DO NOTHING",
        )
        .bind(module_id)
        .bind(m)
        .bind(project_id)
        .bind(workspace_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// `member_ids` as UUIDs from a JSON body. Non-UUID entries are a 400 (DRF `UUIDField`).
fn member_list(b: &Value) -> Result<Option<Vec<Uuid>>, ApiError> {
    let Some(v) = b.get("member_ids") else { return Ok(None) };
    if v.is_null() {
        return Ok(None);
    }
    let arr = v
        .as_array()
        .ok_or_else(|| ApiError::bad_body(json!({"member_ids": ["Expected a list of items."]})))?;
    arr.iter()
        .map(|x| {
            x.as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| ApiError::bad_body(json!({"member_ids": ["Must be a valid UUID."]})))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

async fn list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {} FROM modules m {COUNTS}
             WHERE m.project_id = $2 AND m.workspace_id = $3 AND m.archived_at IS NULL AND m.deleted_at IS NULL
             ORDER BY {} DESC, m.created_at DESC",
            module_json(&fav("$1")),
            fav("$1"),
        ))
        .bind(user.id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let name = s(&b, "name").map(str::trim).filter(|n| !n.is_empty());
    let Some(name) = name else {
        return Err(ApiError::bad_body(json!({"name": ["This field is required."]})));
    };
    let status = s(&b, "status").unwrap_or("planned");
    if !MODULE_STATUSES.contains(&status) {
        return Err(ApiError::bad_body(
            json!({"status": [format!("\"{status}\" is not a valid choice.")]}),
        ));
    }
    let start = s(&b, "start_date");
    let target = s(&b, "target_date");
    if let (Some(a), Some(z)) = (start, target)
        && a > z
    {
        return Err(ApiError::bad_body(
            json!({"non_field_errors": ["Start date cannot exceed target date"]}),
        ));
    }
    let (taken,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM modules WHERE name = $1 AND project_id = $2 AND deleted_at IS NULL)",
    )
    .bind(name)
    .bind(project_id)
    .fetch_one(&st.db)
    .await?;
    if taken {
        return Err(ApiError::bad_body(
            json!({"error": "Module with this name already exists"}),
        ));
    }
    let members = member_list(&b)?;
    let (desc_html, desc_text) = match s(&b, "description_html") {
        Some(h) => {
            let clean = sanitize_html(h)?;
            (Some(clean.clone()), Some(strip_tags(&clean)))
        }
        None => (None, None),
    };
    let mut tx = st.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO modules (name, description, description_html, description_text, start_date, target_date, status,
                              lead_id, view_props, sort_order, logo_props, project_id, workspace_id, created_by_id, updated_by_id)
         VALUES ($1, COALESCE($2, ''), $3::jsonb, $4::jsonb, $5::date, $6::date, $7, $8, COALESCE($9::jsonb, '{}'::jsonb),
                 COALESCE($10::float8, 65535), COALESCE($11::jsonb, '{}'::jsonb), $12, $13, $14, $14)
         RETURNING id",
    )
    .bind(name)
    .bind(s(&b, "description"))
    .bind(desc_html.map(|h| json!(h)))
    .bind(desc_text.map(|t| json!(t)))
    .bind(start)
    .bind(target)
    .bind(status)
    .bind(uuid_of(&b, "lead_id"))
    .bind(b.get("view_props"))
    .bind(b.get("sort_order").and_then(Value::as_f64))
    .bind(b.get("logo_props"))
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(m) = members {
        insert_members(&mut tx, id, project_id, p.workspace_id(), &m, user.id).await?;
    }
    tx.commit().await?;
    let v: Value = sqlx::query_scalar(&format!("{} WHERE m.id = $2", module_select("$1")))
        .bind(user.id)
        .bind(id)
        .fetch_one(&st.db)
        .await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "module", "created", v.clone());
    created(v)
}

async fn retrieve(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let mut v: Value = sqlx::query_scalar(&format!(
        "{} WHERE m.id = $2 AND m.project_id = $3 AND m.workspace_id = $4 AND m.archived_at IS NULL AND m.deleted_at IS NULL",
        module_select("$1")
    ))
    .bind(user.id)
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(module_not_found)?;
    // ModuleDetailSerializer extras: links and sub-issue counts. Distribution is empty (ponytail).
    let links: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {LINK_JSON} FROM module_links l WHERE l.module_id = $1 AND l.deleted_at IS NULL ORDER BY l.created_at DESC"
    ))
    .bind(pk)
    .fetch_all(&st.db)
    .await?;
    let sub: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM issues i JOIN module_issues mi ON mi.issue_id = i.id AND mi.deleted_at IS NULL
         WHERE mi.module_id = $1 AND i.parent_id IS NOT NULL AND i.deleted_at IS NULL",
    )
    .bind(pk)
    .fetch_one(&st.db)
    .await?;
    let obj = v.as_object_mut().ok_or_else(ApiError::not_found)?;
    obj.insert("link_module".into(), Value::Array(links));
    obj.insert("sub_issues".into(), json!(sub.0));
    obj.insert(
        "distribution".into(),
        json!({"assignees": [], "labels": [], "completion_chart": {}}),
    );
    obj.insert("estimate_distribution".into(), json!({}));
    Ok(Json(v))
}

async fn update(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let (archived,): (bool,) = sqlx::query_as(
        "SELECT archived_at IS NOT NULL FROM modules WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(module_not_found)?;
    if archived {
        return Err(ApiError::bad("Archived module cannot be updated"));
    }
    if let Some(st_) = s(&b, "status")
        && !MODULE_STATUSES.contains(&st_)
    {
        return Err(ApiError::bad_body(
            json!({"status": [format!("\"{st_}\" is not a valid choice.")]}),
        ));
    }
    if let Some(n) = s(&b, "name") {
        let taken: (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM modules WHERE name = $1 AND project_id = $2 AND id <> $3 AND deleted_at IS NULL)",
        )
        .bind(n)
        .bind(project_id)
        .bind(pk)
        .fetch_one(&st.db)
        .await?;
        if taken.0 {
            return Err(ApiError::bad_body(
                json!({"error": "Module with this name already exists"}),
            ));
        }
    }
    let members = member_list(&b)?;
    let mut body = b.clone();
    if let Some(h) = s(&b, "description_html") {
        let clean = sanitize_html(h)?;
        body["description_html"] = json!(clean);
        body["description_text"] = json!(strip_tags(&clean));
    }
    if matches!(b.get("name"), Some(Value::Null)) || s(&b, "name").is_some_and(|n| n.trim().is_empty()) {
        return Err(ApiError::bad_body(json!({"name": ["This field may not be blank."]})));
    }
    let mut sets = crate::users::patch_sets(&body, MODULE_FIELDS, 3);
    let mut tx = st.db.begin().await?;
    if !sets.is_empty() {
        sets.push("updated_at = now()".into());
        sets.push("updated_by_id = $2".into());
        sqlx::query(&format!(
            "UPDATE modules SET {} WHERE id = $1 AND project_id = $4 AND workspace_id = $5 AND deleted_at IS NULL",
            sets.join(", ")
        ))
        .bind(pk)
        .bind(user.id)
        .bind(&body)
        .bind(project_id)
        .bind(p.workspace_id())
        .execute(&mut *tx)
        .await?;
    }
    if let Some(m) = members {
        sqlx::query("UPDATE module_members SET deleted_at = now() WHERE module_id = $1 AND deleted_at IS NULL")
            .bind(pk)
            .execute(&mut *tx)
            .await?;
        insert_members(&mut tx, pk, project_id, p.workspace_id(), &m, user.id).await?;
    }
    tx.commit().await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "module", "updated", json!({"id": pk}));
    json_row(
        sqlx::query_scalar(&format!("{} WHERE m.id = $2", module_select("$1")))
            .bind(user.id)
            .bind(pk),
        &st.db,
    )
    .await
}

async fn destroy(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    // Plane: admin, or the module's creator who is also a workspace member.
    if p.require(&[ADMIN]).is_err() {
        let (creator,): (Option<Uuid>,) = sqlx::query_as(
            "SELECT created_by_id FROM modules WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
        )
        .bind(pk)
        .bind(project_id)
        .bind(p.workspace_id())
        .fetch_optional(&st.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
        if creator != Some(user.id) {
            return Err(ApiError::forbidden());
        }
    }
    let mut tx = st.db.begin().await?;
    let r = sqlx::query("UPDATE modules SET deleted_at = now() WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL")
        .bind(pk)
        .bind(project_id)
        .bind(p.workspace_id())
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    // Plane deletes module issues, favorites and recent visits with the module. Soft delete here.
    for q in [
        "UPDATE module_issues SET deleted_at = now() WHERE module_id = $1 AND deleted_at IS NULL",
        "UPDATE module_members SET deleted_at = now() WHERE module_id = $1 AND deleted_at IS NULL",
        "UPDATE user_favorites SET deleted_at = now() WHERE entity_type = 'module' AND entity_identifier = $1 AND deleted_at IS NULL",
        "DELETE FROM user_recent_visits WHERE entity_name = 'module' AND entity_identifier = $1",
    ] {
        sqlx::query(q).bind(pk).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    crate::webhook_out::emit(&st, p.workspace_id(), "module", "deleted", json!({"id": pk}));
    no_content()
}

// ---------- module issues ----------

async fn list_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id)): Path<(String, Uuid, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let mut base = Base::new(
        format!(
            "{ACTIVE} AND i.project_id = $1 AND i.workspace_id = $2
             AND EXISTS (SELECT 1 FROM module_issues mi WHERE mi.issue_id = i.id AND mi.module_id = $3 AND mi.deleted_at IS NULL)"
        ),
        vec![Arg::Uuid(project_id), Arg::Uuid(p.workspace_id()), Arg::Uuid(module_id)],
    );
    if p.restricted_guest() {
        base.and("i.created_by_id = {}", Arg::Uuid(user.id));
    }
    workitems::list(&st.db, base, &q, Scope::Project(project_id)).await
}

#[derive(Deserialize)]
struct IssueIds {
    issues: Option<Vec<Uuid>>,
}

/// Plane `create_module_issues`: bulk add work items to a module. Ignores ids outside the project.
async fn add_module_issues(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let body: IssueIds =
        serde_json::from_value(b).map_err(|_| ApiError::bad_body(json!({"issues": ["Expected a list of UUIDs."]})))?;
    let ids = body.issues.unwrap_or_default();
    if ids.is_empty() {
        return Err(ApiError::bad("Issues are required"));
    }
    module_in_project(&st.db, p.workspace_id(), project_id, module_id).await?;
    let valid: Vec<Uuid> = sqlx::query_scalar(
        "SELECT i.id FROM issues i WHERE i.id = ANY($1) AND i.project_id = $2 AND i.workspace_id = $3 AND i.deleted_at IS NULL",
    )
    .bind(&ids)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    for id in valid {
        sqlx::query(
            "INSERT INTO module_issues (issue_id, module_id, project_id, workspace_id, created_by_id, updated_by_id)
             VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(module_id)
        .bind(project_id)
        .bind(p.workspace_id())
        .bind(user.id)
        .execute(&st.db)
        .await?;
    }
    // ponytail: issue activity rows skipped; the UI reads them only in the module activity tab.
    created(json!({"message": "success"}))
}

#[derive(Deserialize)]
struct ModulesBody {
    modules: Option<Vec<Uuid>>,
    removed_modules: Option<Vec<Uuid>>,
}

/// Plane `create_issue_modules`: link one issue to many modules and unlink some.
async fn add_issue_modules(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let body: ModulesBody =
        serde_json::from_value(b).map_err(|_| ApiError::bad_body(json!({"modules": ["Expected a list of UUIDs."]})))?;
    let (issue_ok,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM issues WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL)",
    )
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;
    if !issue_ok {
        return Err(ApiError::not_found());
    }
    // Only modules in this workspace and project.
    let wanted = body.modules.unwrap_or_default();
    let valid: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM modules WHERE id = ANY($1) AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
    )
    .bind(&wanted)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_all(&st.db)
    .await?;
    for m in valid {
        sqlx::query(
            "INSERT INTO module_issues (issue_id, module_id, project_id, workspace_id, created_by_id, updated_by_id)
             VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT DO NOTHING",
        )
        .bind(issue_id)
        .bind(m)
        .bind(project_id)
        .bind(p.workspace_id())
        .bind(user.id)
        .execute(&st.db)
        .await?;
    }
    for m in body.removed_modules.unwrap_or_default() {
        sqlx::query(
            "UPDATE module_issues SET deleted_at = now() WHERE issue_id = $1 AND module_id = $2
             AND project_id = $3 AND workspace_id = $4 AND deleted_at IS NULL",
        )
        .bind(issue_id)
        .bind(m)
        .bind(project_id)
        .bind(p.workspace_id())
        .execute(&st.db)
        .await?;
    }
    created(json!({"message": "success"}))
}

async fn get_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id, issue_id)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT jsonb_build_object('id', mi.id, 'module_id', mi.module_id, 'issue_id', mi.issue_id,
                    'project_id', mi.project_id, 'workspace_id', mi.workspace_id, 'created_at', mi.created_at,
                    'updated_at', mi.updated_at, 'created_by', mi.created_by_id, 'updated_by', mi.updated_by_id)
             FROM module_issues mi JOIN issues i ON i.id = mi.issue_id AND i.deleted_at IS NULL
             WHERE mi.module_id = $1 AND mi.issue_id = $2 AND mi.project_id = $3 AND mi.workspace_id = $4 AND mi.deleted_at IS NULL"
        ))
        .bind(module_id)
        .bind(issue_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

/// Plane `ModuleIssueViewSet.destroy`: unlink one issue from one module (soft).
async fn remove_issue(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id, issue_id)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    sqlx::query(
        "UPDATE module_issues SET deleted_at = now() WHERE module_id = $1 AND issue_id = $2 AND project_id = $3
         AND workspace_id = $4 AND deleted_at IS NULL",
    )
    .bind(module_id)
    .bind(issue_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .execute(&st.db)
    .await?;
    no_content()
}

// ---------- module links ----------

const LINK_JSON: &str = "jsonb_build_object('id', l.id, 'title', l.title, 'url', l.url, 'module', l.module_id,
    'project_id', l.project_id, 'workspace_id', l.workspace_id, 'metadata', l.metadata,
    'created_at', l.created_at, 'updated_at', l.updated_at, 'created_by', l.created_by_id, 'updated_by', l.updated_by_id)";

/// Plane's `URLField` check: scheme added when missing, then must parse as http(s) with a host.
fn check_url(raw: &str) -> Result<String, ApiError> {
    let has_scheme = raw.starts_with("http://") || raw.starts_with("https://");
    // Any other scheme (ftp://, javascript:) must not be prefixed, or it becomes `http://ftp://...`.
    if !has_scheme && raw.contains("://") {
        return Err(ApiError::bad("Invalid URL format."));
    }
    let full = if has_scheme {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let ok = url::Url::parse(&full)
        .map(|u| (u.scheme() == "http" || u.scheme() == "https") && u.host_str().is_some_and(|h| !h.is_empty()))
        .unwrap_or(false);
    if ok && full.len() <= 200 {
        Ok(full)
    } else {
        Err(ApiError::bad("Invalid URL format."))
    }
}

async fn list_links(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {LINK_JSON} FROM module_links l WHERE l.module_id = $1 AND l.project_id = $2 AND l.workspace_id = $3
             AND l.deleted_at IS NULL ORDER BY l.created_at DESC"
        ))
        .bind(module_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn create_link(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    module_in_project(&st.db, p.workspace_id(), project_id, module_id).await?;
    let raw = s(&b, "url").ok_or_else(|| ApiError::bad_body(json!({"url": ["This field is required."]})))?;
    let url = check_url(raw)?;
    let (dup,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM module_links WHERE url = $1 AND module_id = $2 AND deleted_at IS NULL)",
    )
    .bind(&url)
    .bind(module_id)
    .fetch_one(&st.db)
    .await?;
    if dup {
        return Err(ApiError::bad("URL already exists."));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO module_links (title, url, metadata, module_id, project_id, workspace_id, created_by_id, updated_by_id)
         VALUES ($1, $2, '{}'::jsonb, $3, $4, $5, $6, $6) RETURNING id",
    )
    .bind(s(&b, "title"))
    .bind(&url)
    .bind(module_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    let v: Value = sqlx::query_scalar(&format!("SELECT {LINK_JSON} FROM module_links l WHERE l.id = $1"))
        .bind(id)
        .fetch_one(&st.db)
        .await?;
    created(v)
}

async fn get_link(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id, pk)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {LINK_JSON} FROM module_links l WHERE l.id = $1 AND l.module_id = $2 AND l.project_id = $3
             AND l.workspace_id = $4 AND l.deleted_at IS NULL"
        ))
        .bind(pk)
        .bind(module_id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn update_link(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id, pk)): Path<(String, Uuid, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let url = match s(&b, "url") {
        Some(raw) => Some(check_url(raw)?),
        None => None,
    };
    if let Some(u) = &url {
        let (dup,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM module_links WHERE url = $1 AND module_id = $2 AND id <> $3 AND deleted_at IS NULL)",
        )
        .bind(u)
        .bind(module_id)
        .bind(pk)
        .fetch_one(&st.db)
        .await?;
        if dup {
            return Err(ApiError::bad("URL already exists for this Issue"));
        }
    }
    json_row(
        sqlx::query_scalar(&format!(
            "UPDATE module_links l SET url = COALESCE($5, url), title = COALESCE($6, title),
                updated_by_id = $7, updated_at = now()
             WHERE l.id = $1 AND l.module_id = $2 AND l.project_id = $3 AND l.workspace_id = $4 AND l.deleted_at IS NULL
             RETURNING {LINK_JSON}"
        ))
        .bind(pk)
        .bind(module_id)
        .bind(project_id)
        .bind(p.workspace_id())
        .bind(url)
        .bind(s(&b, "title"))
        .bind(user.id),
        &st.db,
    )
    .await
}

async fn destroy_link(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id, pk)): Path<(String, Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let r = sqlx::query(
        "UPDATE module_links SET deleted_at = now() WHERE id = $1 AND module_id = $2 AND project_id = $3
         AND workspace_id = $4 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(module_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

// ---------- favorites ----------

async fn list_favorites(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(
        sqlx::query_scalar(
            "SELECT jsonb_build_object('id', uf.id, 'entity_type', uf.entity_type, 'entity_identifier', uf.entity_identifier,
                    'name', uf.name, 'is_folder', uf.is_folder, 'sequence', uf.sequence, 'parent', uf.parent_id,
                    'project_id', uf.project_id, 'workspace_id', uf.workspace_id,
                    'created_at', uf.created_at, 'updated_at', uf.updated_at)
             FROM user_favorites uf JOIN modules m ON m.id = uf.entity_identifier AND m.deleted_at IS NULL
             WHERE uf.entity_type = 'module' AND uf.user_id = $1 AND uf.project_id = $2 AND uf.workspace_id = $3
               AND uf.deleted_at IS NULL ORDER BY uf.sequence",
        )
        .bind(user.id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn add_favorite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let module =
        uuid_of(&b, "module").ok_or_else(|| ApiError::bad_body(json!({"module": ["This field is required."]})))?;
    module_in_project(&st.db, p.workspace_id(), project_id, module).await?;
    sqlx::query(
        "INSERT INTO user_favorites (entity_type, entity_identifier, user_id, project_id, workspace_id, is_folder, sequence,
                                     created_by_id, updated_by_id)
         VALUES ('module', $1, $2, $3, $4, false, 65535, $2, $2)
         ON CONFLICT (entity_type, entity_identifier, user_id) WHERE deleted_at IS NULL DO NOTHING",
    )
    .bind(module)
    .bind(user.id)
    .bind(project_id)
    .bind(p.workspace_id())
    .execute(&st.db)
    .await?;
    no_content()
}

async fn remove_favorite(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let r = sqlx::query(
        "UPDATE user_favorites SET deleted_at = now() WHERE entity_type = 'module' AND entity_identifier = $1
         AND user_id = $2 AND project_id = $3 AND workspace_id = $4 AND deleted_at IS NULL",
    )
    .bind(module_id)
    .bind(user.id)
    .bind(project_id)
    .bind(p.workspace_id())
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

// ---------- user properties ----------

const PROPS_JSON: &str = "jsonb_build_object('id', up.id, 'module', up.module_id, 'user', up.user_id,
    'project_id', up.project_id, 'workspace_id', up.workspace_id, 'filters', up.filters,
    'rich_filters', up.rich_filters, 'display_filters', up.display_filters,
    'display_properties', up.display_properties, 'created_at', up.created_at, 'updated_at', up.updated_at)";

/// Plane `get_or_create`: a row per (module, user), created with Plane's defaults on first read.
async fn get_user_properties(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    module_in_project(&st.db, p.workspace_id(), project_id, module_id).await?;
    sqlx::query(
        "INSERT INTO module_user_properties (module_id, user_id, project_id, workspace_id, filters, display_filters,
                                             display_properties, rich_filters, created_by_id, updated_by_id)
         VALUES ($1, $2, $3, $4,
                 '{\"priority\":null,\"state\":null,\"state_group\":null,\"assignees\":null,\"created_by\":null,\"labels\":null,\"start_date\":null,\"target_date\":null,\"subscriber\":null}'::jsonb,
                 '{\"group_by\":null,\"order_by\":\"-created_at\",\"type\":null,\"sub_issue\":true,\"show_empty_groups\":true,\"layout\":\"list\",\"calendar_date_range\":\"\"}'::jsonb,
                 '{\"assignee\":true,\"attachment_count\":true,\"created_on\":true,\"due_date\":true,\"estimate\":true,\"key\":true,\"labels\":true,\"link\":true,\"priority\":true,\"start_date\":true,\"state\":true,\"sub_issue_count\":true,\"updated_on\":true}'::jsonb,
                 '{}'::jsonb, $2, $2)
         ON CONFLICT (module_id, user_id) WHERE deleted_at IS NULL DO NOTHING",
    )
    .bind(module_id)
    .bind(user.id)
    .bind(project_id)
    .bind(p.workspace_id())
    .execute(&st.db)
    .await?;
    json_row(
        sqlx::query_scalar(&format!(
            "SELECT {PROPS_JSON} FROM module_user_properties up
             WHERE up.module_id = $1 AND up.user_id = $2 AND up.project_id = $3 AND up.deleted_at IS NULL"
        ))
        .bind(module_id)
        .bind(user.id)
        .bind(project_id),
        &st.db,
    )
    .await
}

/// Plane replaces each of the four JSON fields when present, then returns 201 with the row.
async fn patch_user_properties(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id)): Path<(String, Uuid, Uuid)>,
    Json(b): Json<Value>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    // Each field keeps its stored value unless the body carries it. $5 is always referenced.
    let set = |f: &str| format!("{f} = COALESCE(($5::jsonb ->> '{f}')::jsonb, {f})");
    let q = format!(
        "UPDATE module_user_properties up SET updated_at = now(), {}, {}, {}, {}
         WHERE up.module_id = $1 AND up.user_id = $2 AND up.project_id = $3 AND up.workspace_id = $4 AND up.deleted_at IS NULL
         RETURNING {PROPS_JSON}",
        set("filters"),
        set("rich_filters"),
        set("display_filters"),
        set("display_properties"),
    );
    let v: Option<Value> = sqlx::query_scalar(&q)
        .bind(module_id)
        .bind(user.id)
        .bind(project_id)
        .bind(p.workspace_id())
        .bind(&b)
        .fetch_optional(&st.db)
        .await?;
    match v {
        Some(v) => Ok((StatusCode::CREATED, Json(v))),
        None => Err(ApiError::not_found()),
    }
}

// ---------- archive ----------

/// Plane: only completed or cancelled modules can be archived. Archiving also drops favorites.
async fn archive(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, module_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let (status,): (String,) = sqlx::query_as(
        "SELECT status FROM modules WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
    )
    .bind(module_id)
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(ApiError::not_found)?;
    if status != "completed" && status != "cancelled" {
        return Err(ApiError::bad("Only completed or cancelled modules can be archived"));
    }
    let (at,): (chrono::DateTime<chrono::Utc>,) = sqlx::query_as(
        "UPDATE modules SET archived_at = now(), updated_at = now() WHERE id = $1 RETURNING archived_at",
    )
    .bind(module_id)
    .fetch_one(&st.db)
    .await?;
    sqlx::query("UPDATE user_favorites SET deleted_at = now() WHERE entity_type = 'module' AND entity_identifier = $1 AND deleted_at IS NULL")
        .bind(module_id)
        .execute(&st.db)
        .await?;
    Ok(Json(json!({"archived_at": at.to_rfc3339()})))
}

async fn unarchive(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let r = sqlx::query(
        "UPDATE modules SET archived_at = NULL, updated_at = now() WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
    )
    .bind(pk)
    .bind(project_id)
    .bind(p.workspace_id())
    .execute(&st.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    no_content()
}

async fn list_archived(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {} FROM modules m {COUNTS}
             WHERE m.project_id = $2 AND m.workspace_id = $3 AND m.archived_at IS NOT NULL AND m.deleted_at IS NULL
             ORDER BY {} DESC, m.created_at DESC",
            module_json(&fav("$1")),
            fav("$1"),
        ))
        .bind(user.id)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

async fn get_archived(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, pk)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    json_row(
        sqlx::query_scalar(&format!(
            "{} WHERE m.id = $2 AND m.project_id = $3 AND m.workspace_id = $4 AND m.archived_at IS NOT NULL AND m.deleted_at IS NULL",
            module_select("$1")
        ))
        .bind(user.id)
        .bind(pk)
        .bind(project_id)
        .bind(p.workspace_id()),
        &st.db,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::check_url;

    #[test]
    fn check_url_adds_scheme_and_rejects_other_schemes() {
        assert_eq!(check_url("example.com/x").unwrap(), "http://example.com/x");
        assert_eq!(check_url("https://a.b").unwrap(), "https://a.b");
        assert!(check_url("ftp://nope").is_err());
        assert!(check_url("javascript:alert(1)").is_err());
        assert!(check_url("http://").is_err());
    }
}

/// `GET /api/workspaces/{slug}/modules/`: non-archived modules of every project the user is an active
/// member of (Plane's `WorkspaceModulesEndpoint`).
async fn workspace_list(State(st): State<AppState>, user: CurrentUser, Path(slug): Path<String>) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    json_rows(
        sqlx::query_scalar(&format!(
            "SELECT {} FROM modules m {COUNTS}
             JOIN projects p ON p.id = m.project_id AND p.deleted_at IS NULL AND p.archived_at IS NULL
             WHERE m.workspace_id = $2 AND m.archived_at IS NULL AND m.deleted_at IS NULL
               AND EXISTS (SELECT 1 FROM project_members pm WHERE pm.project_id = m.project_id AND pm.member_id = $1
                           AND pm.is_active AND pm.deleted_at IS NULL)
             ORDER BY m.created_at DESC",
            module_json(&fav("$1")),
        ))
        .bind(user.id)
        .bind(ws.workspace_id),
        &st.db,
    )
    .await
}
