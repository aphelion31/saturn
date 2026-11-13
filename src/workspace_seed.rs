//! Port of Plane's `workspace_seed_task`: after a workspace is created through the web API, seed a demo
//! project (states, labels, cycles, modules, issues, views, pages) owned by a WORKSPACE_SEED bot user.
//! Runs in a background task; failures are logged and never fail the create request.
//! Disable with SATURN_WORKSPACE_SEED=0.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::common::AppState;

const PROJECTS: &str = include_str!("../assets/seeds/projects.json");
const STATES: &str = include_str!("../assets/seeds/states.json");
const LABELS: &str = include_str!("../assets/seeds/labels.json");
const CYCLES: &str = include_str!("../assets/seeds/cycles.json");
const MODULES: &str = include_str!("../assets/seeds/modules.json");
const ISSUES: &str = include_str!("../assets/seeds/issues.json");
const VIEWS: &str = include_str!("../assets/seeds/views.json");
const PAGES: &str = include_str!("../assets/seeds/pages.json");

#[derive(Deserialize)]
struct ProjectSeed {
    id: i64,
}
#[derive(Deserialize)]
struct StateSeed {
    id: i64,
    project_id: i64,
    name: String,
    color: String,
    sequence: f64,
    group: String,
    default: bool,
}
#[derive(Deserialize)]
struct LabelSeed {
    id: i64,
    project_id: i64,
    name: String,
    color: String,
    sort_order: f64,
}
#[derive(Deserialize)]
struct CycleSeed {
    id: i64,
    project_id: i64,
    name: String,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
struct ModuleSeed {
    id: i64,
    project_id: i64,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    sort_order: f64,
}
#[derive(Deserialize)]
struct IssueSeed {
    project_id: i64,
    state_id: i64,
    name: String,
    sequence_id: i32,
    #[serde(default)]
    description_html: String,
    #[serde(default)]
    description_stripped: String,
    #[serde(default)]
    priority: String,
    #[serde(default)]
    sort_order: f64,
    #[serde(default)]
    labels: Vec<i64>,
    #[serde(default)]
    module_ids: Vec<i64>,
    cycle_id: Option<i64>,
}
#[derive(Deserialize)]
struct ViewSeed {
    project_id: i64,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    query: Value,
    #[serde(default)]
    filters: Value,
    #[serde(default)]
    display_filters: Value,
    #[serde(default)]
    display_properties: Value,
    #[serde(default)]
    sort_order: f64,
}
#[derive(Deserialize)]
struct PageSeed {
    project_id: Option<i64>,
    #[serde(rename = "type", default)]
    kind: String,
    name: String,
    #[serde(default)]
    access: i16,
    #[serde(default)]
    description_html: String,
    #[serde(default)]
    description_stripped: String,
}

fn parse<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<Vec<T>, String> {
    serde_json::from_str(raw).map_err(|e| e.to_string())
}

/// Spawns the seed. Call after the workspace create transaction has committed.
pub fn spawn(st: &AppState, workspace_id: Uuid) {
    if std::env::var("SATURN_WORKSPACE_SEED").as_deref() == Ok("0") {
        return;
    }
    let st = st.clone();
    tokio::spawn(async move {
        if let Err(e) = seed(&st, workspace_id).await {
            tracing::error!(%workspace_id, error = %e, "workspace seed failed");
        }
    });
}

async fn seed(st: &AppState, ws: Uuid) -> Result<(), String> {
    let db = &st.db;
    let mut tx = db.begin().await.map_err(|e| e.to_string())?;
    let (name, owner): (String, Uuid) = sqlx::query_as("SELECT name, owner_id FROM workspaces WHERE id = $1")
        .bind(ws)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // Bot user: email host from WEB_URL like Plane (bot_user_<wsid>@<host>).
    let host = url_host(&st.cfg.web_url).unwrap_or_else(|| "plane.so".into());
    let bot: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, username, password, first_name, last_name, display_name, token, is_active,
                            is_bot, bot_type, is_password_autoset, last_login_medium, token_updated_at, is_staff)
         VALUES ($1, $2, '!', 'Plane', '', 'Plane', $3, true, true, 'WORKSPACE_SEED', true, 'email', now(), false)
         RETURNING id",
    )
    .bind(format!("bot_user_{ws}@{host}"))
    .bind(format!("bot_user_{ws}"))
    .bind(Uuid::new_v4().simple().to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("INSERT INTO workspace_members (workspace_id, member_id, role, company_role, created_by_id) VALUES ($1, $2, 20, '', $3)")
        .bind(ws)
        .bind(bot)
        .bind(owner)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // Every active workspace member (the bot included, as in Plane) gets project membership and properties.
    let members: Vec<(Uuid, i16)> = sqlx::query_as(
        "SELECT member_id, role FROM workspace_members WHERE workspace_id = $1 AND deleted_at IS NULL AND is_active",
    )
    .bind(ws)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    // Project: one per projects.json entry, named after the workspace.
    let identifier: String = name.chars().filter(|c| c.is_alphanumeric()).take(5).collect::<String>().to_uppercase();
    let mut projects: HashMap<i64, Uuid> = HashMap::new();
    for seed in parse::<ProjectSeed>(PROJECTS)? {
        let pid: Uuid = sqlx::query_scalar(
            "INSERT INTO projects (name, identifier, workspace_id, created_by_id, updated_by_id, network, cover_image,
                                   cycle_view, module_view, issue_views_view, timezone)
             VALUES ($1, $2, $3, $4, $4, 2, NULL, true, true, true, 'UTC') RETURNING id",
        )
        .bind(&name)
        .bind(&identifier)
        .bind(ws)
        .bind(bot)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        sqlx::query("INSERT INTO project_identifiers (name, project_id, workspace_id, created_by_id, updated_by_id) VALUES ($1, $2, $3, $4, $4)")
            .bind(&identifier)
            .bind(pid)
            .bind(ws)
            .bind(bot)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        for (member, role) in &members {
            sqlx::query("INSERT INTO project_members (workspace_id, project_id, member_id, role, created_by_id) VALUES ($1, $2, $3, $4, $5)")
                .bind(ws)
                .bind(pid)
                .bind(member)
                .bind(role)
                .bind(bot)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            sqlx::query(
                "INSERT INTO project_user_properties (workspace_id, project_id, user_id, created_by_id, updated_by_id,
                    display_filters, display_properties)
                 VALUES ($1, $2, $3, $4, $4, $5, $6)",
            )
            .bind(ws)
            .bind(pid)
            .bind(member)
            .bind(bot)
            .bind(serde_json::json!({
                "layout": "list", "calendar": {"layout": "month", "show_weekends": false},
                "group_by": "state", "order_by": "sort_order", "sub_issue": true,
                "sub_group_by": null, "show_empty_groups": true
            }))
            .bind(serde_json::json!({
                "key": true, "link": true, "cycle": false, "state": true, "labels": false, "modules": false,
                "assignee": true, "due_date": false, "estimate": true, "priority": true, "created_on": true,
                "issue_type": true, "start_date": false, "updated_on": true, "customer_count": true,
                "sub_issue_count": false, "attachment_count": false, "customer_request_count": true
            }))
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        projects.insert(seed.id, pid);
    }
    let project_of = |id: i64| projects.get(&id).copied().ok_or_else(|| format!("unknown project seed {id}"));
    // Seed IDs are only used inside this function; each row resolves through these maps.

    // States.
    let mut states: HashMap<i64, Uuid> = HashMap::new();
    for s in parse::<StateSeed>(STATES)? {
        let sid: Uuid = sqlx::query_scalar(
            "INSERT INTO states (name, color, sequence, \"group\", \"default\", slug, project_id, workspace_id,
                                 created_by_id, updated_by_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9) RETURNING id",
        )
        .bind(&s.name)
        .bind(&s.color)
        .bind(s.sequence)
        .bind(&s.group)
        .bind(s.default)
        .bind(crate::states::slugify(&s.name))
        .bind(project_of(s.project_id)?)
        .bind(ws)
        .bind(bot)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        states.insert(s.id, sid);
    }

    // Labels.
    let mut labels: HashMap<i64, Uuid> = HashMap::new();
    for l in parse::<LabelSeed>(LABELS)? {
        let lid: Uuid = sqlx::query_scalar(
            "INSERT INTO labels (name, color, sort_order, project_id, workspace_id, created_by_id, updated_by_id)
             VALUES ($1, $2, $3, $4, $5, $6, $6) RETURNING id",
        )
        .bind(&l.name)
        .bind(&l.color)
        .bind(l.sort_order)
        .bind(project_of(l.project_id)?)
        .bind(ws)
        .bind(bot)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        labels.insert(l.id, lid);
    }

    // Cycles: CURRENT starts now, UPCOMING follows the last cycle of its project.
    let mut cycles: HashMap<i64, Uuid> = HashMap::new();
    for c in parse::<CycleSeed>(CYCLES)? {
        let pid = project_of(c.project_id)?;
        let (start, end): (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) = if c.kind == "UPCOMING" {
            let last: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
                "SELECT end_date FROM cycles WHERE project_id = $1 AND deleted_at IS NULL ORDER BY end_date DESC LIMIT 1",
            )
            .bind(pid)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
            .flatten();
            let start = last.map(|d| d + chrono::Duration::days(1)).unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::days(14));
            (start, start + chrono::Duration::days(14))
        } else {
            let start = chrono::Utc::now();
            (start, start + chrono::Duration::days(14))
        };
        let cid: Uuid = sqlx::query_scalar(
            "INSERT INTO cycles (name, start_date, end_date, project_id, workspace_id, created_by_id, owned_by_id, updated_by_id)
             VALUES ($1, $2, $3, $4, $5, $6, $6, $6) RETURNING id",
        )
        .bind(&c.name)
        .bind(start)
        .bind(end)
        .bind(pid)
        .bind(ws)
        .bind(bot)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        cycles.insert(c.id, cid);
    }

    // Modules: staggered two days apart, 14 day windows.
    let mut modules: HashMap<i64, Uuid> = HashMap::new();
    for (index, m) in parse::<ModuleSeed>(MODULES)?.into_iter().enumerate() {
        let start = chrono::Utc::now().date_naive() + chrono::Duration::days(index as i64 * 2);
        let target = start + chrono::Duration::days(14);
        let mid: Uuid = sqlx::query_scalar(
            "INSERT INTO modules (name, description, status, sort_order, start_date, target_date, project_id, workspace_id,
                                  created_by_id, updated_by_id)
             VALUES ($1, $2, COALESCE(NULLIF($3, ''), 'planned'), $4, $5, $6, $7, $8, $9, $9) RETURNING id",
        )
        .bind(&m.name)
        .bind(&m.description)
        .bind(&m.status)
        .bind(m.sort_order)
        .bind(start)
        .bind(target)
        .bind(project_of(m.project_id)?)
        .bind(ws)
        .bind(bot)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        modules.insert(m.id, mid);
    }

    // Issues: sequence, creation activity, labels, cycle and module links.
    for i in parse::<IssueSeed>(ISSUES)? {
        let pid = project_of(i.project_id)?;
        let state = states.get(&i.state_id).copied().ok_or("unknown state seed")?;
        let iid: Uuid = sqlx::query_scalar(
            "INSERT INTO issues (name, project_id, workspace_id, state_id, sequence_id, created_by_id, updated_by_id,
                                 description_html, description_stripped, priority, sort_order)
             VALUES ($1, $2, $3, $4, $5, $6, $6, $7, $8, COALESCE(NULLIF($9, ''), 'none'), $10) RETURNING id",
        )
        .bind(&i.name)
        .bind(pid)
        .bind(ws)
        .bind(state)
        .bind(i.sequence_id)
        .bind(bot)
        .bind(&i.description_html)
        .bind(&i.description_stripped)
        .bind(&i.priority)
        .bind(i.sort_order)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        sqlx::query("INSERT INTO issue_sequences (sequence, project_id, workspace_id, issue_id, created_by_id, updated_by_id) VALUES ($1, $2, $3, $4, $5, $5)")
            .bind(i64::from(i.sequence_id))
            .bind(pid)
            .bind(ws)
            .bind(iid)
            .bind(bot)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("INSERT INTO issue_activities (project_id, workspace_id, issue_id, actor_id, verb, field, comment, epoch, created_by_id, updated_by_id) VALUES ($1, $2, $3, $4, 'created', NULL, 'created the issue', extract(epoch from now()), $4, $4)")
            .bind(pid)
            .bind(ws)
            .bind(iid)
            .bind(bot)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        for l in &i.labels {
            let lid = labels.get(l).copied().ok_or("unknown label seed")?;
            sqlx::query("INSERT INTO issue_labels (issue_id, label_id, project_id, workspace_id, created_by_id, updated_by_id) VALUES ($1, $2, $3, $4, $5, $5)")
                .bind(iid)
                .bind(lid)
                .bind(pid)
                .bind(ws)
                .bind(bot)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        if let Some(c) = i.cycle_id {
            let cid = cycles.get(&c).copied().ok_or("unknown cycle seed")?;
            sqlx::query("INSERT INTO cycle_issues (issue_id, cycle_id, project_id, workspace_id, created_by_id, updated_by_id) VALUES ($1, $2, $3, $4, $5, $5)")
                .bind(iid)
                .bind(cid)
                .bind(pid)
                .bind(ws)
                .bind(bot)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        for m in &i.module_ids {
            let mid = modules.get(m).copied().ok_or("unknown module seed")?;
            sqlx::query("INSERT INTO module_issues (issue_id, module_id, project_id, workspace_id, created_by_id, updated_by_id) VALUES ($1, $2, $3, $4, $5, $5)")
                .bind(iid)
                .bind(mid)
                .bind(pid)
                .bind(ws)
                .bind(bot)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    // Views.
    for v in parse::<ViewSeed>(VIEWS)? {
        sqlx::query(
            "INSERT INTO issue_views (name, description, query, filters, display_filters, display_properties, sort_order,
                                      access, project_id, workspace_id, created_by_id, updated_by_id, owned_by_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, $8, $9, $10, $10, $10)",
        )
        .bind(&v.name)
        .bind(&v.description)
        .bind(&v.query)
        .bind(&v.filters)
        .bind(&v.display_filters)
        .bind(&v.display_properties)
        .bind(v.sort_order)
        .bind(project_of(v.project_id)?)
        .bind(ws)
        .bind(bot)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    // Pages.
    for p in parse::<PageSeed>(PAGES)? {
        let page: Uuid = sqlx::query_scalar(
            "INSERT INTO pages (name, description_html, description_stripped, access, workspace_id, is_global,
                                created_by_id, updated_by_id, owned_by_id)
             VALUES ($1, $2, $3, $4, $5, false, $6, $6, $6) RETURNING id",
        )
        .bind(&p.name)
        .bind(if p.description_html.is_empty() { "<p></p>" } else { &p.description_html })
        .bind(&p.description_stripped)
        .bind(p.access)
        .bind(ws)
        .bind(bot)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        if let (Some(pid), "PROJECT") = (p.project_id, p.kind.as_str()) {
            sqlx::query("INSERT INTO project_pages (workspace_id, project_id, page_id, created_by_id, updated_by_id) VALUES ($1, $2, $3, $4, $4)")
                .bind(ws)
                .bind(project_of(pid)?)
                .bind(page)
                .bind(bot)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    tracing::info!(%ws, "workspace seeded");
    Ok(())
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let host = rest.split(['/', ':']).next()?;
    (!host.is_empty()).then(|| host.to_string())
}
