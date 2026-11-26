mod activity;
mod api_log;
mod api_v1;
mod api_v1_assets;
mod api_v1_extra;
mod auth;
mod common;
mod instance;
mod jobs;
mod jobs_fk;
mod notify;
mod oauth;
mod outbound;
mod session;
mod side_effects;
mod web;
mod webhook_out;
mod workitems;

// API areas. Each module owns its routes (full paths, trailing slash, as Plane's apps call them).
mod analytics;
mod api_v1_items;
mod assets;
mod cycles;
mod external;
mod issues;
mod live_doc;
mod live_http;
mod modules;
mod notifications;
mod pages;
mod projects;
mod public;
mod recent_visits;
mod states;
mod user_dash;
mod users;
mod views;
mod workspace_seed;
mod workspaces;

use axum::{
    Router,
    http::{HeaderName, HeaderValue, Method, header},
    middleware,
    routing::{any, get},
};
use common::{AppState, Config};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

fn plane_cors_origins() -> Vec<String> {
    let explicit: Vec<String> = std::env::var("CORS_ALLOWED_ORIGINS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if !explicit.is_empty() {
        return explicit;
    }
    let mut origins = Vec::new();
    for var in [
        "WEB_URL",
        "APP_BASE_URL",
        "ADMIN_BASE_URL",
        "SPACE_BASE_URL",
        "LIVE_BASE_URL",
    ] {
        let Ok(raw) = std::env::var(var) else { continue };
        let trimmed = raw.trim();
        let Ok(url) = url::Url::parse(trimmed) else { continue };
        if let Some(host) = url.host_str() {
            let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
            let origin = format!("{}://{host}{port}", url.scheme()).to_ascii_lowercase();
            if !origins.contains(&origin) {
                origins.push(origin);
            }
        }
    }
    origins
}

fn cors_layer() -> CorsLayer {
    let origins = plane_cors_origins();
    let methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ];
    let headers = [
        header::ACCEPT,
        header::ACCEPT_ENCODING,
        header::AUTHORIZATION,
        header::CONTENT_TYPE,
        header::ORIGIN,
        header::USER_AGENT,
        HeaderName::from_static("x-api-key"),
        HeaderName::from_static("x-csrftoken"),
        HeaderName::from_static("x-requested-with"),
    ];
    let base = CorsLayer::new().allow_methods(methods).allow_headers(headers);
    let parsed: Vec<HeaderValue> = origins.iter().filter_map(|o| HeaderValue::from_str(o).ok()).collect();
    if !parsed.is_empty() {
        base.allow_origin(parsed).allow_credentials(true)
    } else {
        base.allow_origin(Any)
    }
}

const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_schema", include_str!("../migrations/0001_schema.sql")),
    ("0002_defaults", include_str!("../migrations/0002_defaults.sql")),
    ("0003_auth_tokens", include_str!("../migrations/0003_auth_tokens.sql")),
];

pub fn app(state: AppState) -> Router {
    // Public v1 API: X-Api-Key requests are logged to api_activity_logs (Plane APITokenLogMiddleware).
    let v1 = Router::new()
        .merge(api_v1::routes())
        .merge(api_v1_assets::routes())
        .merge(api_v1_extra::routes())
        .merge(api_v1_items::routes())
        .layer(middleware::from_fn_with_state(state.clone(), api_log::log_api_requests));
    let api = Router::new()
        .merge(v1)
        .merge(auth::routes())
        .merge(oauth::routes())
        .merge(instance::routes())
        .merge(users::routes())
        .merge(workspaces::routes())
        .merge(projects::routes())
        .merge(states::routes())
        .merge(issues::routes())
        .merge(cycles::routes())
        .merge(modules::routes())
        .merge(views::routes())
        .merge(pages::routes())
        .merge(notifications::routes())
        .merge(assets::routes())
        .merge(public::routes())
        .merge(analytics::routes())
        .merge(user_dash::routes())
        .merge(external::routes())
        .merge(live_http::routes())
        .route("/api/{*rest}", any(web::api_not_found))
        .route("/auth/{*rest}", any(web::not_implemented))
        .layer(middleware::from_fn_with_state(state.clone(), web::origin_check));
    Router::new()
        .route("/health", get(web::health))
        .route("/ready", get(web::ready))
        .route("/robots.txt", get(web::robots_txt))
        .merge(api)
        .merge(web::spa_routes(&state))
        .layer(middleware::from_fn(web::security_headers))
        .layer(cors_layer())
        .layer(middleware::from_fn(web::peer_header))
        .layer(tower_http::trace::TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
            tracing::info_span!("http", method = %request.method(), path = %request.uri().path())
        }))
        .with_state(state)
}

/// Apply migrations in order, once each. On an existing Plane database (tables already created by
/// Django) the schema migration is recorded as applied and only Saturn's additions run.
async fn migrate(db: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS saturn_migrations (name text PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())")
        .execute(db)
        .await?;
    let plane_db: (bool,) = sqlx::query_as("SELECT to_regclass('public.issues') IS NOT NULL")
        .fetch_one(db)
        .await?;
    if plane_db.0 {
        sqlx::query("INSERT INTO saturn_migrations (name) VALUES ('0001_schema') ON CONFLICT DO NOTHING")
            .execute(db)
            .await?;
    }
    for (name, sql) in MIGRATIONS {
        let done: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM saturn_migrations WHERE name = $1)")
            .bind(name)
            .fetch_one(db)
            .await?;
        if done.0 {
            continue;
        }
        tracing::info!("applying migration {name}");
        let mut tx = db.begin().await?;
        sqlx::raw_sql(sql).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO saturn_migrations (name) VALUES ($1)")
            .bind(name)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    auth::ensure_instance(db).await?;
    instance::ensure_configurations(db).await
}

fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(env("RUST_LOG", "info,tower_http=warn"))
        .init();
    let url = env("DATABASE_URL", "postgres://localhost/saturn");
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(env("DB_POOL", "10").parse()?)
        .connect(&url)
        .await?;
    migrate(&db).await?;

    let web_url = env("WEB_URL", "http://localhost:3000");
    let secret_key = std::env::var("SECRET_KEY").unwrap_or_else(|_| {
        tracing::warn!("SECRET_KEY not set, using an insecure development key");
        "saturn-insecure-dev-key".into()
    });
    let cfg = Config {
        cookie_secure: web_url.starts_with("https://"),
        web_url,
        secret_key,
        web_dir: env("WEB_DIR", "web/apps").into(),
        storage_dir: env("STORAGE_DIR", "storage").into(),
        file_size_limit: env("FILE_SIZE_LIMIT", "5242880").parse()?,
    };
    let state = AppState { db, cfg: Arc::new(cfg) };

    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("migrate") => return Ok(()),
        Some("seed") => return users::seed(&state, &args[2..]).await,
        Some("jobs") if args.get(2).map(String::as_str) == Some("run") => {
            return jobs::cli_run(&state, &args[3..]).await;
        }
        _ => {}
    }

    jobs::spawn(state.clone());

    let shutdown_state = state.clone();
    let addr = env("BIND", "0.0.0.0:3000");
    tracing::info!("saturn listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    pages::flush_all_rooms(&shutdown_state).await;
    shutdown_state.db.close().await;
    tracing::info!("saturn shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            sig.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received, draining connections");
}
