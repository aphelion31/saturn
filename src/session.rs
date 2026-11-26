//! Django-compatible sessions and passwords, so an existing Plane database keeps its logins.
//!
//! Plane stores sessions in the `sessions` table (custom Django session model) and identifies the
//! browser with a `session-id` cookie (`admin-session-id` for god-mode). `session_data` is
//! `django.core.signing.dumps(dict, salt="django.contrib.sessions.SessionStore", compress=True)`.
//! Passwords default to Argon2id (`argon2$argon2id$v=19$m=19456,t=2,p=1$...`, OWASP) and
//! still verify legacy Django `pbkdf2_sha256$<iterations>$<salt>$<base64 hash>` rows so existing
//! Plane databases keep their logins and upgrade transparently on sign in.
use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version, password_hash::SaltString,
};
use axum::http::{HeaderMap, StatusCode};
use base64::{Engine, engine::general_purpose::STANDARD as B64, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
use hmac::{Hmac, Mac};
use rand::{Rng, RngCore, rngs::OsRng};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use uuid::Uuid;

use crate::common::{ApiError, AppState};

pub const COOKIE: &str = "session-id";
pub const ADMIN_COOKIE: &str = "admin-session-id";
pub const AGE_SECS: i64 = 604_800; // SESSION_COOKIE_AGE
pub const ADMIN_AGE_SECS: i64 = 3_600; // ADMIN_SESSION_COOKIE_AGE
const SALT: &str = "django.contrib.sessions.SessionStore";
const KEY_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const B62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ARGON2_M_COST: u32 = 19_456;
const ARGON2_T_COST: u32 = 2;
const ARGON2_P_COST: u32 = 1;
const ARGON2_CURRENT_PREFIX: &str = "argon2$argon2id$v=19$m=19456,t=2,p=1$";
const DUMMY_ARGON2_HASH: &str =
    "argon2$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// Read one cookie value from the request headers.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all("cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            (k == name && !v.is_empty()).then(|| v.to_string())
        })
}

/// `Set-Cookie` value that stores `key`, or clears the cookie when `key` is None.
pub fn set_cookie(st: &AppState, name: &str, key: Option<&str>, max_age: i64) -> String {
    let secure = if st.cfg.cookie_secure { "; Secure" } else { "" };
    match key {
        Some(k) => format!("{name}={k}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}"),
        None => format!("{name}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax{secure}"),
    }
}

/// Look up a live session and return its user id. Also checks `_auth_user_hash`, so changing a
/// password signs out every other session, as Django does.
pub async fn load(st: &AppState, key: &str) -> Result<Option<Uuid>, ApiError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT session_data FROM sessions WHERE session_key = $1 AND expire_date > now()")
            .bind(key)
            .fetch_optional(&st.db)
            .await?;
    let Some((data,)) = row else { return Ok(None) };
    let Some(v) = decode(&st.cfg.secret_key, &data) else {
        return Ok(None);
    };
    let Some(uid) = v
        .get("_auth_user_id")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
    else {
        return Ok(None);
    };
    if let Some(h) = v.get("_auth_user_hash").and_then(Value::as_str) {
        let pw: Option<(String,)> = sqlx::query_as("SELECT password FROM users WHERE id = $1")
            .bind(uid)
            .fetch_optional(&st.db)
            .await?;
        if pw.map(|(p,)| auth_hash(&st.cfg.secret_key, &p)) != Some(h.to_string()) {
            return Ok(None);
        }
    }
    Ok(Some(uid))
}

/// Create a session row for `user_id` and return its key (the cookie value).
pub async fn create(st: &AppState, user_id: Uuid, admin: bool, device_info: Value) -> Result<String, ApiError> {
    let pw: (String,) = sqlx::query_as("SELECT password FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&st.db)
        .await?;
    let mut data = json!({
        "_auth_user_id": user_id.to_string(),
        "_auth_user_backend": "django.contrib.auth.backends.ModelBackend",
        "_auth_user_hash": auth_hash(&st.cfg.secret_key, &pw.0),
        "device_info": device_info,
    });
    if admin {
        data["_session_expiry"] = json!(ADMIN_AGE_SECS);
    }
    let key = random_key();
    let age = if admin { ADMIN_AGE_SECS } else { AGE_SECS };
    sqlx::query(
        "INSERT INTO sessions (session_key, session_data, expire_date, user_id, device_info)
         VALUES ($1, $2, now() + make_interval(secs => $3), $4, $5)",
    )
    .bind(&key)
    .bind(encode(&st.cfg.secret_key, &data))
    .bind(age as f64)
    .bind(user_id.to_string())
    .bind(&data["device_info"])
    .execute(&st.db)
    .await?;
    sqlx::query("UPDATE users SET last_login = now(), last_login_time = now(), last_active = now() WHERE id = $1")
        .bind(user_id)
        .execute(&st.db)
        .await?;
    Ok(key)
}

pub async fn destroy(st: &AppState, key: &str) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM sessions WHERE session_key = $1")
        .bind(key)
        .execute(&st.db)
        .await?;
    Ok(())
}

