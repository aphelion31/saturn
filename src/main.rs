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
mod modules;
mod notifications;
mod pages;
mod live_doc;
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
    Router, middleware,
    routing::{any, get},
};
use common::{AppState, Config};
use std::sync::Arc;

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
        .route("/api/{*rest}", any(web::not_implemented))
        .route("/auth/{*rest}", any(web::not_implemented))
        .layer(middleware::from_fn_with_state(state.clone(), web::origin_check));
    Router::new()
        .route("/health", get(web::health))
        .merge(api)
        .merge(web::spa_routes(&state))
        .layer(middleware::from_fn(web::security_headers))
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

    let addr = env("BIND", "0.0.0.0:3000");
    tracing::info!("saturn listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
