//! External integrations: Unsplash, AI Assistant (LLM), Issue Exports, Analytics Export, User Activity Export.
//! Mirrors Plane's external views, exporter views, and analytic export bgtask.
use std::collections::HashMap;

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::NaiveDate;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        // 1. Unsplash proxy
        .route("/api/unsplash/", get(unsplash_proxy))
        // 2. AI assistant
        .route("/api/workspaces/{slug}/ai-assistant/", post(workspace_ai_assistant))
        .route(
            "/api/workspaces/{slug}/projects/{project_id}/ai-assistant/",
            post(project_ai_assistant),
        )
        // 3. Issue exports
        .route(
            "/api/workspaces/{slug}/export-issues/",
            get(list_exporters).post(create_export),
        )
        .route(
            "/api/workspaces/{slug}/export-issues/{token}/download/",
            get(download_export),
        )
        // 4. Export analytics
        .route("/api/workspaces/{slug}/export-analytics/", post(export_analytics))
        // 5. User activity CSV export
        .route(
            "/api/workspaces/{slug}/user-activity/{user_id}/export/",
            get(user_activity_export_get).post(user_activity_export_post),
        )
}

type Q = HashMap<String, String>;

// ============================================================================
// 1. Unsplash Proxy
// ============================================================================

async fn unsplash_proxy(State(st): State<AppState>, _user: CurrentUser, Query(q): Query<Q>) -> ApiResult<Response> {
    let key = crate::outbound::config(&st, "UNSPLASH_ACCESS_KEY", "").await;
    if key.is_empty() {
        return Ok(Json(json!([])).into_response());
    }

    let query = q.get("query").map(String::as_str).unwrap_or("");
    let page = q.get("page").map(String::as_str).unwrap_or("1");
    let per_page = q.get("per_page").map(String::as_str).unwrap_or("20");

    // Key goes in the Authorization header (Unsplash accepts `Client-ID`), never in a URL that could be logged.
    let url = if !query.is_empty() {
        format!(
            "https://api.unsplash.com/search/photos/?query={}&page={}&per_page={}",
            urlencoding::encode(query),
            urlencoding::encode(page),
            urlencoding::encode(per_page),
        )
    } else {
        format!(
            "https://api.unsplash.com/photos/?page={}&per_page={}",
            urlencoding::encode(page),
            urlencoding::encode(per_page),
        )
    };

    let client = crate::outbound::http();
    let resp = match client
        .get(&url)
        .header(header::AUTHORIZATION, format!("Client-ID {key}"))
        .header("Content-Type", "application/json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("unsplash request failed: {}", e.without_url());
            return Ok((
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "Failed to connect to Unsplash"})),
            )
                .into_response());
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
    let bytes = resp
        .bytes()
        .await
        .map_err(|_| ApiError::bad("Failed to read Unsplash response"))?;

    let mut res = Body::from(bytes).into_response();
    *res.status_mut() = status;
    res.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(res)
}

// Simple percent-encoding helper without external dependency
mod urlencoding {
    pub fn encode(s: &str) -> String {
        url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
    }
}

// ============================================================================
// 2. AI Assistant (LLM)
// ============================================================================

struct LlmConfig {
    api_key: String,
    provider: String,
    model: String,
}

async fn get_llm_config(st: &AppState) -> Option<LlmConfig> {
    let api_key = crate::outbound::config(st, "LLM_API_KEY", "").await;
    if api_key.is_empty() {
        return None;
    }
    let provider = crate::outbound::config(st, "LLM_PROVIDER", "openai")
        .await
        .to_lowercase();
    let model = crate::outbound::config(st, "LLM_MODEL", "").await;

    let default_model = match provider.as_str() {
        "openai" => "gpt-4o-mini",
        "anthropic" => "claude-3-sonnet-20240229",
        "gemini" => "gemini-pro",
        _ => return None,
    };

    let resolved_model = if model.is_empty() {
        default_model.to_string()
    } else {
        model
    };

    Some(LlmConfig {
        api_key,
        provider,
        model: resolved_model,
    })
}