fn random_key() -> String {
    let mut r = rand::thread_rng();
    (0..128)
        .map(|_| KEY_CHARS[r.gen_range(0..KEY_CHARS.len())] as char)
        .collect()
}

/// Django `salted_hmac(key_salt, value, secret)`: HMAC-SHA256 keyed by sha256(key_salt + secret).
fn salted_hmac(key_salt: &str, value: &[u8], secret: &str) -> Vec<u8> {
    let key = Sha256::digest(format!("{key_salt}{secret}").as_bytes());
    let mut m = Hmac::<Sha256>::new_from_slice(&key).expect("hmac key");
    m.update(value);
    m.finalize().into_bytes().to_vec()
}

/// `AbstractBaseUser.get_session_auth_hash()`.
fn auth_hash(secret: &str, password: &str) -> String {
    hex::encode(salted_hmac(
        "django.contrib.auth.models.AbstractBaseUser.get_session_auth_hash",
        password.as_bytes(),
        secret,
    ))
}

fn b62(mut n: u64) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(B62[(n % 62) as usize]);
        n /= 62;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// `signing.dumps(value, salt=SALT, compress=True)` with a TimestampSigner.
pub fn encode(secret: &str, value: &Value) -> String {
    let raw = serde_json::to_vec(value).unwrap();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).unwrap();
    let compressed = z.finish().unwrap();
    let payload = if compressed.len() < raw.len().saturating_sub(1) {
        format!(".{}", B64URL.encode(compressed))
    } else {
        B64URL.encode(raw)
    };
    let ts = b62(chrono::Utc::now().timestamp() as u64);
    let signed = format!("{payload}:{ts}");
    let sig = B64URL.encode(salted_hmac(&format!("{SALT}signer"), signed.as_bytes(), secret));
    format!("{signed}:{sig}")
}

/// Inverse of `encode`. Returns None when the signature does not match `secret`.
pub fn decode(secret: &str, data: &str) -> Option<Value> {
    let (signed, sig) = data.rsplit_once(':')?;
    let want = B64URL.encode(salted_hmac(&format!("{SALT}signer"), signed.as_bytes(), secret));
    if !constant_eq(want.as_bytes(), sig.as_bytes()) {
        return None;
    }
    let (payload, _ts) = signed.rsplit_once(':')?;
    let (compressed, b) = match payload.strip_prefix('.') {
        Some(rest) => (true, rest),
        None => (false, payload),
    };
    let mut bytes = B64URL.decode(b.trim_end_matches('=')).ok()?;
    if compressed {
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(&bytes[..]).read_to_end(&mut out).ok()?;
        bytes = out;
    }
    serde_json::from_slice(&bytes).ok()
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn argon2id() -> Argon2<'static> {
    let params = Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32)).expect("valid argon2id params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Hash a password with Argon2id (OWASP parameters, prefixed `argon2$...` for Django compatibility
/// and `users.password varchar(128)` storage).
pub fn hash_password(password: &str) -> String {
    let mut salt_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes).expect("16-byte salt base64");
    let phc = argon2id()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2id hash")
        .to_string();
    format!("argon2{phc}")
}

/// Check a password against Argon2id (`argon2$argon2id$...` or raw `$argon2id$...`) or legacy
/// Django `pbkdf2_sha256` hashes. Unusable (`!...`) and unknown hashers return false.
pub fn check_password(password: &str, encoded: &str) -> bool {
    if let Some(phc) = encoded.strip_prefix("argon2").filter(|s| s.starts_with("$argon2")) {
        return check_argon2id(password, phc);
    }
    if encoded.starts_with("$argon2") {
        return check_argon2id(password, encoded);
    }
    check_pbkdf2_sha256(password, encoded)
}

