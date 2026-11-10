//! Work items (issues): CRUD, comments, links, relations, sub-issues, activity, reactions, archive, search.
//! The list engine lives in `workitems.rs`; this module owns the issue routes.
//! TODO (worker): port the rest of `app/urls/issue.py`.
use std::collections::HashMap;

use axum::{
    Router,
    extract::{Path, Query, State},
    routing::get,
};
use uuid::Uuid;

use crate::common::*;
use crate::workitems::{self, ACTIVE, Arg, Base, Scope};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/workspaces/{slug}/projects/{project_id}/issues/", get(list))
        .route("/api/workspaces/{slug}/projects/{project_id}/issues/list/", get(list))
        .route("/api/workspaces/{slug}/projects/{project_id}/v2/issues/", get(list))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/archived-issues/",
            get(archived_list),
        )
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/issues/{issue_id}/",
            get(detail),
        )
}

async fn archived_list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;
    let base = Base::new(
        "i.project_id = $1 AND i.workspace_id = $2 AND i.deleted_at IS NULL AND i.archived_at IS NOT NULL",
        vec![Arg::Uuid(project_id), Arg::Uuid(p.workspace_id())],
    );
    workitems::list(&st.db, base, &q, Scope::Project(project_id)).await
}

async fn detail(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id, issue_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let sql = format!(
        "SELECT {} || jsonb_build_object('description_html', i.description_html,
          'is_subscribed', EXISTS (SELECT 1 FROM issue_subscribers sub WHERE sub.issue_id = i.id
          AND sub.workspace_id = i.workspace_id AND sub.project_id = i.project_id
          AND sub.subscriber_id = $4 AND sub.deleted_at IS NULL), 'is_intake',
          EXISTS (SELECT 1 FROM intake_issues ii WHERE ii.issue_id = i.id AND ii.project_id = i.project_id
          AND ii.workspace_id = i.workspace_id AND ii.status IN (-2, 0) AND ii.deleted_at IS NULL))
         FROM issues i WHERE i.project_id = $1 AND i.workspace_id = $2 AND i.id = $3
         AND {ACTIVE} AND (NOT $5 OR i.created_by_id = $4)",
        workitems::ITEM_JSON
    );
    json_row(
        sqlx::query_scalar(&sql)
            .bind(project_id)
            .bind(p.workspace_id())
            .bind(issue_id)
            .bind(user.id)
            .bind(p.restricted_guest()),
        &st.db,
    )
    .await
}

async fn list(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER, GUEST])?;
    let mut base = Base::new(
        format!("i.project_id = $1 AND i.workspace_id = $2 AND {ACTIVE}"),
        vec![Arg::Uuid(project_id), Arg::Uuid(p.workspace_id())],
    );
    if p.restricted_guest() {
        base.and("i.created_by_id = {}", Arg::Uuid(user.id));
    }
    workitems::list(&st.db, base, &q, Scope::Project(project_id)).await
}
