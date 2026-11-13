//! Background side effects ported from Plane's Celery tasks:
//! - `crawl_work_item_link_title` (`plane/bgtasks/work_item_link_task.py`)
//! - `get_asset_object_metadata` (`plane/bgtasks/storage_metadata_task.py`)
//! - `copy_s3_objects_of_description_and_assets` (`plane/bgtasks/copy_s3_object.py`)
//! - `page_transaction` (`plane/bgtasks/page_transaction_task.py`)

use crate::common::{AppState, strip_tags};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use uuid::Uuid;

const DEFAULT_FAVICON: &str = "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIyNCIgaGVpZ2h0PSIyNCIgdmlld0JveD0iMCAwIDI0IDI0IiBmaWxsPSJub25lIiBzdHJva2U9ImN1cnJlbnRDb2xvciIgc3Ryb2tlLXdpZHRoPSIyIiBzdHJva2UtbGluZWNhcD0icm91bmQiIHN0cm9rZS1saW5lam9pbj0icm91bmQiIGNsYXNzPSJsdWNpZGUgbHVjaWRlLWxpbmstaWNvbiBsdWNpZGUtbGluayI+PHBhdGggZD0iTTEwIDEzYTUgNSAwIDAgMCA3LjU0LjU0bDMtM2E1IDUgMCAwIDAtNy4wNy03LjA3bC0xLjcyIDEuNzEiLz48cGF0aCBkPSJNMTQgMTFhNSA1IDAgMCAwLTcuNTQtLjU0bC0zIDNhNSA1IDAgMCAwIDcuMDcgNy4wN2wxLjcxLTEuNzEiLz48L3N2Zz4=";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";
const MAX_REDIRECTS: usize = 5;
const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
const MAX_FAVICON_BYTES: usize = 512 * 1024;
const CRAWL_TIMEOUT: Duration = Duration::from_secs(5);

async fn pinned_request(
    method: reqwest::Method,
    raw_url: &str,
    max_bytes: usize,
) -> Result<(u16, Option<String>, Vec<u8>, String), String> {
    let mut current = raw_url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let client = crate::webhook_out::pinned_client(&current, CRAWL_TIMEOUT).await?;
        let mut resp = client
            .request(method.clone(), &current)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        if status.is_redirection() {
            let Some(loc) = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()) else {
                return Err("redirect missing location".into());
            };
            let base = url::Url::parse(&current).map_err(|e| e.to_string())?;
            let next = base.join(loc).map_err(|e| e.to_string())?;
            current = next.to_string();
            continue;
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            let take = chunk.len().min(max_bytes.saturating_sub(body.len()));
            body.extend_from_slice(&chunk[..take]);
            if body.len() >= max_bytes {
                break;
            }
        }
        return Ok((status.as_u16(), content_type, body, current));
    }
    Err("too many redirects".into())
}

