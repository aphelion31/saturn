//! Plane `apps/live` HTTP endpoints: `/live/health`, `/live/convert-document`, `/live/pdf-export`.
use std::collections::HashMap;

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use pdf_writer::{Content, Name, Pdf, Rect, Ref, Str, TextStr};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/live/health", get(live_health))
        .route("/live/health/", get(live_health))
        .route("/live/convert-document", post(convert_document))
        .route("/live/convert-document/", post(convert_document))
        .route("/live/pdf-export", post(pdf_export))
        .route("/live/pdf-export/", post(pdf_export))
}

async fn live_health() -> Json<Value> {
    Json(json!({
        "status": "OK",
        "timestamp": Utc::now().to_rfc3339(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Deserialize)]
struct ConvertDocumentReq {
    description_html: Option<Value>,
    variant: Option<Value>,
}

fn validation_error(errors: Vec<Value>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "message": "Validation error",
            "context": {
                "validationErrors": errors
            }
        })),
    )
}

async fn convert_document(body: Result<Json<ConvertDocumentReq>, axum::extract::rejection::JsonRejection>) -> Response {
    let Ok(Json(req)) = body else {
        return validation_error(vec![json!({"path": "", "message": "Invalid JSON body"})]).into_response();
    };
    let mut errs = Vec::new();
    let html = match req.description_html.as_ref().and_then(Value::as_str) {
        Some(h) if !h.trim().is_empty() && h.contains('<') && h.contains('>') => Some(h),
        Some("") => {
            errs.push(json!({"path": "description_html", "message": "HTML content cannot be empty"}));
            None
        }
        Some(h) if h.trim().is_empty() => {
            errs.push(json!({"path": "description_html", "message": "HTML content cannot be just whitespace"}));
            None
        }
        Some(_) => {
            errs.push(json!({"path": "description_html", "message": "Content must be valid HTML"}));
            None
        }
        None => {
            errs.push(json!({"path": "description_html", "message": "Required"}));
            None
        }
    };
    let variant = match req.variant.as_ref().and_then(Value::as_str) {
        Some(v) if matches!(v, "rich" | "document") => Some(v),
        _ => {
            errs.push(json!({"path": "variant", "message": "Variant must be 'rich' or 'document'"}));
            None
        }
    };
    if !errs.is_empty() {
        return validation_error(errs).into_response();
    }
    let html = html.expect("validated above");
    let variant = variant.expect("validated above");
    match crate::live_doc::html_to_all_formats(html, variant) {
        Ok((description_json, bin)) => (
            StatusCode::OK,
            Json(json!({
                "description_json": description_json,
                "description_binary": STANDARD.encode(&bin),
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "convert-document failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"message": "Internal server error."})),
            )
                .into_response()
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PdfExportReq {
    page_id: String,
    workspace_slug: String,
    project_id: Option<String>,
    title: Option<String>,
    author: Option<String>,
    subject: Option<String>,
    page_size: Option<String>,
    page_orientation: Option<String>,
    file_name: Option<String>,
    #[expect(dead_code, reason = "accepted for parity with Plane PdfExportRequestBody schema")]
    no_assets: Option<bool>,
}

fn pdf_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({"message": msg}))).into_response()
}

fn sanitize_filename(name: &str) -> (String, String) {
    let ascii: String = name
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n'))
        .map(|c| if (' '..='~').contains(&c) { c } else { '_' })
        .collect();
    let utf8_clean: String = name
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n'))
        .collect();
    let mut encoded = String::new();
    for b in utf8_clean.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            encoded.push(*b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    (ascii, encoded)
}

fn valid_workspace_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug != "."
        && slug != ".."
        && !slug
            .chars()
            .any(|c| matches!(c, '/' | '\\' | '?' | '#' | '%') || c.is_control())
}

async fn pdf_export(
    State(st): State<AppState>,
    maybe_user: MaybeUser,
    body: Result<Json<PdfExportReq>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Some(user) = maybe_user.0 else {
        return pdf_err(StatusCode::UNAUTHORIZED, "Authentication required");
    };
    let Ok(Json(req)) = body else {
        return pdf_err(StatusCode::BAD_REQUEST, "Invalid request body");
    };
    let page_id_str = req.page_id.trim();
    let ws_slug = req.workspace_slug.trim();
    if page_id_str.is_empty() || !valid_workspace_slug(ws_slug) {
        return pdf_err(StatusCode::BAD_REQUEST, "Invalid request body");
    }
    let page_size = req.page_size.as_deref().unwrap_or("A4");
    if !matches!(page_size, "A4" | "A3" | "A2" | "LETTER" | "LEGAL" | "TABLOID") {
        return pdf_err(StatusCode::BAD_REQUEST, "Invalid request body");
    }
    let orientation = req.page_orientation.as_deref().unwrap_or("portrait");
    if !matches!(orientation, "portrait" | "landscape") {
        return pdf_err(StatusCode::BAD_REQUEST, "Invalid request body");
    }
    let Ok(page_id) = Uuid::parse_str(page_id_str) else {
        return pdf_err(StatusCode::BAD_REQUEST, "Invalid request body");
    };
    let project_filter: Option<Uuid> = match req.project_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => match Uuid::parse_str(raw) {
            Ok(id) => Some(id),
            Err(_) => return pdf_err(StatusCode::BAD_REQUEST, "Invalid request body"),
        },
        None => None,
    };

    let ws = match require_workspace(&st.db, &user, ws_slug).await {
        Ok(ws) => ws,
        Err(_) => return pdf_err(StatusCode::NOT_FOUND, "Page content not found"),
    };
    if let Some(pid) = project_filter
        && require_project(&st.db, &user, ws_slug, pid).await.is_err()
    {
        return pdf_err(StatusCode::NOT_FOUND, "Page content not found");
    }

    let row: Option<(String, Option<Vec<u8>>, String)> = match sqlx::query_as(
        "SELECT p.name, p.description_binary, COALESCE(p.description_html, '<p></p>')
         FROM pages p
         JOIN project_pages pp ON pp.page_id = p.id AND pp.deleted_at IS NULL
         JOIN projects pr ON pr.id = pp.project_id AND pr.deleted_at IS NULL AND pr.archived_at IS NULL
         JOIN project_members pm ON pm.project_id = pr.id AND pm.member_id = $3 AND pm.is_active AND pm.deleted_at IS NULL
         WHERE p.id = $1 AND p.workspace_id = $2 AND p.deleted_at IS NULL
           AND ($4::uuid IS NULL OR pp.project_id = $4)
           AND (p.owned_by_id = $3 OR p.access = 0)
           AND (pm.role > 5 OR pr.guest_view_all_features OR p.owned_by_id = $3)
         LIMIT 1",
    )
    .bind(page_id)
    .bind(ws.workspace_id)
    .bind(user.id)
    .bind(project_filter)
    .fetch_optional(&st.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "pdf_export query failed");
            return pdf_err(StatusCode::INTERNAL_SERVER_ERROR, "Failed to generate PDF");
        }
    };
    let Some((db_name, binary, html)) = row else {
        return pdf_err(StatusCode::NOT_FOUND, "Page content not found");
    };

    let (doc_json, live_title) = match binary.filter(|b| !b.is_empty()) {
        Some(bin) => match crate::live_doc::formats(&bin) {
            Ok((_, j, t)) => (j, t),
            Err(_) => match crate::live_doc::html_to_all_formats(&html, "document") {
                Ok((j, _)) => (j, None),
                Err(_) => (json!({"type": "doc", "content": []}), None),
            },
        },
        None => match crate::live_doc::html_to_all_formats(&html, "document") {
            Ok((j, _)) => (j, None),
            Err(_) => (json!({"type": "doc", "content": []}), None),
        },
    };

    let mention_ids = collect_mention_ids(&doc_json);
    let mentions = if mention_ids.is_empty() {
        HashMap::new()
    } else {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT u.id, COALESCE(NULLIF(u.display_name, ''), NULLIF(trim(u.first_name || ' ' || u.last_name), ''), u.email)
             FROM users u WHERE u.id = ANY($1)",
        )
        .bind(&mention_ids)
        .fetch_all(&st.db)
        .await
        .unwrap_or_default();
        rows.into_iter().collect()
    };

    let doc_title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| live_title.filter(|s| !s.trim().is_empty()))
        .unwrap_or(db_name);

    let pdf_bytes = render_tiptap_pdf(
        &doc_json,
        &PdfRenderOptions {
            title: &doc_title,
            author: req.author.as_deref(),
            subject: req.subject.as_deref(),
            page_size,
            landscape: orientation == "landscape",
            mentions: &mentions,
        },
    );

    let out_name = req
        .file_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("page-{page_id}.pdf"));
    let (ascii_name, encoded_name) = sanitize_filename(&out_name);
    let mut res = Body::from(pdf_bytes.clone()).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/pdf"));
    if let Ok(v) = HeaderValue::from_str(&format!(
        "attachment; filename=\"{ascii_name}\"; filename*=UTF-8''{encoded_name}"
    )) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    if let Ok(v) = HeaderValue::from_str(&pdf_bytes.len().to_string()) {
        h.insert(header::CONTENT_LENGTH, v);
    }
    res
}