async fn call_llm(cfg: &LlmConfig, task: &str, prompt: &str) -> Result<String, String> {
    let final_text = if prompt.is_empty() {
        task.to_string()
    } else {
        format!("{task}\n{prompt}")
    };

    let client = crate::outbound::http();

    match cfg.provider.as_str() {
        "openai" => {
            let body = json!({
                "model": cfg.model,
                "messages": [
                    {"role": "user", "content": final_text}
                ]
            });
            let res = client
                .post("https://api.openai.com/v1/chat/completions")
                .bearer_auth(&cfg.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if !res.status().is_success() {
                let err_text = res.text().await.unwrap_or_default();
                return Err(format!("OpenAI error: {err_text}"));
            }
            let data: Value = res.json().await.map_err(|e| e.to_string())?;
            let content = data["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string();
            Ok(content)
        }
        "anthropic" => {
            let body = json!({
                "model": cfg.model,
                "max_tokens": 4096,
                "messages": [
                    {"role": "user", "content": final_text}
                ]
            });
            let res = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &cfg.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if !res.status().is_success() {
                let err_text = res.text().await.unwrap_or_default();
                return Err(format!("Anthropic error: {err_text}"));
            }
            let data: Value = res.json().await.map_err(|e| e.to_string())?;
            let content = data["content"][0]["text"].as_str().unwrap_or("").to_string();
            Ok(content)
        }
        "gemini" => {
            let model = cfg.model.strip_prefix("gemini/").unwrap_or(&cfg.model);
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                urlencoding::encode(model),
            );
            let body = json!({
                "contents": [
                    {
                        "parts": [
                            {"text": final_text}
                        ]
                    }
                ]
            });
            let res = client
                .post(&url)
                .header("x-goog-api-key", &cfg.api_key)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if !res.status().is_success() {
                let err_text = res.text().await.unwrap_or_default();
                return Err(format!("Gemini error: {err_text}"));
            }
            let data: Value = res.json().await.map_err(|e| e.to_string())?;
            let content = data["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_string();
            Ok(content)
        }
        _ => Err("Unsupported LLM provider".to_string()),
    }
}

async fn workspace_ai_assistant(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;

    let cfg = match get_llm_config(&st).await {
        Some(c) => c,
        None => return Err(ApiError::bad("LLM provider API key and model are required")),
    };

    let task = body
        .get("task")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| ApiError::bad("Task is required"))?;
    let prompt = body.get("prompt").and_then(Value::as_str).unwrap_or("");

    match call_llm(&cfg, task, prompt).await {
        Ok(text) => Ok(Json(json!({
            "response": text,
            "response_html": text.replace('\n', "<br/>"),
        }))),
        Err(e) => {
            tracing::error!("LLM generation failed: {e}");
            Err(ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": "An internal error has occurred."}),
            ))
        }
    }
}

async fn project_ai_assistant(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, project_id)): Path<(String, Uuid)>,
    Json(body): Json<Value>,
) -> ApiResult {
    let p = require_project(&st.db, &user, &slug, project_id).await?;
    p.require(&[ADMIN, MEMBER])?;

    let cfg = match get_llm_config(&st).await {
        Some(c) => c,
        None => return Err(ApiError::bad("LLM provider API key and model are required")),
    };

    let task = body
        .get("task")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| ApiError::bad("Task is required"))?;
    let prompt = body.get("prompt").and_then(Value::as_str).unwrap_or("");

    let (project_detail, workspace_detail): (Value, Value) = sqlx::query_as(
        "SELECT
            jsonb_build_object(
                'id', p.id,
                'identifier', p.identifier,
                'name', p.name,
                'cover_image', p.cover_image,
                'cover_image_url', p.cover_image,
                'logo_props', p.logo_props,
                'description', p.description
            ),
            jsonb_build_object(
                'name', w.name,
                'slug', w.slug,
                'id', w.id,
                'logo_url', NULLIF(w.logo, '')
            )
         FROM projects p
         JOIN workspaces w ON w.id = p.workspace_id
         WHERE p.id = $1 AND p.workspace_id = $2",
    )
    .bind(project_id)
    .bind(p.workspace_id())
    .fetch_one(&st.db)
    .await?;

    match call_llm(&cfg, task, prompt).await {
        Ok(text) => Ok(Json(json!({
            "response": text,
            "response_html": text.replace('\n', "<br/>"),
            "project_detail": project_detail,
            "workspace_detail": workspace_detail,
        }))),
        Err(e) => {
            tracing::error!("LLM generation failed: {e}");
            Err(ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": "An internal error has occurred."}),
            ))
        }
    }
}

// ============================================================================
// 3. Issue Exports
// ============================================================================