fn decode_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        if let Some(semi) = rest[1..].find(';').filter(|&i| i < 12) {
            let entity = &rest[1..1 + semi];
            let decoded = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ if entity.starts_with("#x") || entity.starts_with("#X") => u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32),
                _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            if let Some(ch) = decoded {
                out.push(ch);
                rest = &rest[2 + semi..];
                continue;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

/// Parse `<title>` text and `<link rel=... href=...>` attributes from an HTML document.
fn parse_title_and_icons(html: &str) -> (Option<String>, Vec<(String, String)>) {
    let mut title: Option<String> = None;
    let mut links: Vec<(String, String)> = Vec::new();
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let Some(rel_lt) = lower[i..].find('<') else {
            break;
        };
        let pos = i + rel_lt;
        if lower[pos..].starts_with("<!--") {
            i = lower[pos + 4..].find("-->").map_or(bytes.len(), |end| pos + 4 + end + 3);
            continue;
        }
        if lower[pos..].starts_with("<script") {
            i = lower[pos + 7..].find("</script").map_or(bytes.len(), |end| pos + 7 + end + 8);
            continue;
        }
        if lower[pos..].starts_with("<style") {
            i = lower[pos + 6..].find("</style").map_or(bytes.len(), |end| pos + 6 + end + 7);
            continue;
        }

        if title.is_none() && lower[pos..].starts_with("<title") && matches!(bytes.get(pos + 6), Some(b'>' | b' ' | b'\t' | b'\r' | b'\n')) {
            if let Some(gt) = html[pos..].find('>') {
                let start = pos + gt + 1;
                if let Some(end_rel) = lower[start..].find("</title") {
                    let raw = decode_html_entities(&html[start..start + end_rel]);
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        title = Some(trimmed.to_string());
                    }
                    i = start + end_rel + 8;
                    continue;
                }
            }
        }

        if lower[pos..].starts_with("<link") && matches!(bytes.get(pos + 5), Some(b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>')) {
            if let Some(gt) = find_tag_end(html, pos) {
                let attrs = parse_tag_attrs(&html[pos + 5..gt]);
                let mut rel_val = None;
                let mut href_val = None;
                for (k, v) in attrs {
                    if k.eq_ignore_ascii_case("rel") {
                        rel_val = Some(v.trim().to_ascii_lowercase());
                    } else if k.eq_ignore_ascii_case("href") {
                        href_val = Some(decode_html_entities(v.trim()));
                    }
                }
                if let (Some(r), Some(h)) = (rel_val, href_val) {
                    if !h.is_empty() {
                        links.push((r, h));
                    }
                }
                i = gt + 1;
                continue;
            }
        }

        i = pos + 1;
    }

    (title, links)
}

fn find_tag_end(html: &str, start: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut i = start;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(i),
            None => {}
        }
        i += 1;
    }
    None
}

fn parse_tag_attrs(raw: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let name_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' && bytes[i] != b'/' {
            i += 1;
        }
        let name = raw[name_start..i].to_string();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let val = if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                i += 1;
                let vstart = i;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                let v = raw[vstart..i].to_string();
                if i < bytes.len() {
                    i += 1;
                }
                v
            } else {
                let vstart = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                raw[vstart..i].to_string()
            };
            attrs.push((name, val));
        } else if !name.is_empty() {
            attrs.push((name, String::new()));
        }
    }
    attrs
}

async fn find_favicon_url(links: &[(String, String)], base_url: &str) -> Option<String> {
    let base = url::Url::parse(base_url).ok()?;
    for rel in ["icon", "shortcut icon", "apple-touch-icon", "apple-touch-icon-precomposed"] {
        if let Some((_, href)) = links.iter().find(|(r, _)| r == rel) {
            if let Ok(joined) = base.join(href) {
                let candidate = joined.to_string();
                if crate::webhook_out::pinned_client(&candidate, CRAWL_TIMEOUT).await.is_ok() {
                    return Some(candidate);
                }
            }
        }
    }
    let fallback = format!(
        "{}://{}{}/favicon.ico",
        base.scheme(),
        base.host_str()?,
        base.port().map(|p| format!(":{p}")).unwrap_or_default()
    );
    if let Ok((status, _, _, _)) = pinned_request(reqwest::Method::HEAD, &fallback, 0).await {
        if status == 200 {
            return Some(fallback);
        }
    }
    None
}