struct PdfRenderOptions<'a> {
    title: &'a str,
    author: Option<&'a str>,
    subject: Option<&'a str>,
    page_size: &'a str,
    landscape: bool,
    mentions: &'a HashMap<Uuid, String>,
}

fn collect_mention_ids(node: &Value) -> Vec<Uuid> {
    let mut ids = Vec::new();
    fn walk(n: &Value, out: &mut Vec<Uuid>) {
        if n["type"].as_str() == Some("mention")
            && let Some(id) = n["attrs"]["entity_identifier"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
        {
            out.push(id);
        }
        if let Some(children) = n["content"].as_array() {
            for c in children {
                walk(c, out);
            }
        }
    }
    walk(node, &mut ids);
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[derive(Clone, Copy)]
enum FontKind {
    Regular,
    Bold,
    Italic,
    Mono,
}

impl FontKind {
    fn name(self) -> Name<'static> {
        match self {
            FontKind::Regular => Name(b"F1"),
            FontKind::Bold => Name(b"F2"),
            FontKind::Italic => Name(b"F3"),
            FontKind::Mono => Name(b"F4"),
        }
    }
}

fn to_win_ansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            '\u{2022}' => 0x95, // bullet
            '\u{2013}' => 0x96, // en-dash
            '\u{2014}' => 0x97, // em-dash
            '\u{2018}' | '\u{2019}' => b'\'',
            '\u{201C}' | '\u{201D}' => b'"',
            '\u{2026}' => 0x85, // ellipsis
            c if (' '..='~').contains(&c) => c as u8,
            c if (0xA0..=0xFF).contains(&(c as u32)) => c as u8,
            '\t' => b' ',
            _ => b'?',
        })
        .collect()
}

