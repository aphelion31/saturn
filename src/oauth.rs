//! Plane OAuth login, including one-use server-side state.
use crate::{
    auth::{self, App},
    common::AppState,
    outbound, session,
};
use axum::{
    Router,
    extract::{OriginalUri, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    let mut router = Router::new();
    for name in ["google", "github", "gitlab", "gitea"] {
        router = router
            .route(&format!("/auth/{name}/"), get(web_start))
            .route(&format!("/auth/{name}/callback/"), get(web_callback))
            .route(&format!("/auth/spaces/{name}/"), get(space_start))
            .route(&format!("/auth/spaces/{name}/callback/"), get(space_callback));
    }
    router
}

#[derive(Deserialize, Default)]
struct Params {
    state: Option<String>,
    code: Option<String>,
    next_path: Option<String>,
}
struct Provider {
    name: String,
    id: String,
    secret: String,
    authorize: String,
    token: String,
    user: String,
    scope: String,
    org: String,
}

impl Provider {
    async fn load(st: &AppState, name: &str) -> Option<Self> {
        if !["google", "github", "gitlab", "gitea"].contains(&name) {
            return None;
        }
        let upper = name.to_uppercase();
        let id = outbound::config(st, &format!("{upper}_CLIENT_ID"), "").await;
        let secret = outbound::config(st, &format!("{upper}_CLIENT_SECRET"), "").await;
        let enabled = outbound::config(st, &format!("IS_{upper}_ENABLED"), "0").await;
        let host = match name {
            "gitlab" => outbound::config(st, "GITLAB_HOST", "https://gitlab.com").await,
            "gitea" => outbound::config(st, "GITEA_HOST", "").await,
            _ => String::new(),
        };
        if id.is_empty()
            || secret.is_empty()
            || enabled != "1"
            || (matches!(name, "gitlab" | "gitea")
                && !url::Url::parse(&host)
                    .ok()
                    .is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some()))
        {
            return None;
        }
        let host = host.trim_end_matches('/');
        let org = if name == "github" {
            outbound::config(st, "GITHUB_ORGANIZATION_ID", "").await
        } else {
            String::new()
        };
        let (authorize, token, user, scope) = match name {
            "google" => (
                "https://accounts.google.com/o/oauth2/v2/auth".into(),
                "https://oauth2.googleapis.com/token".into(),
                "https://www.googleapis.com/oauth2/v2/userinfo".into(),
                "https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile"
                    .into(),
            ),
            "github" => (
                "https://github.com/login/oauth/authorize".into(),
                "https://github.com/login/oauth/access_token".into(),
                "https://api.github.com/user".into(),
                if org.is_empty() {
                    "read:user user:email".into()
                } else {
                    "read:user user:email read:org".into()
                },
            ),
            "gitlab" => (
                format!("{host}/oauth/authorize"),
                format!("{host}/oauth/token"),
                format!("{host}/api/v4/user"),
                "read_user".into(),
            ),
            _ => (
                format!("{host}/login/oauth/authorize"),
                format!("{host}/login/oauth/access_token"),
                format!("{host}/api/v1/user"),
                "openid email profile read:user".into(),
            ),
        };
        Some(Self {
            name: name.into(),
            id,
            secret,
            authorize,
            token,
            user,
            scope,
            org,
        })
    }
}
pub async fn enabled(st: &AppState, name: &str) -> bool {
    Provider::load(st, name).await.is_some()
}
fn error(st: &AppState, app: App, next: &str, name: &str, code: u32) -> Response {
    let target = auth::redirect_url(
        st,
        app,
        next,
        &[("error_code", code.to_string()), ("error_message", name.into())],
    );
    (StatusCode::FOUND, [(header::LOCATION, target)]).into_response()
}
fn codes(name: &str, configured: bool) -> (String, u32) {
    let n = name.to_uppercase();
    let c = match (name, configured) {
        ("google", false) => 5105,
        ("github", false) => 5110,
        ("gitlab", false) => 5111,
        ("gitea", false) => 5112,
        ("google", true) => 5115,
        ("github", true) => 5120,
        ("gitlab", true) => 5121,
        ("gitea", true) => 5123,
        _ => 5104,
    };
    (
        if c == 5104 {
            "OAUTH_NOT_CONFIGURED".into()
        } else if configured {
            format!("{n}_OAUTH_PROVIDER_ERROR")
        } else {
            format!("{n}_NOT_CONFIGURED")
        },
        c,
    )
}
fn callback_uri(st: &AppState, app: App, provider: &str) -> String {
    format!(
        "{}/auth/{}{provider}/callback/",
        st.cfg.web_url.trim_end_matches('/'),
        if app == App::Space { "spaces/" } else { "" }
    )
}
async fn start(st: AppState, name: String, app: App, q: Params) -> Response {
    let next = auth::safe_next(
        q.next_path
            .as_deref()
            .filter(|p| p.starts_with('/') && !p.starts_with("//")),
    );
    let Some(p) = Provider::load(&st, &name).await else {
        let (n, c) = codes(&name, false);
        return error(&st, app, &next, &n, c);
    };
    let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let key = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let data = session::encode(
        &st.cfg.secret_key,
        &json!({"state": state, "provider":name, "space":app == App::Space, "next":next}),
    );
    if sqlx::query(
        "INSERT INTO sessions (session_key, session_data, expire_date) VALUES ($1,$2,now() + interval '10 minutes')",
    )
    .bind(&key)
    .bind(data)
    .execute(&st.db)
    .await
    .is_err()
    {
        let (n, c) = codes(&name, true);
        return error(&st, app, "", &n, c);
    }
    let mut url = match url::Url::parse(&p.authorize) {
        Ok(u) => u,
        Err(_) => {
            let (n, c) = codes(&name, false);
            return error(&st, app, "", &n, c);
        }
    };
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("client_id", &p.id)
            .append_pair("scope", &p.scope)
            .append_pair("redirect_uri", &callback_uri(&st, app, &name))
            .append_pair("state", &state);
        if name != "github" {
            pairs.append_pair("response_type", "code");
        }
        if name == "google" {
            pairs
                .append_pair("access_type", "offline")
                .append_pair("prompt", "consent");
        }
    }
    (
        StatusCode::FOUND,
        [
            (header::LOCATION, url.to_string()),
            (
                header::SET_COOKIE,
                session::set_cookie(&st, "oauth-state", Some(&key), 600),
            ),
        ],
    )
        .into_response()
}
fn provider(uri: &axum::http::Uri) -> String {
    uri.path()
        .split('/')
        .find(|p| ["google", "github", "gitlab", "gitea"].contains(p))
        .unwrap_or("")
        .to_owned()
}
async fn web_start(State(st): State<AppState>, OriginalUri(uri): OriginalUri, Query(q): Query<Params>) -> Response {
    start(st, provider(&uri), App::Web, q).await
}
async fn space_start(State(st): State<AppState>, OriginalUri(uri): OriginalUri, Query(q): Query<Params>) -> Response {
    start(st, provider(&uri), App::Space, q).await
}
async fn web_callback(
    State(st): State<AppState>,
    OriginalUri(uri): OriginalUri,
    Query(q): Query<Params>,
    h: HeaderMap,
) -> Response {
    callback(st, provider(&uri), App::Web, q, h).await
}
async fn space_callback(
    State(st): State<AppState>,
    OriginalUri(uri): OriginalUri,
    Query(q): Query<Params>,
    h: HeaderMap,
) -> Response {
    callback(st, provider(&uri), App::Space, q, h).await
}