fn check_argon2id(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    parsed.algorithm.as_str() == "argon2id" && argon2id().verify_password(password.as_bytes(), &parsed).is_ok()
}

fn check_pbkdf2_sha256(password: &str, encoded: &str) -> bool {
    let mut parts = encoded.splitn(4, '$');
    let (Some("pbkdf2_sha256"), Some(iter), Some(salt), Some(hash)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let Ok(iter) = iter.parse::<u32>() else { return false };
    let Ok(want) = B64.decode(hash) else { return false };
    let mut out = vec![0u8; want.len()];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt.as_bytes(), iter, &mut out);
    constant_eq(&out, &want)
}

/// True when a stored hash is legacy (`pbkdf2_sha256`) or uses outdated Argon2id parameters and
/// should be transparently upgraded to current Argon2id on login.
pub fn needs_rehash(encoded: &str) -> bool {
    !encoded.starts_with(ARGON2_CURRENT_PREFIX)
}

/// Hash a password off the Tokio async worker pool.
pub async fn hash_password_async(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": format!("password hash task: {e}")}),
            )
        })
}

/// Verify a password off the Tokio async worker pool.
pub async fn check_password_async(password: String, encoded: String) -> Result<bool, ApiError> {
    tokio::task::spawn_blocking(move || check_password(&password, &encoded))
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": format!("password verify task: {e}")}),
            )
        })
}

/// Perform constant-time Argon2id work when an account lookup misses, avoiding email enumeration
/// timing side-channels without blocking the Tokio runtime.
pub async fn verify_dummy_async() {
    let _ = tokio::task::spawn_blocking(|| {
        let _ = check_password("dummy-password", DUMMY_ARGON2_HASH);
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = json!({"_auth_user_id": "abc", "device_info": {"user_agent": "x".repeat(200)}});
        let enc = encode("s3cret", &v);
        assert_eq!(decode("s3cret", &enc), Some(v));
        assert_eq!(decode("other", &enc), None);
    }

    #[test]
    fn decodes_django_session() {
        // Produced by Django 5.2: signing.dumps({"_auth_user_id": "1"}, salt="django.contrib.sessions.SessionStore",
        // compress=True, key="x").
        let d = "eyJfYXV0aF91c2VyX2lkIjoiMSJ9:1xQolH:UvaNfdbSwPNgEY4hhoybjYwWQgmHIAn-iR9lG_PpdjM";
        assert_eq!(
            decode("x", d).and_then(|v| v["_auth_user_id"].as_str().map(String::from)),
            Some("1".into())
        );
    }

    #[test]
    fn django_password() {
        // Django 5.2: PBKDF2PasswordHasher().encode("hunter2", "abc", 1000)
        let h = "pbkdf2_sha256$1000$abc$d2O18KKBjiNyWKxf7lHXttSlasKRO44Zytmt9CqZ21c=";
        assert!(check_password("hunter2", h));
        assert!(!check_password("nope", h));
        assert!(!check_password("x", "!unusable"));
        assert!(needs_rehash(h));
    }

    #[tokio::test]
    async fn argon2id_password_roundtrip() {
        let h = hash_password_async("hunter2".into()).await.expect("hash");
        assert!(h.starts_with(ARGON2_CURRENT_PREFIX));
        assert!(
            h.len() <= 128,
            "argon2id hash must fit users.password varchar(128): {}",
            h.len()
        );
        assert!(!needs_rehash(&h));
        assert!(check_password_async("hunter2".into(), h.clone()).await.expect("check"));
        assert!(!check_password_async("nope".into(), h.clone()).await.expect("check"));
        let raw_phc = h.strip_prefix("argon2").expect("argon2 prefix");
        assert!(check_password("hunter2", raw_phc));
        assert!(needs_rehash(raw_phc));
        assert!(needs_rehash("argon2$argon2id$v=19$m=4096,t=1,p=1$abc$def"));
        assert!(PasswordHash::new(DUMMY_ARGON2_HASH.strip_prefix("argon2").expect("prefix")).is_ok());
        assert!(!check_password("any", DUMMY_ARGON2_HASH));
        verify_dummy_async().await;
    }

    #[test]
    fn django_auth_hash() {
        // salted_hmac("...get_session_auth_hash", "pw", secret="x", algorithm="sha256").hexdigest()
        assert_eq!(
            auth_hash("x", "pw"),
            "3980b987111709c6588c468dfc20fc84ac40d78f1095e4a7db71d7c3ad75a250"
        );
    }
}