fn wrap_text(text: &str, max_width: f32, font_size: f32, mono: bool) -> Vec<String> {
    let avg_char = font_size * if mono { 0.60 } else { 0.50 };
    let max_chars = ((max_width / avg_char).floor() as usize).max(12);
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        let trimmed = raw_line.trim_end();
        if trimmed.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in trimmed.split_whitespace() {
            let word_len = word.chars().count();
            if current.is_empty() {
                if word_len <= max_chars {
                    current.push_str(word);
                } else {
                    let mut chunk = String::new();
                    for ch in word.chars() {
                        chunk.push(ch);
                        if chunk.chars().count() >= max_chars {
                            lines.push(std::mem::take(&mut chunk));
                        }
                    }
                    current = chunk;
                }
            } else if current.chars().count() + 1 + word_len <= max_chars {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(std::mem::take(&mut current));
                if word_len <= max_chars {
                    current.push_str(word);
                } else {
                    let mut chunk = String::new();
                    for ch in word.chars() {
                        chunk.push(ch);
                        if chunk.chars().count() >= max_chars {
                            lines.push(std::mem::take(&mut chunk));
                        }
                    }
                    current = chunk;
                }
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn inline_text(node: &Value, mentions: &HashMap<Uuid, String>) -> String {
    match node["type"].as_str().unwrap_or("") {
        "text" => node["text"].as_str().unwrap_or("").to_string(),
        "hardBreak" => "\n".to_string(),
        "mention" => {
            if let Some(id) = node["attrs"]["entity_identifier"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                && let Some(name) = mentions.get(&id)
            {
                return format!("@{name}");
            }
            "@mention".to_string()
        }
        "imageComponent" | "image" => {
            let alt = node["attrs"]["alt"]
                .as_str()
                .or_else(|| node["attrs"]["title"].as_str())
                .or_else(|| node["attrs"]["src"].as_str())
                .unwrap_or("image");
            format!("[Image: {alt}]")
        }
        _ => {
            let mut out = String::new();
            if let Some(children) = node["content"].as_array() {
                for c in children {
                    out.push_str(&inline_text(c, mentions));
                }
            }
            out
        }
    }
}

struct PdfCursor {
    pages: Vec<Content>,
    width: f32,
    height: f32,
    margin: f32,
    y: f32,
}

impl PdfCursor {
    fn new(width: f32, height: f32, margin: f32) -> Self {
        Self {
            pages: vec![Content::new()],
            width,
            height,
            margin,
            y: height - margin,
        }
    }

    fn content(&mut self) -> &mut Content {
        self.pages.last_mut().expect("at least one page")
    }

    fn ensure_space(&mut self, needed: f32) {
        if self.y - needed < self.margin {
            self.pages.push(Content::new());
            self.y = self.height - self.margin;
        }
    }

    fn draw_lines(&mut self, lines: &[String], font: FontKind, size: f32, indent: f32, color: (f32, f32, f32)) {
        let leading = size * 1.42;
        for line in lines {
            self.ensure_space(leading);
            let x = self.margin + indent;
            let y = self.y - size;
            if !line.is_empty() {
                let c = self.content();
                c.set_fill_rgb(color.0, color.1, color.2);
                c.begin_text();
                c.set_font(font.name(), size);
                c.next_line(x, y);
                let bytes = to_win_ansi(line);
                c.show(Str(&bytes));
                c.end_text();
            }
            self.y -= leading;
        }
    }

    fn draw_block(&mut self, node: &Value, indent: f32, mentions: &HashMap<Uuid, String>) {
        let kind = node["type"].as_str().unwrap_or("");
        let avail = (self.width - self.margin * 2.0 - indent).max(80.0);
        match kind {
            "doc" => {
                if let Some(children) = node["content"].as_array() {
                    for child in children {
                        self.draw_block(child, indent, mentions);
                    }
                }
            }
            "heading" => {
                let level = node["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6);
                let size = match level {
                    1 => 20.0,
                    2 => 16.0,
                    3 => 13.5,
                    _ => 11.5,
                };
                self.y -= 4.0;
                let text = inline_text(node, mentions);
                let lines = wrap_text(&text, avail, size, false);
                self.draw_lines(&lines, FontKind::Bold, size, indent, (0.09, 0.11, 0.14));
                self.y -= 4.0;
            }
            "paragraph" => {
                let text = inline_text(node, mentions);
                if text.trim().is_empty() {
                    self.y -= 8.0;
                } else {
                    let lines = wrap_text(&text, avail, 10.5, false);
                    self.draw_lines(&lines, FontKind::Regular, 10.5, indent, (0.15, 0.18, 0.22));
                    self.y -= 5.0;
                }
            }
            "bulletList" | "taskList" => {
                if let Some(items) = node["content"].as_array() {
                    for item in items {
                        let prefix = if item["type"].as_str() == Some("taskItem") {
                            if item["attrs"]["checked"].as_bool() == Some(true) {
                                "[x] "
                            } else {
                                "[ ] "
                            }
                        } else {
                            "- "
                        };
                        let text = format!("{prefix}{}", inline_text(item, mentions).trim());
                        let lines = wrap_text(&text, avail - 14.0, 10.5, false);
                        self.draw_lines(&lines, FontKind::Regular, 10.5, indent + 14.0, (0.15, 0.18, 0.22));
                    }
                    self.y -= 4.0;
                }
            }
            "orderedList" => {
                let start = node["attrs"]["start"].as_i64().unwrap_or(1);
                if let Some(items) = node["content"].as_array() {
                    for (idx, item) in items.iter().enumerate() {
                        let text = format!("{}. {}", start + idx as i64, inline_text(item, mentions).trim());
                        let lines = wrap_text(&text, avail - 14.0, 10.5, false);
                        self.draw_lines(&lines, FontKind::Regular, 10.5, indent + 14.0, (0.15, 0.18, 0.22));
                    }
                    self.y -= 4.0;
                }
            }
            "blockquote" => {
                let text = inline_text(node, mentions);
                let lines = wrap_text(&text, avail - 16.0, 10.5, false);
                let start_y = self.y;
                self.draw_lines(&lines, FontKind::Italic, 10.5, indent + 14.0, (0.35, 0.40, 0.48));
                let end_y = self.y;
                if end_y < start_y {
                    let x = self.margin + indent + 4.0;
                    let bar_h = (start_y - end_y).min(self.height);
                    let c = self.content();
                    c.set_fill_rgb(0.75, 0.78, 0.83);
                    c.rect(x, end_y + 2.0, 2.0, bar_h);
                    c.fill_nonzero();
                }
                self.y -= 4.0;
            }
            "codeBlock" => {
                let text = inline_text(node, mentions);
                let lines = wrap_text(&text, avail - 16.0, 9.5, true);
                self.draw_lines(&lines, FontKind::Mono, 9.5, indent + 8.0, (0.15, 0.17, 0.21));
                self.y -= 6.0;
            }
            "horizontalRule" => {
                self.ensure_space(12.0);
                let y = self.y - 6.0;
                let x1 = self.margin + indent;
                let x2 = self.width - self.margin;
                let c = self.content();
                c.move_to(x1, y);
                c.line_to(x2, y);
                c.stroke();
                self.y -= 12.0;
            }
            "table" => {
                if let Some(rows) = node["content"].as_array() {
                    for row in rows {
                        let cells: Vec<String> = row["content"]
                            .as_array()
                            .map(|cs| cs.iter().map(|c| inline_text(c, mentions).trim().to_string()).collect())
                            .unwrap_or_default();
                        if !cells.is_empty() {
                            let line = cells.join(" | ");
                            let lines = wrap_text(&line, avail, 9.5, false);
                            self.draw_lines(&lines, FontKind::Regular, 9.5, indent, (0.15, 0.18, 0.22));
                        }
                    }
                    self.y -= 6.0;
                }
            }
            "imageComponent" | "image" => {
                let caption = inline_text(node, mentions);
                let lines = wrap_text(&caption, avail, 10.0, false);
                self.draw_lines(&lines, FontKind::Italic, 10.0, indent, (0.40, 0.45, 0.52));
                self.y -= 4.0;
            }
            _ => {
                let text = inline_text(node, mentions);
                if !text.trim().is_empty() {
                    let lines = wrap_text(&text, avail, 10.5, false);
                    self.draw_lines(&lines, FontKind::Regular, 10.5, indent, (0.15, 0.18, 0.22));
                    self.y -= 4.0;
                }
            }
        }
    }
}

fn render_tiptap_pdf(doc_json: &Value, opts: &PdfRenderOptions<'_>) -> Vec<u8> {
    let (mut w, mut h): (f32, f32) = match opts.page_size {
        "A3" => (841.89, 1190.55),
        "A2" => (1190.55, 1683.78),
        "LETTER" => (612.0, 792.0),
        "LEGAL" => (612.0, 1008.0),
        "TABLOID" => (792.0, 1224.0),
        _ => (595.28, 841.89),
    };
    if opts.landscape {
        std::mem::swap(&mut w, &mut h);
    }
    let margin = 54.0;
    let mut cursor = PdfCursor::new(w, h, margin);

    if !opts.title.trim().is_empty() {
        let title_lines = wrap_text(opts.title, w - margin * 2.0, 22.0, false);
        cursor.draw_lines(&title_lines, FontKind::Bold, 22.0, 0.0, (0.06, 0.09, 0.12));
        cursor.y -= 10.0;
    }
    cursor.draw_block(doc_json, 0.0, opts.mentions);

    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let pages_id = Ref::new(2);
    let info_id = Ref::new(3);
    let f1_id = Ref::new(4);
    let f2_id = Ref::new(5);
    let f3_id = Ref::new(6);
    let f4_id = Ref::new(7);

    pdf.catalog(catalog_id).pages(pages_id);
    {
        let mut info = pdf.document_info(info_id);
        info.title(TextStr(opts.title));
        if let Some(a) = opts.author.filter(|s| !s.is_empty()) {
            info.author(TextStr(a));
        }
        if let Some(s) = opts.subject.filter(|s| !s.is_empty()) {
            info.subject(TextStr(s));
        }
        info.producer(TextStr("Saturn"));
    }
    for (id, base) in [
        (f1_id, b"Helvetica".as_slice()),
        (f2_id, b"Helvetica-Bold".as_slice()),
        (f3_id, b"Helvetica-Oblique".as_slice()),
        (f4_id, b"Courier".as_slice()),
    ] {
        let mut font = pdf.type1_font(id);
        font.base_font(Name(base));
        font.encoding_predefined(Name(b"WinAnsiEncoding"));
    }

    let mut page_ids = Vec::with_capacity(cursor.pages.len());
    let mut next_id = 8i32;
    let page_pairs: Vec<(Ref, Ref, Content)> = cursor
        .pages
        .into_iter()
        .map(|content| {
            let pid = Ref::new(next_id);
            let cid = Ref::new(next_id + 1);
            next_id += 2;
            page_ids.push(pid);
            (pid, cid, content)
        })
        .collect();

    pdf.pages(pages_id).kids(page_ids).count(page_pairs.len() as i32);
    for (page_id, content_id, content) in page_pairs {
        let data = content.finish();
        pdf.stream(content_id, &data);
        let mut page = pdf.page(page_id);
        page.parent(pages_id);
        page.media_box(Rect::new(0.0, 0.0, w, h));
        page.contents(content_id);
        let mut res = page.resources();
        let mut fonts = res.fonts();
        fonts.pair(Name(b"F1"), f1_id);
        fonts.pair(Name(b"F2"), f2_id);
        fonts.pair(Name(b"F3"), f3_id);
        fonts.pair(Name(b"F4"), f4_id);
    }

    pdf.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_renders_multipage_with_metadata() {
        let mut content = Vec::new();
        for i in 0..80 {
            content.push(json!({
                "type": "paragraph",
                "content": [{"type": "text", "text": format!("Line {i}: Saturn PDF export rendering paragraph check.")}]
            }));
        }
        let doc = json!({"type": "doc", "content": content});
        let mentions = HashMap::new();
        let bytes = render_tiptap_pdf(
            &doc,
            &PdfRenderOptions {
                title: "Architecture Spec",
                author: Some("Saturn"),
                subject: Some("Export"),
                page_size: "A4",
                landscape: false,
                mentions: &mentions,
            },
        );
        assert!(bytes.starts_with(b"%PDF-1."), "missing PDF magic header");
        assert!(bytes.windows(5).any(|w| w == b"%%EOF"), "missing EOF marker");
        assert!(bytes.len() > 2000);
    }

    #[test]
    fn sanitize_filename_strips_header_injection() {
        let (ascii, utf8) = sanitize_filename("my\"report\r\n—v1.pdf");
        assert!(!ascii.contains('"') && !ascii.contains('\r') && !ascii.contains('\n'));
        assert_eq!(ascii, "myreport_v1.pdf");
        assert!(!utf8.contains('"') && !utf8.contains("%0D"));
    }

    #[test]
    fn workspace_slug_validation_matches_plane_live() {
        assert!(valid_workspace_slug("acme"));
        assert!(valid_workspace_slug("my.org-1"));
        for bad in ["", ".", "..", "a/b", "a\\b", "a?b", "a#b", "a%20b", "a\x00b"] {
            assert!(!valid_workspace_slug(bad), "expected invalid slug: {bad:?}");
        }
    }
}