async fn callback(st: AppState, name: String, app: App, q: Params, h: HeaderMap) -> Response {
    let (err, ec) = codes(&name, true);
    let data: Option<String> = if let Some(key) = session::cookie(&h, "oauth-state") {
        sqlx::query_scalar("DELETE FROM sessions WHERE session_key=$1 AND expire_date > now() AND user_id IS NULL RETURNING session_data").bind(key).fetch_optional(&st.db).await.ok().flatten()
    } else {
        None
    };
    let data = data
        .and_then(|d| session::decode(&st.cfg.secret_key, &d))
        .unwrap_or_default();
    let next = auth::safe_next(data["next"].as_str());
    if data["provider"].as_str() != Some(&name)
        || data["space"].as_bool() != Some(app == App::Space)
        || q.state.as_deref().filter(|s| !s.is_empty()) != data["state"].as_str()
        || q.code.as_deref().is_none_or(str::is_empty)
    {
        return error(&st, app, &next, &err, ec);
    }
    let Some(p) = Provider::load(&st, &name).await else {
        let (n, c) = codes(&name, false);
        return error(&st, app, &next, &n, c);
    };
    match complete(&st, &p, app, &q.code.unwrap_or_default(), &next, &h).await {
        Ok(response) => response,
        Err((n, c)) => error(&st, app, &next, &n, c),
    }
}
async fn complete(
    st: &AppState,
    p: &Provider,
    app: App,
    code: &str,
    next: &str,
    h: &HeaderMap,
) -> Result<Response, (String, u32)> {
    let fail = || codes(&p.name, true);
    let redirect = callback_uri(st, app, &p.name);
    let token: Value = outbound::http()
        .post(&p.token)
        .header(header::ACCEPT, "application/json")
        .form(&[
            ("client_id", p.id.as_str()),
            ("client_secret", p.secret.as_str()),
            ("code", code),
            ("redirect_uri", &redirect),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|_| fail())?
        .error_for_status()
        .map_err(|_| fail())?
        .json()
        .await
        .map_err(|_| fail())?;
    let access = token["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(fail)?;
    let get = |url: String| {
        outbound::http()
            .get(url)
            .bearer_auth(access)
            .header(header::ACCEPT, "application/json")
    };
    let user: Value = get(p.user.clone())
        .send()
        .await
        .map_err(|_| fail())?
        .error_for_status()
        .map_err(|_| fail())?
        .json()
        .await
        .map_err(|_| fail())?;
    if p.name == "github" && !p.org.is_empty() {
        let login = user["login"].as_str().ok_or_else(fail)?;
        let mut url = url::Url::parse("https://api.github.com/orgs/").map_err(|_| fail())?;
        url.path_segments_mut()
            .map_err(|_| fail())?
            .push(&p.org)
            .push("memberships")
            .push(login);
        if !get(url.to_string())
            .send()
            .await
            .map_err(|_| fail())?
            .status()
            .is_success()
        {
            return Err(("GITHUB_USER_NOT_IN_ORG".into(), 5122));
        }
    }
    let unverified = || ("OAUTH_PROVIDER_UNVERIFIED_EMAIL".to_string(), 5124);
    let email = if p.name == "github" || p.name == "gitea" {
        let emails: Vec<Value> = get(format!("{}/emails", p.user))
            .send()
            .await
            .map_err(|_| fail())?
            .error_for_status()
            .map_err(|_| fail())?
            .json()
            .await
            .map_err(|_| fail())?;
        emails
            .iter()
            .find(|e| e["verified"] == true && e["primary"] == true)
            .or_else(|| {
                if p.name == "gitea" {
                    emails.iter().find(|e| e["verified"] == true)
                } else {
                    None
                }
            })
            .and_then(|e| e["email"].as_str())
            .ok_or_else(unverified)?
            .to_owned()
    } else {
        if (p.name == "google" && user["verified_email"] != true)
            || (p.name == "gitlab" && user["confirmed_at"].as_str().is_none_or(str::is_empty))
        {
            return Err(unverified());
        }
        user["email"].as_str().ok_or_else(unverified)?.to_owned()
    }
    .trim()
    .to_lowercase();
    if !email.contains('@') || email.len() > 255 {
        return Err(("INVALID_EMAIL".into(), 5005));
    }
    let provider_id = match &user["id"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(fail()),
    };
    let existing: Option<(Uuid, bool, bool, bool)> = sqlx::query_as(
        "SELECT id,is_active,is_bot,last_logout_time IS NOT NULL FROM users WHERE lower(email)=$1 LIMIT 1",
    )
    .bind(&email)
    .fetch_optional(&st.db)
    .await
    .map_err(|_| fail())?;
    let id = if let Some((id, active, bot, logged_out)) = existing {
        if bot {
            return Err(("BOT_USER_LOGIN_FORBIDDEN".into(), 5017));
        }
        if !active && logged_out {
            return Err(("USER_ACCOUNT_DEACTIVATED".into(), 5019));
        }
        id
    } else {
        if outbound::config(st, "ENABLE_SIGNUP", "1").await == "0" {
            let invited: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM workspace_member_invites WHERE lower(email)=$1 AND deleted_at IS NULL)",
            )
            .bind(&email)
            .fetch_one(&st.db)
            .await
            .map_err(|_| fail())?;
            if !invited {
                return Err(("SIGNUP_DISABLED".into(), 5015));
            }
        }
        let first = user[if p.name == "google" {
            "given_name"
        } else if p.name == "gitea" {
            "full_name"
        } else {
            "name"
        }]
        .as_str()
        .unwrap_or("");
        let last = user["family_name"].as_str().unwrap_or("");
        let id = auth::create_user(&st.db, &email, &Uuid::new_v4().to_string(), first, last)
            .await
            .map_err(|_| fail())?;
        sqlx::query("UPDATE users SET is_password_autoset=true,is_email_verified=true WHERE id=$1")
            .bind(id)
            .execute(&st.db)
            .await
            .map_err(|_| fail())?;
        id
    };
    sqlx::query("UPDATE users SET is_active=true,last_login_medium=$2 WHERE id=$1")
        .bind(id)
        .bind(&p.name)
        .execute(&st.db)
        .await
        .map_err(|_| fail())?;
    // Plane's account tokens are encrypted at rest; do not retain provider secrets in logs.
    let a = outbound::encrypt(&st.cfg.secret_key, access);
    let r = token["refresh_token"]
        .as_str()
        .map(|s| outbound::encrypt(&st.cfg.secret_key, s));
    let it = outbound::encrypt(&st.cfg.secret_key, token["id_token"].as_str().unwrap_or(""));
    let updated = sqlx::query("UPDATE accounts SET access_token=$4,refresh_token=$5,id_token=$6,last_connected_at=now(),updated_at=now() WHERE user_id=$1 AND provider=$2 AND provider_account_id=$3").bind(id).bind(&p.name).bind(&provider_id).bind(&a).bind(&r).bind(&it).execute(&st.db).await.map_err(|_|fail())?;
    if updated.rows_affected() == 0 {
        sqlx::query("INSERT INTO accounts (user_id,provider,provider_account_id,access_token,refresh_token,id_token) VALUES ($1,$2,$3,$4,$5,$6)").bind(id).bind(&p.name).bind(&provider_id).bind(&a).bind(&r).bind(&it).execute(&st.db).await.map_err(|_|fail())?;
    }
    if app == App::Web {
        auth::join_accepted_invites(st, id, &email).await.map_err(|_| fail())?;
    }
    let key = session::create(st, id, false, auth::device_info(h, st, app))
        .await
        .map_err(|_| fail())?;
    let path = if !next.is_empty() {
        next.to_string()
    } else if app == App::Web {
        auth::landing(st, id, &email).await.map_err(|_| fail())?
    } else {
        String::new()
    };
    let target = format!(
        "{}/{}",
        auth::base(st, app).trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    Ok((
        StatusCode::FOUND,
        [
            (header::LOCATION, target),
            (
                header::SET_COOKIE,
                session::set_cookie(st, session::COOKIE, Some(&key), session::AGE_SECS),
            ),
        ],
    )
        .into_response())
}