const EXPORTER_JSON: &str = "jsonb_build_object(
    'id', e.id,
    'created_at', e.created_at,
    'updated_at', e.updated_at,
    'project', e.project,
    'provider', e.provider,
    'status', e.status,
    'url', e.url,
    'token', e.token,
    'created_by', e.created_by_id,
    'updated_by', e.updated_by_id,
    'initiated_by', e.initiated_by_id,
    'initiated_by_detail', (SELECT jsonb_build_object('id', u.id, 'first_name', u.first_name, 'last_name', u.last_name,
        'avatar', u.avatar, 'avatar_url', CASE WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id || '/'
                                               ELSE NULLIF(u.avatar, '') END,
        'is_bot', u.is_bot, 'display_name', u.display_name) FROM users u WHERE u.id = e.initiated_by_id)
)";

fn page_of_cursor(cursor: Option<&String>) -> i64 {
    cursor
        .and_then(|c| c.split(':').nth(1))
        .and_then(|p| p.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0)
}

async fn list_exporters(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Query(q): Query<Q>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;

    let per_page = match q.get("per_page").and_then(|v| v.parse::<i64>().ok()) {
        Some(p) => p.clamp(1, 100),
        None => return Err(ApiError::bad("per_page and cursor are required")),
    };
    if !q.contains_key("cursor") {
        return Err(ApiError::bad("per_page and cursor are required"));
    }
    let page = page_of_cursor(q.get("cursor"));

    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM exporters e
         WHERE e.workspace_id = $1 AND e.type = 'issue_exports' AND e.initiated_by_id = $2
           AND e.deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;

    let mut rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT {EXPORTER_JSON} FROM exporters e
         WHERE e.workspace_id = $1 AND e.type = 'issue_exports' AND e.initiated_by_id = $2
           AND e.deleted_at IS NULL
         ORDER BY e.created_at DESC
         LIMIT $3 OFFSET $4"
    ))
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(per_page + 1)
    .bind(page * per_page)
    .fetch_all(&st.db)
    .await?;

    let has_next = rows.len() as i64 > per_page;
    rows.truncate(per_page as usize);

    Ok(Json(json!({
        "next_cursor": format!("{per_page}:{}:0", page + 1),
        "prev_cursor": format!("{per_page}:{}:1", (page - 1).max(0)),
        "next_page_results": has_next,
        "prev_page_results": page > 0,
        "count": rows.len(),
        "total_pages": (total + per_page - 1) / per_page,
        "total_results": total,
        "extra_stats": null,
        "results": rows,
    })))
}

async fn create_export(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;

    let provider = body.get("provider").and_then(Value::as_str).unwrap_or("");
    if !["csv", "xlsx", "json"].contains(&provider) {
        return Err(ApiError::bad(format!("Provider '{provider}' not found.")));
    }

    // Selected project ids from body, or all active non-archived projects user has access to
    let mut project_ids: Vec<Uuid> = Vec::new();
    if let Some(arr) = body.get("project").and_then(Value::as_array) {
        for v in arr {
            if let Some(s) = v.as_str() {
                if let Ok(u) = Uuid::parse_str(s) {
                    project_ids.push(u);
                }
            }
        }
    }

    if project_ids.is_empty() {
        project_ids = sqlx::query_scalar(
            "SELECT p.id FROM projects p
             JOIN project_members pm ON pm.project_id = p.id
             WHERE p.workspace_id = $1 AND pm.member_id = $2 AND pm.is_active
               AND pm.deleted_at IS NULL AND p.deleted_at IS NULL AND p.archived_at IS NULL",
        )
        .bind(ws.workspace_id)
        .bind(user.id)
        .fetch_all(&st.db)
        .await?;
    }

    let token = Uuid::new_v4().simple().to_string();
    let exporter_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO exporters (
            id, created_at, updated_at, workspace_id, initiated_by_id, created_by_id, updated_by_id,
            project, provider, status, reason, key, token, type, filters, rich_filters
         ) VALUES ($1, now(), now(), $2, $3, $3, $3, $4, $5, 'queued', '', '', $6, 'issue_exports', null, '{}')",
    )
    .bind(exporter_id)
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(&project_ids)
    .bind(provider)
    .bind(&token)
    .execute(&st.db)
    .await?;

    // Background task to perform export
    let st_bg = st.clone();
    let slug_bg = slug.clone();
    let provider_bg = provider.to_string();
    let token_bg = token.clone();
    let ws_id = ws.workspace_id;
    let user_id = user.id;

    tokio::spawn(async move {
        run_issue_export_bg(
            st_bg,
            exporter_id,
            ws_id,
            slug_bg,
            user_id,
            project_ids,
            provider_bg,
            token_bg,
        )
        .await;
    });

    Ok(Json(json!({
        "message": "Once the export is ready you will be able to download it"
    })))
}

