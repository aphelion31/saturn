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
    Router::new().route("/api/workspaces/{slug}/projects/{project_id}/issues/", get(list))
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
