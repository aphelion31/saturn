//! Plane document XML conversion.
use anyhow::{Context, bail};
use serde_json::{Value, json};
use yrs::{
    Doc, ReadTxn, StateVector, Text, Transact, Update, Xml, XmlElementPrelim, XmlFragment, XmlOut, XmlTextPrelim,
    updates::decoder::Decode,
};

fn nodes<T: ReadTxn>(children: impl Iterator<Item = XmlOut>, txn: &T) -> anyhow::Result<Vec<Value>> {
    let mut result = Vec::new();
    for child in children {
        match child {
            XmlOut::Element(el) => {
                let kind = el.tag().to_string();
                let mut attrs = serde_json::Map::new();
                for (k, v) in el.attributes(txn) {
                    if let yrs::Out::Any(a) = v {
                        attrs.insert(k.to_string(), serde_json::to_value(a)?);
                    }
                }
                match kind.as_str() {
                    "paragraph" => {
                        attrs.entry("textAlign").or_insert(Value::Null);
                    }
                    "heading" => {
                        attrs.entry("textAlign").or_insert(Value::Null);
                        attrs.entry("level").or_insert(json!(1));
                    }
                    "orderedList" => {
                        attrs.entry("start").or_insert(json!(1));
                        attrs.entry("type").or_insert(Value::Null);
                    }
                    "taskItem" => {
                        attrs.entry("checked").or_insert(json!(false));
                    }
                    "codeBlock" => {
                        attrs.entry("language").or_insert(Value::Null);
                    }
                    "calloutComponent" => {
                        attrs.entry("id").or_insert(Value::Null);
                    }
                    "issue-embed-component" => {
                        attrs.retain(|_, v| !v.is_null());
                    }
                    "tableRow" => {
                        attrs.entry("background").or_insert(Value::Null);
                        attrs.entry("textColor").or_insert(Value::Null);
                    }
                    "tableCell" | "tableHeader" => {
                        attrs.entry("colspan").or_insert(json!(1));
                        attrs.entry("rowspan").or_insert(json!(1));
                        attrs.entry("colwidth").or_insert(json!([150]));
                        if kind == "tableCell" {
                            attrs.entry("background").or_insert(Value::Null);
                            attrs.entry("textColor").or_insert(Value::Null);
                        } else {
                            attrs.entry("background").or_insert(json!("none"));
                        }
                    }
                    "mention" => {
                        for k in ["id", "entity_identifier", "entity_name"] {
                            attrs.entry(k).or_insert(Value::Null);
                        }
                    }
                    "imageComponent" => {
                        for (k, v) in [
                            ("src", Value::Null),
                            ("alt", Value::Null),
                            ("title", Value::Null),
                            ("width", json!("35%")),
                            ("height", json!("auto")),
                            ("id", Value::Null),
                            ("aspectRatio", Value::Null),
                            ("alignment", json!("left")),
                            ("status", json!("pending")),
                        ] {
                            attrs.entry(k).or_insert(v);
                        }
                    }
                    "bulletList" | "listItem" | "taskList" | "blockquote" | "horizontalRule" | "hardBreak"
                    | "image" | "callout" | "table" => {}
                    _ => bail!("unsupported editor node: {kind}"),
                }
                let content = nodes(el.children(txn), txn)?;
                let mut node = json!({"type":kind});
                if !attrs.is_empty() {
                    node["attrs"] = Value::Object(attrs);
                }
                if !content.is_empty() {
                    node["content"] = json!(content);
                }
                result.push(node);
            }
            XmlOut::Text(text) => {
                for part in text.diff(txn, |_| ()) {
                    let yrs::Out::Any(yrs::Any::String(s)) = part.insert else {
                        bail!("unsupported embedded text")
                    };
                    if s.is_empty() {
                        continue;
                    }
                    let mut node = json!({"type":"text","text":s.as_ref()});
                    if let Some(attrs) = part.attributes {
                        let mut marks: Vec<_> = attrs
                            .iter()
                            .map(|(k, v)| {
                                let value = serde_json::to_value(v).unwrap_or(Value::Null);
                                if value.is_object() && value.as_object().is_some_and(|o| !o.is_empty()) {
                                    json!({"type": k.as_ref(), "attrs": value})
                                } else {
                                    json!({"type":k.as_ref()})
                                }
                            })
                            .collect();
                        marks.sort_by_key(|m| match m["type"].as_str().unwrap_or("") {
                            "link" => 0,
                            "bold" => 1,
                            "italic" => 2,
                            "strike" => 3,
                            "underline" => 4,
                            "code" => 5,
                            "textStyle" => 6,
                            _ => 7,
                        });
                        if !marks.is_empty() {
                            node["marks"] = json!(marks);
                        }
                    }
                    result.push(node);
                }
            }
            XmlOut::Fragment(_) => bail!("nested XML fragment"),
        }
    }
    Ok(result)
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn attr(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn alignment(a: &Value) -> String {
    a["textAlign"]
        .as_str()
        .map(|v| format!(" style=\"text-align: {};\"", escape(v)))
        .unwrap_or_default()
}
fn render(node: &Value) -> anyhow::Result<String> {
    let kind = node["type"].as_str().context("missing node type")?;
    let a = &node["attrs"];
    let mut body = String::new();
    if let Some(content) = node["content"].as_array() {
        for child in content {
            body.push_str(&render(child)?);
        }
    }
    if kind == "text" {
        body = escape(node["text"].as_str().unwrap_or("")).replace("&quot;", "\"");
        if let Some(marks) = node["marks"].as_array() {
            for mark in marks.iter().rev() {
                let m = mark["type"].as_str().unwrap_or("");
                let ma = &mark["attrs"];
                body = match m {
                    "bold" => format!("<strong>{body}</strong>"),
                    "italic" => format!("<em>{body}</em>"),
                    "strike" => format!("<s>{body}</s>"),
                    "underline" => format!("<u>{body}</u>"),
                    "code" => format!(
                        "<code class=\"rounded-sm bg-layer-3 px-[6px] py-[1.5px] font-code font-medium text-(--extended-color-orange-600) border-[0.5px] border-subtle\" spellcheck=\"false\">{body}</code>"
                    ),
                    "link" => format!(
                        "<a target=\"_blank\" rel=\"noopener noreferrer nofollow\" class=\"text-accent-secondary underline underline-offset-[3px] hover:text-accent-primary transition-colors cursor-pointer\" href=\"{}\">{body}</a>",
                        escape(&attr(ma, "href"))
                    ),
                    "textStyle" | "customColor" => format!("<span>{body}</span>"),
                    _ => bail!("unsupported editor mark: {m}"),
                };
            }
        }
        return Ok(body);
    }
    Ok(match kind {
        "doc" => body,
        "paragraph" => format!("<p class=\"editor-paragraph-block\"{}>{body}</p>", alignment(a)),
        "heading" => {
            let n = a["level"].as_u64().unwrap_or(1).clamp(1, 6);
            format!("<h{n} class=\"editor-heading-block\"{}>{body}</h{n}>", alignment(a))
        }
        "bulletList" => format!("<ul class=\"list-disc pl-7 space-y-(--list-spacing-y)\">{body}</ul>"),
        "orderedList" => format!(
            "<ol class=\"list-decimal pl-7 space-y-(--list-spacing-y)\"{}>{body}</ol>",
            if a["start"].as_i64().unwrap_or(1) != 1 {
                format!(" start=\"{}\"", a["start"])
            } else {
                String::new()
            }
        ),
        "listItem" => format!("<li class=\"not-prose space-y-2\">{body}</li>"),
        "blockquote" => format!("<blockquote>{body}</blockquote>"),
        "codeBlock" => format!(
            "<pre><code{}>{body}</code></pre>",
            a["language"]
                .as_str()
                .map(|s| format!(" class=\"language-{}\"", escape(s)))
                .unwrap_or_default()
        ),
        "horizontalRule" => "<div class=\"py-4 border-strong-1\" data-type=\"horizontalRule\"><div></div></div>".into(),
        "hardBreak" => "<br>".into(),
        "mention" | "imageComponent" | "image" => {
            let tag = match kind {
                "mention" => "mention-component",
                "imageComponent" => "image-component",
                _ => "img",
            };
            let keys: &[&str] = match kind {
                "mention" => &["id", "entity_identifier", "entity_name"],
                "imageComponent" => &[
                    "src",
                    "alt",
                    "title",
                    "width",
                    "height",
                    "id",
                    "aspectRatio",
                    "alignment",
                    "status",
                ],
                _ => &["src", "alt", "title"],
            };
            let mut s = format!("<{tag}");
            for k in keys {
                if let Some(v) = a.get(*k).filter(|v| !v.is_null()) {
                    let value = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                    let key = if *k == "aspectRatio" { "aspectratio" } else { k };
                    s.push_str(&format!(" {key}=\"{}\"", escape(&value)));
                }
            }
            s.push('>');
            if tag != "img" {
                s.push_str(&format!("</{tag}>"));
            }
            s
        }
        "table" => format!("<table><tbody>{body}</tbody></table>"),
        "tableRow" => format!("<tr style=\"\">{body}</tr>"),
        "tableCell" | "tableHeader" => {
            let tag = if kind == "tableCell" { "td" } else { "th" };
            let width = a["colwidth"]
                .as_array()
                .map(|a| a.iter().map(Value::to_string).collect::<Vec<_>>().join(","))
                .unwrap_or_default();
            let bg = a["background"].as_str();
            format!(
                "<{tag} colspan=\"{}\" rowspan=\"{}\" colwidth=\"{}\"{} style=\"{}\">{body}</{tag}>",
                a["colspan"],
                a["rowspan"],
                width,
                bg.map(|v| format!(" background=\"{}\"", escape(v))).unwrap_or_default(),
                bg.map(|v| format!("background-color: {};", escape(v)))
                    .unwrap_or_default()
            )
        }
        "taskList" => format!("<ul class=\"not-prose pl-2 space-y-2\" data-type=\"taskList\">{body}</ul>"),
        "taskItem" => {
            let checked = a["checked"].as_bool().unwrap_or(false);
            format!(
                "<li class=\"flex\" data-checked=\"{checked}\" data-type=\"taskItem\"><label><input type=\"checkbox\"{}><span></span></label><div>{body}</div></li>",
                if checked { " checked=\"checked\"" } else { "" }
            )
        }
        "callout" => format!("<div data-type=\"callout\">{body}</div>"),
        "calloutComponent" | "issue-embed-component" => {
            let (tag, keys): (&str, &[&str]) = if kind == "calloutComponent" {
                (
                    "div",
                    &[
                        "id",
                        "data-icon-color",
                        "data-icon-name",
                        "data-emoji-unicode",
                        "data-emoji-url",
                        "data-logo-in-use",
                        "data-background",
                        "data-block-type",
                    ],
                )
            } else {
                (
                    "issue-embed-component",
                    &[
                        "entity_identifier",
                        "project_identifier",
                        "workspace_identifier",
                        "id",
                        "entity_name",
                    ],
                )
            };
            let mut out = format!("<{tag}");
            for key in keys {
                if let Some(v) = a.get(*key).filter(|v| !v.is_null()) {
                    out.push_str(&format!(
                        " {key}=\"{}\"",
                        escape(&v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
                    ));
                }
            }
            format!("{out}>{body}</{tag}>")
        }
        _ => bail!("unsupported editor node: {kind}"),
    })
}
pub fn formats(state: &[u8]) -> anyhow::Result<(String, Value, Option<String>)> {
    let doc = Doc::new();
    doc.transact_mut().apply_update(Update::decode_v1(state)?)?;
    let txn = doc.transact();
    let content = match txn.get_xml_fragment("default") {
        Some(f) => nodes(f.children(&txn), &txn)?,
        None => Vec::new(),
    };
    let json = json!({"type":"doc","content":content});
    let html = render(&json)?;
    let title = txn
        .get_xml_fragment("title")
        .map(|f| nodes(f.children(&txn), &txn))
        .transpose()?
        .map(|nodes| {
            fn text(n: &Value, out: &mut String) {
                if let Some(s) = n["text"].as_str() {
                    out.push_str(s);
                }
                if let Some(c) = n["content"].as_array() {
                    for n in c {
                        text(n, out);
                    }
                }
            }
            let mut out = String::new();
            for n in nodes {
                text(&n, &mut out);
            }
            out
        });
    Ok((html, json, title))
}

#[derive(Default)]
struct HtmlSink(std::cell::RefCell<Vec<HtmlNode>>);
#[derive(Default, Clone)]
struct HtmlNode {
    tag: String,
    attrs: std::collections::HashMap<String, String>,
    children: Vec<HtmlNode>,
    text: String,
}
impl html5ever::tokenizer::TokenSink for HtmlSink {
    type Handle = ();
    fn process_token(&self, token: html5ever::tokenizer::Token, _: u64) -> html5ever::tokenizer::TokenSinkResult<()> {
        use html5ever::tokenizer::{TagKind::*, Token::*};
        let mut stack = self.0.borrow_mut();
        match token {
            TagToken(t) if t.kind == StartTag => {
                let tag = t.name.to_string();
                let node = HtmlNode {
                    tag: tag.clone(),
                    attrs: t
                        .attrs
                        .iter()
                        .map(|a| (a.name.local.to_string(), a.value.to_string()))
                        .collect(),
                    ..Default::default()
                };
                if t.self_closing || matches!(tag.as_str(), "br" | "hr" | "img") {
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    }
                } else {
                    stack.push(node);
                }
            }
            TagToken(t) if t.kind == EndTag => {
                if let Some(pos) = stack.iter().rposition(|n| n.tag == t.name.as_ref()) {
                    while stack.len() > pos && stack.len() > 1 {
                        let n = stack.pop().unwrap();
                        stack.last_mut().unwrap().children.push(n);
                    }
                }
            }
            CharacterTokens(s) => {
                if let Some(p) = stack.last_mut() {
                    if let Some(last) = p.children.last_mut().filter(|n| n.tag.is_empty()) {
                        last.text.push_str(&s);
                    } else {
                        p.children.push(HtmlNode {
                            text: s.to_string(),
                            ..Default::default()
                        });
                    }
                }
            }
            _ => {}
        }
        html5ever::tokenizer::TokenSinkResult::Continue
    }
}
fn html_tree(html: &str) -> HtmlNode {
    use html5ever::tokenizer::{BufferQueue, Tokenizer, TokenizerOpts};
    let sink = HtmlSink(std::cell::RefCell::new(vec![HtmlNode {
        tag: "doc".into(),
        ..Default::default()
    }]));
    let tok = Tokenizer::new(sink, TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(html.into());
    let _ = tok.feed(&input);
    tok.end();
    let mut stack = tok.sink.0.into_inner();
    while stack.len() > 1 {
        let n = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(n);
    }
    stack.pop().unwrap()
}
fn insert_nodes<P: XmlFragment>(
    parent: &P,
    txn: &mut yrs::TransactionMut,
    children: &[HtmlNode],
    marks: &yrs::types::Attrs,
) -> anyhow::Result<()> {
    for child in children {
        if child.tag.is_empty() {
            if !child.text.is_empty() {
                let text = parent.insert(txn, parent.len(txn), XmlTextPrelim::new(""));
                if marks.is_empty() {
                    text.insert(txn, 0, &child.text);
                } else {
                    text.insert_with_attributes(txn, 0, &child.text, marks.clone());
                }
            }
            continue;
        }
        let kind = match child.tag.as_str() {
            "p" => "paragraph",
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
            "ul" => {
                if child.attrs.get("data-type").is_some_and(|s| s == "taskList") {
                    "taskList"
                } else {
                    "bulletList"
                }
            }
            "ol" => "orderedList",
            "li" => {
                if child.attrs.get("data-type").is_some_and(|s| s == "taskItem") {
                    "taskItem"
                } else {
                    "listItem"
                }
            }
            "blockquote" => "blockquote",
            "pre" => "codeBlock",
            "hr" => "horizontalRule",
            "br" => "hardBreak",
            "mention-component" => "mention",
            "image-component" => "imageComponent",
            "img" => "image",
            "div"
                if child
                    .attrs
                    .get("data-block-type")
                    .is_some_and(|v| v == "callout-component") =>
            {
                "calloutComponent"
            }
            "issue-embed-component" => "issue-embed-component",
            "table" => "table",
            "tr" => "tableRow",
            "td" => "tableCell",
            "th" => "tableHeader",
            "strong" | "b" | "em" | "i" | "u" | "s" | "del" | "code" | "a" | "span" => {
                let mark = match child.tag.as_str() {
                    "strong" | "b" => "bold",
                    "em" | "i" => "italic",
                    "u" => "underline",
                    "s" | "del" => "strike",
                    "code" => "code",
                    "a" => "link",
                    _ if child.attrs.get("data-type").is_some_and(|v| v == "emoji") => "customColor",
                    _ => "textStyle",
                };
                let mut m = marks.clone();
                let mut attrs = serde_json::Map::new();
                if mark == "customColor" {
                    attrs.insert("color".into(), Value::Null);
                    attrs.insert("backgroundColor".into(), Value::Null);
                }
                if mark == "link" {
                    attrs.insert("target".into(), json!("_blank"));
                    attrs.insert("rel".into(), json!("noopener noreferrer nofollow"));
                    attrs.insert("class".into(), json!("text-accent-secondary underline underline-offset-[3px] hover:text-accent-primary transition-colors cursor-pointer"));
                    if let Some(v) = child.attrs.get("href") {
                        attrs.insert("href".into(), json!(v));
                    }
                }
                m.insert(mark.into(), yrs::Any::from_json(&Value::Object(attrs).to_string())?);
                insert_nodes(parent, txn, &child.children, &m)?;
                continue;
            }
            "tbody" | "thead" | "tfoot" => {
                insert_nodes(parent, txn, &child.children, marks)?;
                continue;
            }
            tag => bail!("unsupported HTML element: {tag}"),
        };
        let el = parent.insert(txn, parent.len(txn), XmlElementPrelim::empty(kind));
        if kind == "heading" {
            let level = child.tag[1..].parse::<i64>()?;
            el.insert_attribute(txn, "level", level);
        }
        if matches!(kind, "paragraph" | "heading") {
            if let Some(style) = child.attrs.get("style") {
                for declaration in style.split(';') {
                    if let Some(("text-align", value)) = declaration.trim().split_once(':') {
                        el.insert_attribute(txn, "textAlign", value.trim().to_string());
                    }
                }
            }
        }
        if kind == "orderedList" {
            if let Some(start) = child.attrs.get("start").and_then(|v| v.parse::<i64>().ok()) {
                el.insert_attribute(txn, "start", start);
            }
        }
        if kind == "taskItem" {
            el.insert_attribute(
                txn,
                "checked",
                child.attrs.get("data-checked").is_some_and(|v| v == "true"),
            );
        }
        if kind == "codeBlock" {
            if let Some(language) = child
                .children
                .first()
                .and_then(|c| c.attrs.get("class"))
                .and_then(|s| s.split_whitespace().find_map(|s| s.strip_prefix("language-")))
            {
                el.insert_attribute(txn, "language", language.to_string());
            }
        }
        if kind == "imageComponent" {
            if let Some(v) = child.attrs.get("aspectratio").and_then(|v| v.parse::<f64>().ok()) {
                el.insert_attribute(txn, "aspectRatio", v);
            }
        }
        if matches!(kind, "calloutComponent" | "issue-embed-component") {
            for (k, v) in &child.attrs {
                if k == "data-emoji-unicode" {
                    if let Ok(n) = v.parse::<i64>() {
                        el.insert_attribute(txn, k.clone(), n);
                    }
                } else {
                    el.insert_attribute(txn, k.clone(), v.clone());
                }
            }
        }
        if matches!(kind, "mention" | "imageComponent" | "image") {
            for (k, v) in &child.attrs {
                if matches!(
                    k.as_str(),
                    "id" | "entity_identifier"
                        | "entity_name"
                        | "src"
                        | "alt"
                        | "title"
                        | "width"
                        | "height"
                        | "alignment"
                        | "status"
                ) {
                    el.insert_attribute(txn, k.clone(), v.clone());
                }
            }
        }
        if kind == "codeBlock" && child.children.len() == 1 && child.children[0].tag == "code" {
            insert_nodes(&el, txn, &child.children[0].children, marks)?;
        } else {
            insert_nodes(&el, txn, &child.children, marks)?;
        }
    }
    Ok(())
}
pub fn initialize(html: &str, title: &str) -> anyhow::Result<Vec<u8>> {
    let tree = html_tree(html);
    let doc = Doc::new();
    let root = doc.get_or_insert_xml_fragment("default");
    let title_root = doc.get_or_insert_xml_fragment("title");
    {
        let mut txn = doc.transact_mut();
        insert_nodes(&root, &mut txn, &tree.children, &Default::default())?;
        if root.len(&txn) == 0 {
            root.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        }
        let h = title_root.insert(&mut txn, 0, XmlElementPrelim::empty("heading"));
        h.insert_attribute(&mut txn, "level", 1i64);
        if !title.is_empty() {
            h.insert(&mut txn, 0, XmlTextPrelim::new(title));
        }
    }
    let state = doc.transact().encode_state_as_update_v1(&StateVector::default());
    Ok(state)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plane_fixture() {
        use base64::Engine;
        let state=base64::engine::general_purpose::STANDARD.decode("Agb13tjYDgAHAQdkZWZhdWx0AwlwYXJhZ3JhcGgHAPXe2NgOAAYEAPXe2NgOAQZIZWxsbyCG9d7Y2A4HBGJvbGQCe32E9d7Y2A4IBXdvcmxkhvXe2NgODQRib2xkBG51bGwEvYnQiAsABwEFdGl0bGUDB2hlYWRpbmcHAL2J0IgLAAYEAL2J0IgLAQVUaXRsZSgAvYnQiAsABWxldmVsAX0BAA==").unwrap();
        let (html, json, title) = formats(&state).unwrap();
        assert_eq!(
            html,
            "<p class=\"editor-paragraph-block\">Hello <strong>world</strong></p>"
        );
        assert_eq!(json["content"][0]["content"][1]["marks"][0]["type"], "bold");
        assert_eq!(title.as_deref(), Some("Title"));
    }
    #[test]
    fn roundtrip() {
        for html in [
            "<p>Hello <strong>world</strong></p>",
            "<p><mention-component id=\"abc\" entity_identifier=\"a\" entity_name=\"user\"></mention-component></p>",
            "<image-component src=\"https://example.com/a.png\"></image-component>",
        ] {
            let state = initialize(html, "Title").unwrap();
            let (a, _, title) = formats(&state).unwrap();
            let (b, _, _) = formats(&initialize(&a, "Title").unwrap()).unwrap();
            assert_eq!(a, b);
            assert_eq!(title.as_deref(), Some("Title"));
        }
    }
}

/// Parity with Plane's own editor (`@plane/editor/server`), fixtures generated by
/// tests/fixtures/plane_live_doc.gen.mjs. For every case:
/// 1. decoding Plane's Y binary yields Plane's HTML and JSON, and
/// 2. initializing from the input HTML then decoding yields the same HTML and JSON.
#[cfg(test)]
mod plane_parity {
    use base64::Engine;
    use serde_json::Value;

    fn fixtures() -> serde_json::Map<String, Value> {
        serde_json::from_str::<Value>(include_str!("../tests/fixtures/plane_live_doc.json"))
            .unwrap()
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn decode_plane_binary() {
        let mut failures = Vec::new();
        for (name, case) in fixtures() {
            let bin = base64::engine::general_purpose::STANDARD
                .decode(case["binary"].as_str().unwrap())
                .unwrap();
            match super::formats(&bin) {
                Ok((html, json, title)) => {
                    if html != case["html"].as_str().unwrap() {
                        failures.push(format!(
                            "{name}: html\n  got  {html}\n  want {}",
                            case["html"].as_str().unwrap()
                        ));
                    }
                    if json != case["json"] {
                        failures.push(format!("{name}: json\n  got  {json}\n  want {}", case["json"]));
                    }
                    if title.as_deref() != Some(&format!("Title {name}")[..]) {
                        failures.push(format!("{name}: title {title:?}"));
                    }
                }
                Err(e) => failures.push(format!("{name}: decode error {e:#}")),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn initialize_matches_plane() {
        let mut failures = Vec::new();
        for (name, case) in fixtures() {
            let result = super::initialize(case["input"].as_str().unwrap(), &format!("Title {name}"))
                .and_then(|state| super::formats(&state));
            match result {
                Ok((html, json, _)) => {
                    if html != case["html"].as_str().unwrap() {
                        failures.push(format!(
                            "{name}: html\n  got  {html}\n  want {}",
                            case["html"].as_str().unwrap()
                        ));
                    }
                    if json != case["json"] {
                        failures.push(format!("{name}: json\n  got  {json}\n  want {}", case["json"]));
                    }
                }
                Err(e) => failures.push(format!("{name}: error {e:#}")),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