async fn run_issue_export_bg(
    st: AppState,
    exporter_id: Uuid,
    ws_id: Uuid,
    slug: String,
    user_id: Uuid,
    project_ids: Vec<Uuid>,
    provider: String,
    token: String,
) {
    let _ = sqlx::query("UPDATE exporters SET status = 'processing', updated_at = now() WHERE id = $1")
        .bind(exporter_id)
        .execute(&st.db)
        .await;

    // ponytail: xlsx format falls back to CSV content with .xlsx filename; add full openxml generator when needed
    let is_json = provider == "json";
    let ext = if is_json {
        "json"
    } else if provider == "xlsx" {
        "xlsx"
    } else {
        "csv"
    };
    let filename = format!("export-{slug}-{token}.{ext}");
    let key = format!("exports/{token}/{filename}");

    // Fetch issues visible to user
    let issues_query = "
        SELECT
            p.name AS project_name,
            p.identifier AS project_identifier,
            COALESCE(parent_p.identifier || '-' || parent_i.sequence_id::text, '') AS parent_identifier,
            p.identifier || '-' || i.sequence_id::text AS identifier,
            i.sequence_id,
            i.name AS issue_name,
            COALESCE(s.name, '') AS state_name,
            COALESCE(i.priority, 'none') AS priority,
            COALESCE((
                SELECT string_agg(u.first_name || ' ' || u.last_name, ', ')
                FROM issue_assignees ia
                JOIN users u ON u.id = ia.assignee_id
                WHERE ia.issue_id = i.id AND ia.deleted_at IS NULL AND u.is_active
            ), '') AS assignees,
            COALESCE((
                SELECT string_agg(u.first_name || ' ' || u.last_name, ', ')
                FROM issue_subscribers isub
                JOIN users u ON u.id = isub.subscriber_id
                WHERE isub.issue_id = i.id
            ), '') AS subscribers,
            COALESCE(cb.first_name || ' ' || cb.last_name, '') AS created_by_name,
            COALESCE(i.start_date::text, '') AS start_date,
            COALESCE(i.target_date::text, '') AS target_date,
            COALESCE(to_char(i.completed_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), '') AS completed_at,
            to_char(i.created_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at,
            to_char(i.updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS updated_at,
            COALESCE(i.archived_at::text, '') AS archived_at,
            COALESCE(ep.value::text, '') AS estimate,
            COALESCE((
                SELECT string_agg(l.name, ', ')
                FROM issue_labels il
                JOIN labels l ON l.id = il.label_id
                WHERE il.issue_id = i.id AND il.deleted_at IS NULL
            ), '') AS labels,
            COALESCE((
                SELECT string_agg(c.name, ', ')
                FROM cycle_issues ic
                JOIN cycles c ON c.id = ic.cycle_id
                WHERE ic.issue_id = i.id AND ic.deleted_at IS NULL
            ), '') AS cycles,
            COALESCE((
                SELECT string_agg(m.name, ', ')
                FROM module_issues mi
                JOIN modules m ON m.id = mi.module_id
                WHERE mi.issue_id = i.id AND mi.deleted_at IS NULL
            ), '') AS modules,
            COALESCE((SELECT count(*)::text FROM issue_links ilnk WHERE ilnk.issue_id = i.id), '0') AS link_count,
            COALESCE((SELECT count(*)::text FROM issue_attachments iatt WHERE iatt.issue_id = i.id), '0') AS attachment_count,
            i.is_draft
        FROM issues i
        JOIN projects p ON p.id = i.project_id
        LEFT JOIN states s ON s.id = i.state_id
        LEFT JOIN users cb ON cb.id = i.created_by_id
        LEFT JOIN estimate_points ep ON ep.id = i.estimate_point_id
        LEFT JOIN issues parent_i ON parent_i.id = i.parent_id
        LEFT JOIN projects parent_p ON parent_p.id = parent_i.project_id
        WHERE i.workspace_id = $1
          AND i.project_id = ANY($2)
          AND i.deleted_at IS NULL
          AND p.deleted_at IS NULL
          AND p.archived_at IS NULL
          AND EXISTS (
              SELECT 1 FROM project_members pm
              WHERE pm.project_id = i.project_id AND pm.member_id = $3
                AND pm.is_active AND pm.deleted_at IS NULL
                AND (pm.role > 5 OR p.guest_view_all_features OR i.created_by_id = $3)
          )
        ORDER BY i.created_at DESC";

    #[derive(sqlx::FromRow)]
    struct IssueRow {
        project_name: String,
        project_identifier: String,
        parent_identifier: String,
        identifier: String,
        sequence_id: i32,
        issue_name: String,
        state_name: String,
        priority: String,
        assignees: String,
        subscribers: String,
        created_by_name: String,
        start_date: String,
        target_date: String,
        completed_at: String,
        created_at: String,
        updated_at: String,
        archived_at: String,
        estimate: String,
        labels: String,
        cycles: String,
        modules: String,
        link_count: String,
        attachment_count: String,
        is_draft: bool,
    }

    let rows: Result<Vec<IssueRow>, _> = sqlx::query_as(issues_query)
        .bind(ws_id)
        .bind(&project_ids)
        .bind(user_id)
        .fetch_all(&st.db)
        .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to fetch issues for export: {e}");
            let _ =
                sqlx::query("UPDATE exporters SET status = 'failed', reason = $1, updated_at = now() WHERE id = $2")
                    .bind(e.to_string())
                    .bind(exporter_id)
                    .execute(&st.db)
                    .await;
            return;
        }
    };

    let content = if is_json {
        let json_items: Vec<Value> = rows
            .into_iter()
            .map(|r| {
                json!({
                    "project_name": r.project_name,
                    "project_identifier": r.project_identifier,
                    "parent": r.parent_identifier,
                    "identifier": r.identifier,
                    "sequence_id": r.sequence_id,
                    "name": r.issue_name,
                    "state_name": r.state_name,
                    "priority": r.priority,
                    "assignees": r.assignees.split(", ").filter(|s| !s.is_empty()).collect::<Vec<_>>(),
                    "subscribers": r.subscribers.split(", ").filter(|s| !s.is_empty()).collect::<Vec<_>>(),
                    "created_by_name": r.created_by_name,
                    "start_date": r.start_date,
                    "target_date": r.target_date,
                    "completed_at": r.completed_at,
                    "created_at": r.created_at,
                    "updated_at": r.updated_at,
                    "archived_at": r.archived_at,
                    "estimate": r.estimate,
                    "labels": r.labels.split(", ").filter(|s| !s.is_empty()).collect::<Vec<_>>(),
                    "cycles": r.cycles.split(", ").filter(|s| !s.is_empty()).collect::<Vec<_>>(),
                    "modules": r.modules.split(", ").filter(|s| !s.is_empty()).collect::<Vec<_>>(),
                    "link_count": r.link_count,
                    "attachment_count": r.attachment_count,
                    "is_draft": r.is_draft,
                })
            })
            .collect();
        serde_json::to_string_pretty(&json_items).unwrap_or_default()
    } else {
        let headers = [
            "Project Name",
            "Project Identifier",
            "Parent",
            "Identifier",
            "Sequence ID",
            "Name",
            "State",
            "Priority",
            "Assignees",
            "Subscribers",
            "Created By",
            "Start Date",
            "Target Date",
            "Completed At",
            "Created At",
            "Updated At",
            "Archived At",
            "Estimate",
            "Labels",
            "Cycles",
            "Modules",
            "Link Count",
            "Attachment Count",
            "Is Draft",
        ];
        let mut csv = String::new();
        append_csv_line(&mut csv, headers.iter().map(|s| s.to_string()));
        for r in rows {
            let line = [
                r.project_name,
                r.project_identifier,
                r.parent_identifier,
                r.identifier,
                r.sequence_id.to_string(),
                r.issue_name,
                r.state_name,
                r.priority,
                r.assignees,
                r.subscribers,
                r.created_by_name,
                r.start_date,
                r.target_date,
                r.completed_at,
                r.created_at,
                r.updated_at,
                r.archived_at,
                r.estimate,
                r.labels,
                r.cycles,
                r.modules,
                r.link_count,
                r.attachment_count,
                r.is_draft.to_string(),
            ];
            append_csv_line(&mut csv, line.into_iter());
        }
        csv
    };

    // Save to STORAGE_DIR
    let export_dir = st.cfg.storage_dir.join("exports").join(&token);
    if let Err(e) = tokio::fs::create_dir_all(&export_dir).await {
        tracing::error!("Failed to create export dir: {e}");
        let _ = sqlx::query("UPDATE exporters SET status = 'failed', reason = $1, updated_at = now() WHERE id = $2")
            .bind(e.to_string())
            .bind(exporter_id)
            .execute(&st.db)
            .await;
        return;
    }

    let file_path = export_dir.join(&filename);
    if let Err(e) = tokio::fs::write(&file_path, content.as_bytes()).await {
        tracing::error!("Failed to write export file: {e}");
        let _ = sqlx::query("UPDATE exporters SET status = 'failed', reason = $1, updated_at = now() WHERE id = $2")
            .bind(e.to_string())
            .bind(exporter_id)
            .execute(&st.db)
            .await;
        return;
    }

    let download_url = format!(
        "{}/api/workspaces/{slug}/export-issues/{token}/download/",
        st.cfg.web_url
    );
    let _ =
        sqlx::query("UPDATE exporters SET status = 'completed', url = $1, key = $2, updated_at = now() WHERE id = $3")
            .bind(&download_url)
            .bind(&key)
            .bind(exporter_id)
            .execute(&st.db)
            .await;
}

