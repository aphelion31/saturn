//! Outbound webhooks. DNS is pinned to the addresses validated immediately before each attempt.
use crate::common::AppState;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::time::Duration;
use uuid::Uuid;

fn signature(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

pub(crate) async fn pinned_client(raw: &str, timeout: Duration) -> Result<reqwest::Client, String> {
    let url = url::Url::parse(raw).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return Err("invalid URL".into());
    }
    let host = url.host_str().ok_or("missing host")?;
    let addresses: Vec<_> = tokio::net::lookup_host((host, url.port_or_known_default().ok_or("missing port")?))
        .await
        .map_err(|e| e.to_string())?
        .collect();
    if addresses.is_empty()
        || (std::env::var("SATURN_WEBHOOK_ALLOW_PRIVATE").as_deref() != Ok("1")
            && addresses.iter().any(|a| crate::workspaces::blocked_ip(a.ip())))
    {
        return Err("disallowed address".into());
    }
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|e| e.to_string())
}

pub(crate) async fn model_data(
    db: &sqlx::PgPool,
    event: &str,
    id: Uuid,
    workspace_id: Uuid,
) -> Result<Value, sqlx::Error> {
    let sql: String = match event {
        "issue" => "SELECT to_jsonb(i) || jsonb_build_object('state', (SELECT jsonb_build_object('id',s.id,'name',s.name,'color',s.color,'group',s.\"group\") FROM states s WHERE s.id=i.state_id AND s.workspace_id=i.workspace_id AND s.deleted_at IS NULL), 'description',i.description_json,'labels',COALESCE((SELECT jsonb_agg(il.label_id) FROM issue_labels il WHERE il.issue_id=i.id AND il.workspace_id=i.workspace_id AND il.deleted_at IS NULL),'[]'::jsonb),'assignees',COALESCE((SELECT jsonb_agg(ia.assignee_id) FROM issue_assignees ia WHERE ia.issue_id=i.id AND ia.workspace_id=i.workspace_id AND ia.deleted_at IS NULL),'[]'::jsonb)) FROM issues i WHERE i.id=$1 AND i.workspace_id=$2 AND i.deleted_at IS NULL".into(),
        "issue_comment" => "SELECT to_jsonb(c) - 'comment_stripped' - 'comment_json' FROM issue_comments c WHERE c.id=$1 AND c.workspace_id=$2 AND c.deleted_at IS NULL".into(),
        "project" => "SELECT to_jsonb(p) || jsonb_build_object('cover_image_url',CASE WHEN p.cover_image_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || p.cover_image_asset_id || '/' ELSE NULLIF(p.cover_image,'') END) FROM projects p WHERE p.id=$1 AND p.workspace_id=$2 AND p.deleted_at IS NULL".into(),
        "module" => "SELECT to_jsonb(m) || jsonb_build_object('members',COALESCE((SELECT jsonb_agg(mm.member_id) FROM module_members mm WHERE mm.module_id=m.id AND mm.workspace_id=m.workspace_id AND mm.deleted_at IS NULL),'[]'::jsonb)) FROM modules m WHERE m.id=$1 AND m.workspace_id=$2 AND m.deleted_at IS NULL".into(),
        "cycle" => "SELECT to_jsonb(c) FROM cycles c WHERE c.id=$1 AND c.workspace_id=$2 AND c.deleted_at IS NULL".into(),
        _ => return Err(sqlx::Error::RowNotFound),
    };
    let mut data: Value = sqlx::query_scalar(&sql)
        .bind(id)
        .bind(workspace_id)
        .fetch_one(db)
        .await?;
    normalize(event, &mut data);
    Ok(data)
}

fn normalize(event: &str, data: &mut Value) {
    let Some(obj) = data.as_object_mut() else { return };
    for key in [
        "created_by",
        "updated_by",
        "workspace",
        "project",
        "parent",
        "estimate_point",
        "type",
        "actor",
        "issue",
        "description",
        "owned_by",
        "lead",
        "default_assignee",
        "project_lead",
        "estimate",
        "default_state",
        "cover_image_asset",
    ] {
        let column = format!("{key}_id");
        if let Some(value) = obj.remove(&column) {
            obj.insert(key.into(), value);
        }
    }
    if event == "issue" {
        obj.remove("state_id");
    }
}

