//! Convert Atlassian Document Format (ADF) JSON to Markdown for human display.
//!
//! ADF is a tree of `{ type, content, text, marks, attrs }` nodes. We render
//! the common subset (paragraphs, headings, lists, code, quotes, links, marks).
//! Unknown node types fall through to their children so we never lose text.

use serde_json::Value;

/// Render an ADF document (or any subtree) as Markdown.
/// Returns an empty string when the input is `null` or not an object/array.
pub fn to_markdown(value: &Value) -> String {
    let mut out = String::new();
    walk(value, &mut out);
    out.trim().to_string()
}

fn children(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(|c| c.as_array())
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}

fn walk(node: &Value, out: &mut String) {
    if let Some(arr) = node.as_array() {
        for c in arr {
            walk(c, out);
        }
        return;
    }
    let ntype = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match ntype {
        "doc" => {
            for c in children(node) {
                walk(c, out);
            }
        }
        "paragraph" => {
            for c in children(node) {
                walk(c, out);
            }
            out.push_str("\n\n");
        }
        "heading" => {
            let level = node
                .get("attrs")
                .and_then(|a| a.get("level"))
                .and_then(|l| l.as_u64())
                .unwrap_or(1)
                .clamp(1, 6) as usize;
            out.push_str(&"#".repeat(level));
            out.push(' ');
            for c in children(node) {
                walk(c, out);
            }
            out.push_str("\n\n");
        }
        "text" => {
            let text = node.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let mut wrapped = text.to_string();
            let mut link_href: Option<String> = None;
            if let Some(marks) = node.get("marks").and_then(|m| m.as_array()) {
                for mark in marks {
                    let mt = mark.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match mt {
                        "strong" => wrapped = format!("**{}**", wrapped),
                        "em" => wrapped = format!("*{}*", wrapped),
                        "code" => wrapped = format!("`{}`", wrapped),
                        "strike" => wrapped = format!("~~{}~~", wrapped),
                        "link" => {
                            link_href = mark
                                .get("attrs")
                                .and_then(|a| a.get("href"))
                                .and_then(|h| h.as_str())
                                .map(String::from);
                        }
                        _ => {}
                    }
                }
            }
            if let Some(href) = link_href {
                wrapped = format!("[{}]({})", wrapped, href);
            }
            out.push_str(&wrapped);
        }
        "hardBreak" => out.push('\n'),
        "rule" => out.push_str("\n---\n\n"),
        "bulletList" => {
            render_list(node, out, |_| "- ".to_string());
        }
        "orderedList" => {
            let start = node
                .get("attrs")
                .and_then(|a| a.get("order"))
                .and_then(|o| o.as_u64())
                .unwrap_or(1) as usize;
            render_list(node, out, move |i| format!("{}. ", start + i));
        }
        "listItem" => {
            // Rendered inside render_list; recurse defensively.
            for c in children(node) {
                walk(c, out);
            }
        }
        "codeBlock" => {
            let lang = node
                .get("attrs")
                .and_then(|a| a.get("language"))
                .and_then(|l| l.as_str())
                .unwrap_or("");
            out.push_str("```");
            out.push_str(lang);
            out.push('\n');
            for c in children(node) {
                walk(c, out);
            }
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("```\n\n");
        }
        "blockquote" => {
            let mut buf = String::new();
            for c in children(node) {
                walk(c, &mut buf);
            }
            for line in buf.trim_end().lines() {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
        "mention" => {
            let attrs = node.get("attrs");
            let name = attrs
                .and_then(|a| a.get("text"))
                .or_else(|| attrs.and_then(|a| a.get("displayName")))
                .and_then(|n| n.as_str())
                .unwrap_or("@user");
            out.push_str(name);
        }
        "emoji" => {
            let short = node
                .get("attrs")
                .and_then(|a| a.get("shortName"))
                .and_then(|s| s.as_str())
                .unwrap_or(":emoji:");
            out.push_str(short);
        }
        "inlineCard" | "blockCard" | "embedCard" => {
            let url = node
                .get("attrs")
                .and_then(|a| a.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            out.push_str(url);
        }
        _ => {
            for c in children(node) {
                walk(c, out);
            }
        }
    }
}

fn render_list<F>(node: &Value, out: &mut String, prefix: F)
where
    F: Fn(usize) -> String,
{
    for (i, item) in children(node).iter().enumerate() {
        let mut buf = String::new();
        for c in children(item) {
            walk(c, &mut buf);
        }
        let trimmed = buf.trim_end();
        let mut lines = trimmed.lines();
        if let Some(first) = lines.next() {
            out.push_str(&prefix(i));
            out.push_str(first);
            out.push('\n');
            for rest in lines {
                out.push_str("  ");
                out.push_str(rest);
                out.push('\n');
            }
        }
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_paragraph() {
        let v = json!({
            "type": "doc",
            "content": [
                { "type": "paragraph",
                  "content": [ { "type": "text", "text": "Hello world" } ] }
            ]
        });
        assert_eq!(to_markdown(&v), "Hello world");
    }

    #[test]
    fn marks_and_link() {
        let v = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "bold", "marks": [{"type":"strong"}] },
                    { "type": "text", "text": " and " },
                    { "type": "text", "text": "link",
                      "marks": [{"type":"link","attrs":{"href":"https://x.test"}}] }
                ]
            }]
        });
        assert_eq!(to_markdown(&v), "**bold** and [link](https://x.test)");
    }

    #[test]
    fn bullet_and_code() {
        let v = json!({
            "type": "doc",
            "content": [
                {
                    "type": "bulletList",
                    "content": [
                        { "type": "listItem", "content": [
                            { "type": "paragraph", "content": [
                                { "type": "text", "text": "one" } ] } ] },
                        { "type": "listItem", "content": [
                            { "type": "paragraph", "content": [
                                { "type": "text", "text": "two" } ] } ] }
                    ]
                },
                {
                    "type": "codeBlock",
                    "attrs": { "language": "rust" },
                    "content": [ { "type": "text", "text": "fn main() {}" } ]
                }
            ]
        });
        let md = to_markdown(&v);
        assert!(md.contains("- one\n- two"), "got: {md}");
        assert!(md.contains("```rust\nfn main() {}\n```"), "got: {md}");
    }

    #[test]
    fn empty_for_null() {
        assert_eq!(to_markdown(&Value::Null), "");
    }
}