async fn download_export(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, token)): Path<(String, String)>,
) -> ApiResult<Response> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;

    // Find exporter row
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT status, provider, key FROM exporters
         WHERE workspace_id = $1 AND token = $2 AND deleted_at IS NULL",
    )
    .bind(ws.workspace_id)
    .bind(&token)
    .fetch_optional(&st.db)
    .await?;

    let (status, provider, key) = row.ok_or_else(ApiError::not_found)?;
    if status != "completed" {
        return Err(ApiError::bad("Export is not ready for download"));
    }

    let disk_file = st.cfg.storage_dir.join(&key);
    let bytes = tokio::fs::read(&disk_file).await.map_err(|_| ApiError::not_found())?;

    let content_type = if provider == "json" {
        "application/json"
    } else {
        "text/csv; charset=utf-8"
    };

    let filename = disk_file.file_name().and_then(|n| n.to_str()).unwrap_or("export.csv");

    let mut res = Body::from(bytes).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap());
    let disp = format!("attachment; filename=\"{filename}\"");
    if let Ok(v) = HeaderValue::from_str(&disp) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(res)
}

fn append_csv_line(out: &mut String, fields: impl IntoIterator<Item = String>) {
    let mut first = true;
    for f in fields {
        if !first {
            out.push(',');
        }
        first = false;
        out.push('"');
        for c in f.chars() {
            if c == '"' {
                out.push('"');
                out.push('"');
            } else {
                out.push(c);
            }
        }
        out.push('"');
    }
    out.push('\r');
    out.push('\n');
}

