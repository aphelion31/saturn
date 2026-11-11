//! Shared outbound plumbing: instance configuration (with Plane-compatible Fernet encryption), SMTP mail and an
//! HTTP client for third-party APIs (OAuth providers, Unsplash, LLMs). Used by auth, instance, ai and exports.
use crate::common::AppState;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::header::ContentType};
use std::sync::OnceLock;

/// Plane `derive_key`: PBKDF2-HMAC-SHA256(SECRET_KEY, b"salt", 100000), urlsafe base64.
fn fernet(secret: &str) -> fernet::Fernet {
    static KEY: OnceLock<(String, String)> = OnceLock::new();
    let (s, key) = KEY.get_or_init(|| {
        let mut dk = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(secret.as_bytes(), b"salt", 100_000, &mut dk);
        (
            secret.to_string(),
            base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE, dk),
        )
    });
    let key = if s == secret {
        key.clone()
    } else {
        let mut dk = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(secret.as_bytes(), b"salt", 100_000, &mut dk);
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE, dk)
    };
    fernet::Fernet::new(&key).expect("32-byte key")
}

/// Plane `encrypt_data`. Empty stays empty.
pub fn encrypt(secret: &str, plain: &str) -> String {
    if plain.is_empty() {
        String::new()
    } else {
        fernet(secret).encrypt(plain.as_bytes())
    }
}

/// Plane `decrypt_data`. Bad input decrypts to empty, as in Plane.
pub fn decrypt(secret: &str, token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    fernet(secret)
        .decrypt(token)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}

/// Instance configuration value: database first (decrypted when `is_encrypted`), then env var, then default.
pub async fn config(st: &AppState, key: &str, default: &str) -> String {
    let row: Option<(Option<String>, bool)> =
        sqlx::query_as("SELECT value, is_encrypted FROM instance_configurations WHERE key = $1 AND deleted_at IS NULL")
            .bind(key)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();
    match row {
        Some((Some(v), true)) if !v.is_empty() => decrypt(&st.cfg.secret_key, &v),
        Some((Some(v), false)) if !v.is_empty() => v,
        _ => std::env::var(key).unwrap_or_else(|_| default.to_string()),
    }
}

/// Shared HTTP client with sane timeouts (Plane #9981: OAuth requests without timeouts hang sign in).
pub fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("saturn")
            .build()
            .expect("http client")
    })
}

pub struct Smtp {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub from: String,
    pub tls: bool,
    pub ssl: bool,
}

/// Plane `get_email_configuration`. None when SMTP is not configured.
pub async fn smtp(st: &AppState) -> Option<Smtp> {
    let host = config(st, "EMAIL_HOST", "").await;
    if host.is_empty() {
        return None;
    }
    let port = config(st, "EMAIL_PORT", "587").await.parse().ok()?;
    Some(Smtp {
        host,
        port,
        user: config(st, "EMAIL_HOST_USER", "").await,
        password: config(st, "EMAIL_HOST_PASSWORD", "").await,
        from: config(st, "EMAIL_FROM", "Team Plane <team@mailer.plane.so>").await,
        tls: config(st, "EMAIL_USE_TLS", "1").await == "1",
        ssl: config(st, "EMAIL_USE_SSL", "0").await == "1",
    })
}

/// Send one email. Errors are returned as text so callers can surface them (test email) or log them.
pub async fn send_mail(s: &Smtp, to: &str, subject: &str, html: &str, text: &str) -> Result<(), String> {
    let msg = Message::builder()
        .from(s.from.parse().map_err(|e| format!("invalid from address: {e}"))?)
        .to(to.parse().map_err(|e| format!("invalid to address: {e}"))?)
        .subject(subject)
        .multipart(
            lettre::message::MultiPart::alternative()
                .singlepart(
                    lettre::message::SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(text.to_string()),
                )
                .singlepart(
                    lettre::message::SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(html.to_string()),
                ),
        )
        .map_err(|e| e.to_string())?;
    let builder = if s.ssl {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&s.host).map_err(|e| e.to_string())?
    } else if s.tls {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&s.host).map_err(|e| e.to_string())?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&s.host)
    };
    let mut builder = builder.port(s.port).timeout(Some(std::time::Duration::from_secs(10)));
    if !s.user.is_empty() {
        builder = builder.credentials(lettre::transport::smtp::authentication::Credentials::new(
            s.user.clone(),
            s.password.clone(),
        ));
    }
    builder.build().send(msg).await.map(|_| ()).map_err(|e| e.to_string())
}

/// Fire-and-forget email used by flows that must not fail when mail is down (magic code, reset, invites).
/// The body is never logged: it carries login codes and reset links. Set SATURN_DEV_LOG_TOKENS=1 for local dev.
pub fn send_mail_bg(st: &AppState, to: String, subject: String, html: String, text: String) {
    let st = st.clone();
    tokio::spawn(async move {
        match smtp(&st).await {
            Some(s) => {
                if let Err(e) = send_mail(&s, &to, &subject, &html, &text).await {
                    tracing::warn!(%to, error = %e, "email send failed");
                }
            }
            None => tracing::info!(%to, "SMTP not configured, email not sent"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fernet_roundtrip_and_bad_token() {
        let t = encrypt("secret", "hunter2");
        assert_ne!(t, "hunter2");
        assert_eq!(decrypt("secret", &t), "hunter2");
        assert_eq!(decrypt("other", &t), "");
        assert_eq!(decrypt("secret", "garbage"), "");
        assert_eq!(encrypt("secret", ""), "");
    }

    /// Token produced by Plane's Python: Fernet(derive_key("plane-secret")).encrypt(b"plane-value").
    #[test]
    fn decrypts_plane_token() {
        let t = include_str!("../tools/fixtures/plane_fernet_token.txt").trim();
        assert_eq!(decrypt("plane-secret", t), "plane-value");
    }
}