pub fn changes(before: &Value, body: &Value) -> Vec<(String, Value, Value)> {
    body.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, new)| {
            before
                .get(key)
                .filter(|old| *old != new)
                .map(|old| (key.clone(), old.clone(), new.clone()))
        })
        .collect()
}

pub fn emit(st: &AppState, workspace_id: Uuid, event: &str, action: &str, payload: Value, actor: Uuid) {
    emit_with_changes(st, workspace_id, event, action, payload, actor, vec![]);
}

pub fn emit_with_changes(
    st: &AppState,
    workspace_id: Uuid,
    event: &str,
    action: &str,
    payload: Value,
    actor: Uuid,
    changes: Vec<(String, Value, Value)>,
) {
    let (st, event, action) = (st.clone(), event.to_owned(), action.to_owned());
    tokio::spawn(async move {
        let rows: Result<Vec<(Uuid, String, String, String)>, _> = sqlx::query_as(
            "SELECT h.id, h.url, h.secret_key, w.slug FROM webhooks h JOIN workspaces w ON w.id = h.workspace_id WHERE h.workspace_id = $1 AND h.deleted_at IS NULL AND w.deleted_at IS NULL AND h.is_active AND CASE $2 WHEN 'project' THEN h.project WHEN 'issue' THEN h.issue WHEN 'module' THEN h.module WHEN 'cycle' THEN h.cycle WHEN 'issue_comment' THEN h.issue_comment ELSE false END")
            .bind(workspace_id).bind(&event).fetch_all(&st.db).await;
        let rows = match rows {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error=%e, "webhook lookup failed");
                return;
            }
        };
        let data = if action == "deleted" {
            payload.clone()
        } else {
            let Some(id) = payload
                .get("id")
                .and_then(Value::as_str)
                .and_then(|v| Uuid::parse_str(v).ok())
            else {
                tracing::warn!(%event, "webhook event missing id");
                return;
            };
            match model_data(&st.db, &event, id, workspace_id).await {
                Ok(data) => data,
                Err(e) => {
                    tracing::warn!(%event, error=%e, "webhook serialization failed");
                    return;
                }
            }
        };
        let actor: Value = match sqlx::query_scalar("SELECT jsonb_build_object('id',u.id,'first_name',u.first_name,'last_name',u.last_name,'email',u.email,'avatar',u.avatar,'avatar_url',CASE WHEN u.avatar_asset_id IS NOT NULL THEN '/api/assets/v2/static/' || u.avatar_asset_id || '/' ELSE NULLIF(u.avatar,'') END,'display_name',u.display_name) FROM users u WHERE u.id=$1").bind(actor).fetch_one(&st.db).await { Ok(v) => v, Err(e) => { tracing::warn!(error=%e, "webhook actor lookup failed"); return; } };
        let activities: Vec<Value> = if action == "updated" {
            changes.into_iter().map(|(field, old, new)| json!({"field":field,"old_value":old,"new_value":new,"actor":actor,"old_identifier":null,"new_identifier":null})).collect()
        } else {
            vec![
                json!({"field":null,"old_value":null,"new_value":null,"actor":actor,"old_identifier":null,"new_identifier":null}),
            ]
        };
        for (id, url, secret, slug) in rows {
            for activity in &activities {
                let body = json!({"event":event,"action":action,"webhook_id":id,"workspace_id":workspace_id,"workspace_slug":slug,"data":data,"activity":activity}).to_string();
                for retry in 0..=5i16 {
                    let delivery = Uuid::new_v4().to_string();
                    let sig = signature(&secret, body.as_bytes());
                    let headers = json!({"Content-Type":"application/json","User-Agent":"Autopilot","X-Plane-Delivery":delivery,"X-Plane-Event":event,"X-Plane-Signature":sig}).to_string();
                    let resolved = pinned_client(&url, Duration::from_secs(30)).await;
                    let rejected = resolved.is_err();
                    let result = match resolved {
                        Ok(client) => client
                            .post(&url)
                            .header("Content-Type", "application/json")
                            .header("User-Agent", "Autopilot")
                            .header("X-Plane-Delivery", &delivery)
                            .header("X-Plane-Event", &event)
                            .header("X-Plane-Signature", &sig)
                            .body(body.clone())
                            .send()
                            .await
                            .map_err(|e| e.to_string()),
                        Err(e) => Err(e),
                    };
                    let failed = result.is_err();
                    let (status, response_headers, response_body) = match result {
                        Ok(r) => {
                            let status = r.status().as_u16().to_string();
                            let headers = format!("{:?}", r.headers());
                            (status, headers, r.text().await.unwrap_or_default())
                        }
                        Err(e) => (if rejected { "400" } else { "500" }.to_string(), String::new(), e),
                    };
                    if let Err(e) = sqlx::query("INSERT INTO webhook_logs (id, workspace_id, webhook, event_type, request_method, request_headers, request_body, response_status, response_headers, response_body, retry_count) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
                    .bind(Uuid::new_v4()).bind(workspace_id).bind(id).bind(&event).bind(&action).bind(&headers).bind(&body).bind(status).bind(response_headers).bind(response_body).bind(retry).execute(&st.db).await { tracing::warn!(error=%e, "webhook log failed"); }
                    if !failed || rejected {
                        break;
                    }
                    if retry == 5 {
                        match sqlx::query_scalar::<_, Option<String>>("UPDATE webhooks h SET is_active=false WHERE h.id=$1 AND h.workspace_id=$2 AND h.deleted_at IS NULL RETURNING (SELECT email FROM users WHERE id=h.created_by_id)").bind(id).bind(workspace_id).fetch_optional(&st.db).await {
                        Ok(Some(Some(email))) => { let text = format!("Webhook {url} has been deactivated due to failed requests. {}/{slug}/settings/webhooks/{id}", st.cfg.web_url.trim_end_matches('/')); crate::outbound::send_mail_bg(&st, email, "Webhook Deactivated".into(), ammonia::clean(&text), text); }
                        Ok(_) => (), Err(e) => tracing::warn!(error=%e, "webhook deactivation failed"),
                    }
                        break;
                    }
                    let cap = 600u64 * (1 << retry);
                    let delay = rand::random::<u64>() % (cap + 1);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn issue_fixture_shape_and_changes() {
        let fixture: Value = serde_json::from_str(include_str!("../tests/fixtures/plane_webhook_issue.json")).unwrap();
        let expected = &fixture[0]["data"];
        let mut row = expected.clone();
        let obj = row.as_object_mut().unwrap();
        for key in [
            "created_by",
            "updated_by",
            "workspace",
            "project",
            "parent",
            "estimate_point",
            "type",
        ] {
            let value = obj.remove(key).unwrap();
            obj.insert(format!("{key}_id"), value);
        }
        obj.insert("state_id".into(), json!("state-id"));
        normalize("issue", &mut row);
        assert_eq!(row, *expected);
        assert_eq!(
            changes(&json!({"priority":"none"}), &json!({"priority":"high", "unknown":1})),
            vec![("priority".into(), json!("none"), json!("high"))]
        );
    }
    #[test]
    fn hmac_sha256() {
        assert_eq!(
            signature("key", b"The quick brown fox jumps over the lazy dog"),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }
    #[test]
    fn private_addresses() {
        for ip in ["127.0.0.1", "10.0.0.1", "169.254.1.1", "::1", "::ffff:127.0.0.1"] {
            assert!(crate::workspaces::blocked_ip(ip.parse::<std::net::IpAddr>().unwrap()));
        }
        assert!(!crate::workspaces::blocked_ip("8.8.8.8".parse().unwrap()));
    }
}