// ============================================================================
// 4. Export Analytics
// ============================================================================

const VALID_ANALYTICS_FIELDS: &[&str] = &[
    "state_id",
    "state__group",
    "labels__id",
    "assignees__id",
    "estimate_point__value",
    "issue_cycle__cycle_id",
    "issue_module__module_id",
    "priority",
    "start_date",
    "target_date",
    "created_at",
    "completed_at",
];

const VALID_YAXIS: &[&str] = &["issue_count", "estimate"];

async fn export_analytics(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER])?;

    let x_axis = body.get("x_axis").and_then(Value::as_str).unwrap_or("");
    let y_axis = body.get("y_axis").and_then(Value::as_str).unwrap_or("");
    let segment = body.get("segment").and_then(Value::as_str);

    if !VALID_ANALYTICS_FIELDS.contains(&x_axis) || !VALID_YAXIS.contains(&y_axis) {
        return Err(ApiError::bad_body(json!({
            "error": "x-axis and y-axis dimensions are required and the values should be valid"
        })));
    }

    if let Some(seg) = segment {
        if !VALID_ANALYTICS_FIELDS.contains(&seg) || seg == x_axis {
            return Err(ApiError::bad_body(json!({
                "error": "Both segment and x axis cannot be same and segment should be valid"
            })));
        }
    }

    // Build CSV export content in background and send email
    let st_bg = st.clone();
    let email = user.email.clone();
    let slug_bg = slug.clone();
    let x_axis_s = x_axis.to_string();
    let y_axis_s = y_axis.to_string();
    let segment_s = segment.map(ToString::to_string);
    let ws_id = ws.workspace_id;
    let user_id = user.id;

    tokio::spawn(async move {
        run_analytics_export_bg(st_bg, ws_id, slug_bg, email, user_id, x_axis_s, y_axis_s, segment_s).await;
    });

    Ok(Json(json!({
        "message": format!("Once the export is ready it will be emailed to you at {}", user.email)
    })))
}