async fn crawl_link_metadata(url: &str) -> Value {
    let default_favicon = format!("data:image/svg+xml;base64,{DEFAULT_FAVICON}");
    let mut title: Option<String> = None;
    let mut links: Vec<(String, String)> = Vec::new();
    let mut final_url = url.to_string();

    match pinned_request(reqwest::Method::GET, url, MAX_HTML_BYTES).await {
        Ok((_status, _ct, body, resolved_url)) => {
            final_url = resolved_url;
            let html = String::from_utf8_lossy(&body);
            let (t, l) = parse_title_and_icons(&html);
            title = t;
            links = l;
        }
        Err(e) => {
            tracing::warn!(%url, error = %e, "failed to crawl work item link");
        }
    }

    let mut favicon_url: Option<String> = None;
    let mut favicon_data = default_favicon;
    if let Some(fav_url) = find_favicon_url(&links, &final_url).await {
        if let Ok((status, ct, bytes, _)) = pinned_request(reqwest::Method::GET, &fav_url, MAX_FAVICON_BYTES).await {
            if status == 200 && !bytes.is_empty() {
                let content_type = ct.unwrap_or_else(|| "image/x-icon".into());
                favicon_data = format!("data:{content_type};base64,{}", STANDARD.encode(&bytes));
                favicon_url = Some(fav_url);
            }
        }
    }

    json!({
        "title": title,
        "favicon": favicon_data,
        "url": url,
        "favicon_url": favicon_url,
    })
}

/// Port of Plane's `crawl_work_item_link_title`.
pub fn spawn_crawl_work_item_link_title(st: AppState, workspace_id: Uuid, link_id: Uuid, url: String) {
    tokio::spawn(async move {
        let metadata = crawl_link_metadata(&url).await;
        if let Err(e) = sqlx::query(
            "UPDATE issue_links SET metadata = $1, updated_at = now()
             WHERE id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
        )
        .bind(metadata)
        .bind(link_id)
        .bind(workspace_id)
        .execute(&st.db)
        .await
        {
            tracing::warn!(%link_id, error = %e, "failed to save crawled link metadata");
        }
    });
}

/// Port of Plane's `get_asset_object_metadata`.
pub fn spawn_asset_storage_metadata(st: AppState, asset_id: Uuid) {
    tokio::spawn(async move {
        let row: Option<(String, Value)> = match sqlx::query_as(
            "SELECT asset, attributes FROM file_assets WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(asset_id)
        .fetch_optional(&st.db)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(%asset_id, error = %e, "storage metadata lookup failed");
                return;
            }
        };
        let Some((key, attrs)) = row else {
            return;
        };
        let Ok(path) = crate::assets::disk_path(&st, &key) else {
            return;
        };
        let meta = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!(%asset_id, error = %e, "storage metadata stat failed");
                return;
            }
        };
        let content_type = attrs.get("type").and_then(Value::as_str);
        let last_modified = meta
            .modified()
            .ok()
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
        let storage_metadata = json!({
            "ContentType": content_type,
            "ContentLength": meta.len(),
            "LastModified": last_modified,
            "ETag": Value::Null,
            "Metadata": {},
        });
        if let Err(e) = sqlx::query(
            "UPDATE file_assets SET storage_metadata = $2, updated_at = now() WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(asset_id)
        .bind(storage_metadata)
        .execute(&st.db)
        .await
        {
            tracing::warn!(%asset_id, error = %e, "failed to update file_assets.storage_metadata");
        }
    });
}

/// Extract `src` UUIDs from `<image-component src="...">` tags.
fn extract_image_component_asset_ids(html: &str) -> Vec<Uuid> {
    let mut ids = Vec::new();
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let Some(rel) = lower[i..].find("<image-component") else {
            break;
        };
        let pos = i + rel;
        if matches!(bytes.get(pos + 16), Some(b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>')) {
            if let Some(gt) = find_tag_end(html, pos) {
                for (k, v) in parse_tag_attrs(&html[pos + 16..gt]) {
                    if k.eq_ignore_ascii_case("src") {
                        if let Ok(uid) = Uuid::parse_str(v.trim()) {
                            ids.push(uid);
                        }
                    }
                }
                i = gt + 1;
                continue;
            }
        }
        i = pos + 1;
    }
    ids
}

