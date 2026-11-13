//! Port of Plane's `APITokenLogMiddleware` + `process_logs`. Requests to the public `/api/v1` router that carry
//! `X-Api-Key` are recorded in `api_activity_logs`. The key is never stored: `token_identifier` is a keyed
//! HMAC-SHA256 of the key, and sensitive headers are redacted. The insert runs in a spawned task so the
//! response is not delayed. Bodies are capped so one large request cannot bloat the table.

use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, header},
    middleware::Next,
    response::Response,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::common::AppState;

const MAX_BODY: usize = 64 * 1024;
const SENSITIVE: [&str; 3] = ["x-api-key", "authorization", "cookie"];

pub async fn log_api_requests(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let Some(key) = req.headers().get("x-api-key").and_then(|v| v.to_str().ok()).map(str::to_owned) else {
        return next.run(req).await;
    };
    let method = req.method().to_string();
    let path = req.uri().path().to_owned();
    let query = req.uri().query().unwrap_or("").to_owned();
    let headers = redact(req.headers());
    let user_agent = req.headers().get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(|s| s.chars().take(512).collect::<String>());
    let ip = req
        .headers()
        .get("x-saturn-peer")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    // Buffer the body so it can be logged and still forwarded.
    let (parts, body) = req.into_parts();
    let bytes = to_bytes(body, MAX_BODY).await.unwrap_or_default();
    let body_text = decode(&bytes);
    let req = Request::from_parts(parts, Body::from(bytes));

    let resp = next.run(req).await;
    let status = i32::from(resp.status().as_u16());
    let (rparts, rbody) = resp.into_parts();
    let rbytes = to_bytes(rbody, MAX_BODY).await.unwrap_or_default();
    let response_body = decode(&rbytes);
    let resp = Response::from_parts(rparts, Body::from(rbytes));

    let token_identifier = token_id(&st.cfg.secret_key, &key);
    let db = st.db.clone();
    tokio::spawn(async move {
        let res = sqlx::query(
            "INSERT INTO api_activity_logs (id, created_at, updated_at, token_identifier, path, method, query_params,
                                            headers, body, response_code, response_body, ip_address, user_agent)
             VALUES (gen_random_uuid(), now(), now(), $1, $2, $3, $4, $5, $6, $7, $8, $9::inet, $10)",
        )
        .bind(token_identifier)
        .bind(path.chars().take(255).collect::<String>())
        .bind(method)
        .bind(query)
        .bind(headers)
        .bind(body_text)
        .bind(status)
        .bind(response_body)
        .bind(ip)
        .bind(user_agent)
        .execute(&db)
        .await;
        if let Err(e) = res {
            tracing::warn!(error = %e, "api activity log insert failed");
        }
    });
    resp
}

fn token_id(secret: &str, key: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(key.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn redact(headers: &HeaderMap) -> String {
    let mut out = String::new();
    for (name, value) in headers {
        let v = if SENSITIVE.contains(&name.as_str()) { "[REDACTED]" } else { value.to_str().unwrap_or("[binary]") };
        out.push_str(&format!("{}: {}\n", name, v));
    }
    out
}

fn decode(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    if bytes.starts_with(b"\x89PNG") || bytes.starts_with(b"\xff\xd8\xff") || bytes.starts_with(b"%PDF") {
        return Some("[Binary Content]".into());
    }
    Some(String::from_utf8(bytes.to_vec()).unwrap_or_else(|_| "[Could not decode content]".into()))
}