async fn run_analytics_export_bg(
    st: AppState,
    ws_id: Uuid,
    slug: String,
    email: String,
    user_id: Uuid,
    x_axis: String,
    y_axis: String,
    segment: Option<String>,
) {
    let mut csv = String::new();
    let is_estimate = y_axis == "estimate";

    if let Some(seg) = segment {
        append_csv_line(&mut csv, [x_axis.clone(), seg.clone(), y_axis.clone()]);
        // Query issues grouped by x and segment
        let sql = format!(
            "SELECT
                COALESCE({}::text, 'None') AS x_dim,
                COALESCE({}::text, 'None') AS s_dim,
                {}::bigint AS val
             FROM issues i
             WHERE i.workspace_id = $1 AND i.deleted_at IS NULL AND NOT i.is_draft
               AND EXISTS (
                   SELECT 1 FROM project_members pm
                   WHERE pm.project_id = i.project_id AND pm.member_id = $2
                     AND pm.is_active AND pm.deleted_at IS NULL
               )
             GROUP BY 1, 2
             ORDER BY 1, 2",
            analytics_field_expr(&x_axis),
            analytics_field_expr(&seg),
            if is_estimate {
                "COALESCE(sum(i.point), 0)"
            } else {
                "count(*)"
            }
        );
        if let Ok(rows) = sqlx::query_as::<_, (String, String, i64)>(&sql)
            .bind(ws_id)
            .bind(user_id)
            .fetch_all(&st.db)
            .await
        {
            for (x_val, s_val, count) in rows {
                append_csv_line(&mut csv, [x_val, s_val, count.to_string()]);
            }
        }
    } else {
        append_csv_line(&mut csv, [x_axis.clone(), y_axis.clone()]);
        let sql = format!(
            "SELECT
                COALESCE({}::text, 'None') AS x_dim,
                {}::bigint AS val
             FROM issues i
             WHERE i.workspace_id = $1 AND i.deleted_at IS NULL AND NOT i.is_draft
               AND EXISTS (
                   SELECT 1 FROM project_members pm
                   WHERE pm.project_id = i.project_id AND pm.member_id = $2
                     AND pm.is_active AND pm.deleted_at IS NULL
               )
             GROUP BY 1
             ORDER BY 1",
            analytics_field_expr(&x_axis),
            if is_estimate {
                "COALESCE(sum(i.point), 0)"
            } else {
                "count(*)"
            }
        );
        if let Ok(rows) = sqlx::query_as::<_, (String, i64)>(&sql)
            .bind(ws_id)
            .bind(user_id)
            .fetch_all(&st.db)
            .await
        {
            for (x_val, count) in rows {
                append_csv_line(&mut csv, [x_val, count.to_string()]);
            }
        }
    }

    let subject = format!("Plane Analytics Export - {slug}");
    let text = format!("Attached is your analytics export for workspace {slug}.\n\nCSV Data:\n{csv}");
    let html = format!(
        "<p>Your analytics export for workspace <b>{}</b> is ready.</p><pre>{}</pre>",
        html_escape(&slug),
        html_escape(&csv)
    );

    crate::outbound::send_mail_bg(&st, email, subject, html, text);
}

fn analytics_field_expr(field: &str) -> &'static str {
    match field {
        "state_id" => "i.state_id",
        "state__group" => "(SELECT s.group FROM states s WHERE s.id = i.state_id)",
        "priority" => "i.priority",
        "start_date" => "to_char(i.start_date, 'YYYY-MM')",
        "target_date" => "to_char(i.target_date, 'YYYY-MM')",
        "created_at" => "to_char(i.created_at, 'YYYY-MM')",
        "completed_at" => "to_char(i.completed_at, 'YYYY-MM')",
        "estimate_point__value" => "(SELECT ep.value::text FROM estimate_points ep WHERE ep.id = i.estimate_point_id)",
        "labels__id" => {
            "(SELECT il.label_id::text FROM issue_labels il WHERE il.issue_id = i.id AND il.deleted_at IS NULL LIMIT 1)"
        }
        "assignees__id" => {
            "(SELECT ia.assignee_id::text FROM issue_assignees ia WHERE ia.issue_id = i.id AND ia.deleted_at IS NULL LIMIT 1)"
        }
        "issue_cycle__cycle_id" => {
            "(SELECT ic.cycle_id::text FROM cycle_issues ic WHERE ic.issue_id = i.id AND ic.deleted_at IS NULL LIMIT 1)"
        }
        "issue_module__module_id" => {
            "(SELECT mi.module_id::text FROM module_issues mi WHERE mi.issue_id = i.id AND mi.deleted_at IS NULL LIMIT 1)"
        }
        _ => "i.id",
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ============================================================================
// 5. User Activity CSV Export
// ============================================================================

async fn user_activity_export_get(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, user_id)): Path<(String, Uuid)>,
    Query(q): Query<Q>,
) -> ApiResult<Response> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let date = q.get("date").map(String::as_str);
    serve_user_activity_csv(&st, &ws, &user, user_id, date).await
}

