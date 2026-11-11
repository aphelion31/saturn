//! Static apps, health, security headers, origin check and the 501 fallback for unported routes.
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::net::SocketAddr;
use tower_http::services::{ServeDir, ServeFile};

use crate::common::AppState;

/// Internal header carrying the TCP peer IP. Always overwritten from the socket, so clients cannot set it.
pub const PEER_HEADER: &str = "x-saturn-peer";

pub async fn peer_header(mut req: Request<Body>, next: Next) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .and_then(|c| HeaderValue::from_str(&c.0.ip().to_string()).ok());
    let h = req.headers_mut();
    h.remove(PEER_HEADER);
    if let Some(v) = peer {
        h.insert(PEER_HEADER, v);
    }
    next.run(req).await
}

/// The three Plane React Router apps, built as static SPAs (see scripts/build-web.sh):
/// `web` at `/`, `admin` (god-mode) at `/god-mode`, `space` (public boards) at `/spaces`.
pub fn spa_routes(st: &AppState) -> Router<AppState> {
    let dir = |app: &str| st.cfg.web_dir.join(app).join("build/client");
    let spa = |app: &str| ServeDir::new(dir(app)).fallback(ServeFile::new(dir(app).join("index.html")));
    Router::new()
        .nest_service("/god-mode", spa("admin"))
        .nest_service("/spaces", spa("space"))
        .fallback_service(spa("web"))
}

pub async fn health() -> Json<serde_json::Value> {
    Json(json!({"status": "OK"}))
}

/// Any `/api/...` route Saturn does not implement yet. Logged so gaps are visible.
pub async fn not_implemented(req: Request) -> Response {
    tracing::warn!(method = %req.method(), path = %req.uri().path(), "unported endpoint");
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": "Not implemented in Saturn"})),
    )
        .into_response()
}

/// Cookie-authenticated writes must come from Saturn's own origin, the same rule as Plane's
/// `BaseSessionAuthentication.enforce_csrf`. Requests without an Origin header (API clients
/// using `X-Api-Key`, curl) pass.
pub async fn origin_check(State(st): State<AppState>, req: Request, next: Next) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    if let Some(origin) = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let origin_host = origin.split("://").nth(1).unwrap_or("");
        let web_host = st.cfg.web_url.split("://").nth(1).unwrap_or("").trim_end_matches('/');
        let ok = !origin_host.is_empty()
            && (origin_host.eq_ignore_ascii_case(host) || origin_host.eq_ignore_ascii_case(web_host));
        if !ok {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"detail": "CSRF Failed: origin not allowed"})),
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Headers Plane's apps set through Next/React Router `headers()` exports and Django's middleware.
pub async fn security_headers(req: Request<Body>, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    for (k, v) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "origin-when-cross-origin"),
        ("x-frame-options", "SAMEORIGIN"),
    ] {
        h.entry(k).or_insert(HeaderValue::from_static(v));
    }
    res
}