/// Rewrite `src` attributes on `<image-component>` tags for duplicated assets.
fn replace_image_component_srcs(html: &str, replacements: &[(String, String)]) -> String {
    if replacements.is_empty() {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut last = 0;
    let mut i = 0;
    while i < bytes.len() {
        let Some(rel) = lower[i..].find("<image-component") else {
            break;
        };
        let pos = i + rel;
        if matches!(bytes.get(pos + 16), Some(b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>')) {
            if let Some(gt) = find_tag_end(html, pos) {
                let mut tag_slice = html[pos..=gt].to_string();
                for (old_id, new_id) in replacements {
                    for quote in ['"', '\''] {
                        let pat = format!("src={quote}{old_id}{quote}");
                        if tag_slice.contains(&pat) {
                            let rep = format!("src={quote}{new_id}{quote}");
                            tag_slice = tag_slice.replace(&pat, &rep);
                        }
                    }
                }
                out.push_str(&html[last..pos]);
                out.push_str(&tag_slice);
                last = gt + 1;
                i = gt + 1;
                continue;
            }
        }
        i = pos + 1;
    }
    out.push_str(&html[last..]);
    out
}

#[derive(sqlx::FromRow)]
struct AssetRow {
    id: Uuid,
    attributes: Value,
    asset: String,
    size: f64,
    entity_type: Option<String>,
    storage_metadata: Option<Value>,
}

/// Port of Plane's `copy_s3_objects_of_description_and_assets` for duplicated pages.
/// Note: Plane calls the Node live server `/convert-document` to regenerate `description_binary` and
/// `description_json`. Saturn has no Node runtime, so `description_binary` is cleared to `NULL` and
/// the frontend rebuilds the Yjs document from `description_html`.
pub fn spawn_copy_page_assets(
    st: AppState,
    workspace_id: Uuid,
    project_id: Uuid,
    page_id: Uuid,
    user_id: Uuid,
) {
    tokio::spawn(async move {
        let html: Option<String> = match sqlx::query_scalar(
            "SELECT description_html FROM pages WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
        )
        .bind(page_id)
        .bind(workspace_id)
        .fetch_optional(&st.db)
        .await
        {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(%page_id, error = %e, "copy_page_assets failed to load page");
                return;
            }
        };
        let Some(html) = html else {
            return;
        };
        let asset_ids = extract_image_component_asset_ids(&html);
        if asset_ids.is_empty() {
            return;
        }
        let originals: Vec<AssetRow> = match sqlx::query_as(
            "SELECT id, attributes, asset, size, entity_type, storage_metadata
             FROM file_assets
             WHERE workspace_id = $1 AND project_id = $2 AND id = ANY($3) AND deleted_at IS NULL",
        )
        .bind(workspace_id)
        .bind(project_id)
        .bind(&asset_ids)
        .fetch_all(&st.db)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(%page_id, error = %e, "copy_page_assets failed to query assets");
                return;
            }
        };
        if originals.is_empty() {
            return;
        }

        let mut replacements: Vec<(String, String)> = Vec::new();
        for orig in originals {
            let new_id = Uuid::new_v4();
            let name = crate::assets::sanitize_filename(
                orig.attributes.get("name").and_then(Value::as_str).unwrap_or("unnamed"),
            );
            let new_key = format!("{}/{}-{}", workspace_id, new_id.simple(), name);
            if let (Ok(src), Ok(dst)) = (
                crate::assets::disk_path(&st, &orig.asset),
                crate::assets::disk_path(&st, &new_key),
            ) {
                if let Some(parent) = dst.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                if let Err(e) = tokio::fs::copy(&src, &dst).await {
                    tracing::warn!(old_asset = %orig.id, error = %e, "asset file copy failed; continuing with DB row");
                }
            }
            let et = orig.entity_type.as_deref().unwrap_or("PAGE_DESCRIPTION");
            let page_col = if et == "PAGE_DESCRIPTION" { Some(page_id) } else { None };
            let attrs = json!({
                "name": orig.attributes.get("name"),
                "type": orig.attributes.get("type"),
                "size": orig.attributes.get("size"),
            });
            if let Err(e) = sqlx::query(
                "INSERT INTO file_assets
                 (id, attributes, asset, size, workspace_id, created_by_id, updated_by_id, entity_type, project_id, page_id, storage_metadata, is_uploaded)
                 VALUES ($1, $2, $3, $4, $5, $6, $6, $7, $8, $9, $10, true)",
            )
            .bind(new_id)
            .bind(attrs)
            .bind(&new_key)
            .bind(orig.size)
            .bind(workspace_id)
            .bind(user_id)
            .bind(et)
            .bind(project_id)
            .bind(page_col)
            .bind(orig.storage_metadata)
            .execute(&st.db)
            .await
            {
                tracing::warn!(old_asset = %orig.id, error = %e, "failed to insert duplicated file_asset");
                continue;
            }
            replacements.push((orig.id.to_string(), new_id.to_string()));
        }

        if replacements.is_empty() {
            return;
        }
        let updated_html = replace_image_component_srcs(&html, &replacements);
        let stripped = strip_tags(&updated_html);
        if let Err(e) = sqlx::query(
            "UPDATE pages SET description_html = $1, description_stripped = $2, description_binary = NULL, updated_at = now()
             WHERE id = $3 AND workspace_id = $4 AND deleted_at IS NULL",
        )
        .bind(&updated_html)
        .bind(&stripped)
        .bind(page_id)
        .bind(workspace_id)
        .execute(&st.db)
        .await
        {
            tracing::warn!(%page_id, error = %e, "failed to update duplicated page description_html");
        }
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExtractedComponent {
    transaction_id: Uuid,
    entity_identifier: Option<Uuid>,
    entity_name: String,
    entity_type: Option<String>,
}

fn extract_page_components(html: &str) -> Vec<ExtractedComponent> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let Some(rel) = lower[i..].find('<') else {
            break;
        };
        let pos = i + rel;
        for (tag_prefix, is_image) in [("<mention-component", false), ("<image-component", true)] {
            let len = tag_prefix.len();
            if lower[pos..].starts_with(tag_prefix)
                && matches!(bytes.get(pos + len), Some(b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>'))
            {
                if let Some(gt) = find_tag_end(html, pos) {
                    let attrs: HashMap<String, String> = parse_tag_attrs(&html[pos + len..gt])
                        .into_iter()
                        .map(|(k, v)| (k.to_ascii_lowercase(), v))
                        .collect();
                    if let Some(tx_id) = attrs.get("id").and_then(|v| Uuid::parse_str(v.trim()).ok()) {
                        let (entity_identifier, entity_name) = if is_image {
                            (
                                attrs.get("src").and_then(|v| Uuid::parse_str(v.trim()).ok()),
                                "image".to_string(),
                            )
                        } else {
                            let name: String = attrs
                                .get("entity_name")
                                .map(|s| s.trim().chars().take(30).collect())
                                .unwrap_or_default();
                            (
                                attrs
                                    .get("entity_identifier")
                                    .and_then(|v| Uuid::parse_str(v.trim()).ok()),
                                name,
                            )
                        };
                        out.push(ExtractedComponent {
                            transaction_id: tx_id,
                            entity_identifier,
                            entity_name,
                            entity_type: None,
                        });
                    }
                    i = gt + 1;
                    break;
                }
            }
        }
        if i <= pos {
            i = pos + 1;
        }
    }
    out
}

/// Port of Plane's `page_transaction`.
pub fn spawn_page_transaction(
    st: AppState,
    workspace_id: Uuid,
    page_id: Uuid,
    new_description_html: Option<String>,
    old_description_html: Option<String>,
) {
    tokio::spawn(async move {
        let exists: (bool,) = match sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM pages WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL)",
        )
        .bind(page_id)
        .bind(workspace_id)
        .fetch_one(&st.db)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(%page_id, error = %e, "page_transaction page lookup failed");
                return;
            }
        };
        if !exists.0 {
            return;
        }

        let has_existing_logs: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM page_logs WHERE page_id = $1 AND workspace_id = $2)",
        )
        .bind(page_id)
        .bind(workspace_id)
        .fetch_one(&st.db)
        .await
        .unwrap_or(false);

        let old_components = old_description_html
            .as_deref()
            .map(extract_page_components)
            .unwrap_or_default();
        let new_components = new_description_html
            .as_deref()
            .map(extract_page_components)
            .unwrap_or_default();

        let old_ids: HashSet<Uuid> = old_components.iter().map(|c| c.transaction_id).collect();
        let new_ids: HashSet<Uuid> = new_components.iter().map(|c| c.transaction_id).collect();
        let deleted_ids: Vec<Uuid> = old_ids.difference(&new_ids).copied().collect();

        for comp in new_components {
            if old_ids.contains(&comp.transaction_id) && has_existing_logs {
                continue;
            }
            if let Err(e) = sqlx::query(
                "INSERT INTO page_logs (transaction, page_id, entity_identifier, entity_name, entity_type, workspace_id)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (page_id, transaction) DO NOTHING",
            )
            .bind(comp.transaction_id)
            .bind(page_id)
            .bind(comp.entity_identifier)
            .bind(&comp.entity_name)
            .bind(&comp.entity_type)
            .bind(workspace_id)
            .execute(&st.db)
            .await
            {
                tracing::warn!(%page_id, transaction = %comp.transaction_id, error = %e, "page_logs insert failed");
            }
        }

        if !deleted_ids.is_empty() {
            if let Err(e) = sqlx::query(
                "DELETE FROM page_logs WHERE page_id = $1 AND workspace_id = $2 AND transaction = ANY($3)",
            )
            .bind(page_id)
            .bind(workspace_id)
            .bind(&deleted_ids)
            .execute(&st.db)
            .await
            {
                tracing::warn!(%page_id, error = %e, "page_logs delete failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_title_and_favicon_links() {
        let html = r#"<!doctype html><html><head>
            <script>var x = "<title>Nope</title>";</script>
            <TITLE>  Hello &amp; World &#39;Test&#39; </TITLE>
            <link rel="shortcut icon" href="/shortcut.ico">
            <link rel="icon" type="image/png" href="https://cdn.example.com/fav.png?v=1">
        </head><body></body></html>"#;
        let (title, links) = parse_title_and_icons(html);
        assert_eq!(title.as_deref(), Some("Hello & World 'Test'"));
        assert_eq!(
            links,
            vec![
                ("shortcut icon".into(), "/shortcut.ico".into()),
                ("icon".into(), "https://cdn.example.com/fav.png?v=1".into()),
            ]
        );
    }

    #[test]
    fn extracts_and_rewrites_page_components() {
        let tx1 = Uuid::new_v4();
        let ent1 = Uuid::new_v4();
        let tx2 = Uuid::new_v4();
        let asset_old = Uuid::new_v4();
        let asset_new = Uuid::new_v4();
        let html = format!(
            r#"<p><mention-component id="{tx1}" entity_identifier="{ent1}" entity_name="issue"></mention-component></p><image-component id="{tx2}" src="{asset_old}"></image-component>"#
        );
        let comps = extract_page_components(&html);
        assert_eq!(comps.len(), 2);
        assert_eq!(comps[0].transaction_id, tx1);
        assert_eq!(comps[0].entity_identifier, Some(ent1));
        assert_eq!(comps[0].entity_name, "issue");
        assert_eq!(comps[1].transaction_id, tx2);
        assert_eq!(comps[1].entity_identifier, Some(asset_old));
        assert_eq!(comps[1].entity_name, "image");

        assert_eq!(extract_image_component_asset_ids(&html), vec![asset_old]);
        let rewritten = replace_image_component_srcs(&html, &[(asset_old.to_string(), asset_new.to_string())]);
        assert!(rewritten.contains(&format!(r#"src="{asset_new}""#)));
        assert!(!rewritten.contains(&asset_old.to_string()));
    }
}