async fn user_activity_export_post(
    State(st): State<AppState>,
    user: CurrentUser,
    Path((slug, user_id)): Path<(String, Uuid)>,
    Json(body): Json<Value>,
) -> ApiResult<Response> {
    let ws = require_workspace(&st.db, &user, &slug).await?;
    ws.require(&[ADMIN, MEMBER, GUEST])?;

    let date_str = body
        .get("date")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad("Date is required"))?;

    serve_user_activity_csv(&st, &ws, &user, user_id, Some(date_str)).await
}

async fn serve_user_activity_csv(
    st: &AppState,
    ws: &WorkspaceCtx,
    user: &CurrentUser,
    target_user_id: Uuid,
    date_str: Option<&str>,
) -> ApiResult<Response> {
    let parsed_date = if let Some(d) = date_str {
        Some(NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| ApiError::bad("Enter a valid date."))?)
    } else {
        None
    };

    let base_query = "
        SELECT
            COALESCE(u.display_name, '') AS actor_name,
            p.identifier || ' - ' || COALESCE(i.sequence_id::text, '') AS issue_id,
            p.name AS project_name,
            to_char(a.created_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at,
            to_char(a.updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS updated_at,
            COALESCE(a.verb, '') AS action,
            COALESCE(a.field, '') AS field,
            COALESCE(a.old_value, '') AS old_value,
            COALESCE(a.new_value, '') AS new_value
        FROM issue_activities a
        JOIN users u ON u.id = a.actor_id
        JOIN projects p ON p.id = a.project_id
        LEFT JOIN issues i ON i.id = a.issue_id
        WHERE a.workspace_id = $1
          AND a.actor_id = $2
          AND a.deleted_at IS NULL
          AND a.field NOT IN ('comment', 'vote', 'reaction', 'draft')
          AND ($3::date IS NULL OR a.created_at::date = $3)
          AND EXISTS (
              SELECT 1 FROM project_members pm
              WHERE pm.project_id = a.project_id AND pm.member_id = $4
                AND pm.is_active AND pm.deleted_at IS NULL
                AND (pm.role > 5 OR p.guest_view_all_features OR (i.id IS NOT NULL AND i.created_by_id = $4))
          )
        ORDER BY a.created_at DESC
        LIMIT 10000";

    type ActivityRow = (String, String, String, String, String, String, String, String, String);

    let rows: Vec<ActivityRow> = sqlx::query_as(base_query)
        .bind(ws.workspace_id)
        .bind(target_user_id)
        .bind(parsed_date)
        .bind(user.id)
        .fetch_all(&st.db)
        .await?;

    let headers = [
        "Actor name",
        "Issue ID",
        "Project",
        "Created at",
        "Updated at",
        "Action",
        "Field",
        "Old value",
        "New value",
    ];

    let mut csv = String::new();
    append_csv_line(&mut csv, headers.iter().map(|s| s.to_string()));
    for r in rows {
        let line = [r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8];
        append_csv_line(&mut csv, line.into_iter());
    }

    let mut res = Body::from(csv).into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"workspace-user-activity.csv\""),
    );
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_line_escaping() {
        let mut out = String::new();
        append_csv_line(
            &mut out,
            vec![
                "simple".to_string(),
                "with,comma".to_string(),
                "with \"quotes\"".to_string(),
                "multi\nline".to_string(),
            ],
        );
        assert_eq!(
            out,
            "\"simple\",\"with,comma\",\"with \"\"quotes\"\"\",\"multi\nline\"\r\n"
        );
    }

    #[test]
    fn url_encoding() {
        assert_eq!(urlencoding::encode("hello world"), "hello+world");
        assert_eq!(urlencoding::encode("a=1&b=2"), "a%3D1%26b%3D2");
    }

    #[test]
    fn html_escaping() {
        assert_eq!(
            html_escape("<b>\"foo\" & 'bar'</b>"),
            "&lt;b&gt;&quot;foo&quot; &amp; 'bar'&lt;/b&gt;"
        );
    }

    #[test]
    fn cursor_parsing() {
        assert_eq!(page_of_cursor(Some(&"20:2:0".to_string())), 2);
        assert_eq!(page_of_cursor(Some(&"invalid".to_string())), 0);
        assert_eq!(page_of_cursor(None), 0);
    }

    #[test]
    fn analytics_dimensions() {
        for field in VALID_ANALYTICS_FIELDS {
            assert!(!analytics_field_expr(field).is_empty());
        }
    }
}
