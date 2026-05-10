use std::collections::HashSet;

use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Selector, node::Node};
use url::Url;

use crate::{
    ConversionError, ConversionOptions, ConversionWarning,
    metadata::{SanitizedMetadata, sanitize_metadata_value},
    text::{collapse_whitespace, escape_xml_attr, escape_xml_text, strip_html_tags},
};

#[derive(Debug)]
pub(crate) struct Chapter {
    pub(crate) title: String,
    pub(crate) xhtml: String,
    pub(crate) warnings: Vec<ConversionWarning>,
}

pub(crate) fn extract_single_chapter(
    html: &str,
    source_url: &Url,
    metadata: &SanitizedMetadata,
    options: &ConversionOptions,
) -> Result<Chapter, ConversionError> {
    let sanitized_html = sanitize_html_fragment(html);
    let document = Html::parse_document(&sanitized_html);
    let root = select_reading_root(&document).ok_or(ConversionError::NoReadableContent)?;
    let ids = collect_ids(root);
    let context = RenderContext {
        source_url,
        ids,
        include_images: options.include_images,
    };

    let visible_text = collapse_whitespace(&visible_text_for_children(root));
    if visible_text.is_empty() {
        return Err(ConversionError::NoReadableContent);
    }

    let mut body = String::new();
    for child in root.children() {
        render_node(child, &context, &mut body);
    }

    if collapse_whitespace(&strip_html_tags(&body)).is_empty() {
        return Err(ConversionError::NoReadableContent);
    }

    let title = first_heading(root)
        .or_else(|| document_title(&document))
        .unwrap_or_else(|| metadata.title.clone());
    let title = sanitize_metadata_value(&title, &metadata.title);
    let xhtml = chapter_document(&metadata.language, &title, &body);

    Ok(Chapter {
        title,
        xhtml,
        warnings: Vec::new(),
    })
}

fn sanitize_html_fragment(html: &str) -> String {
    ammonia::Builder::default()
        .add_generic_attributes(&["id"])
        .link_rel(None)
        .clean(html)
        .to_string()
}

fn select_reading_root(document: &Html) -> Option<ElementRef<'_>> {
    for selector in ["article", "main", "body"] {
        let selector = Selector::parse(selector).expect("static selector should parse");
        if let Some(element) = document.select(&selector).next() {
            return Some(element);
        }
    }
    None
}

fn first_heading(root: ElementRef<'_>) -> Option<String> {
    let selector = Selector::parse("h1, h2, h3, h4, h5, h6").expect("static selector should parse");
    root.select(&selector)
        .next()
        .map(|heading| sanitize_metadata_value(&heading.text().collect::<Vec<_>>().join(" "), ""))
        .filter(|title| !title.is_empty())
}

fn document_title(document: &Html) -> Option<String> {
    let selector = Selector::parse("title").expect("static selector should parse");
    document
        .select(&selector)
        .next()
        .map(|title| sanitize_metadata_value(&title.text().collect::<Vec<_>>().join(" "), ""))
        .filter(|title| !title.is_empty())
}

struct RenderContext<'a> {
    source_url: &'a Url,
    ids: HashSet<String>,
    include_images: bool,
}

fn collect_ids(root: ElementRef<'_>) -> HashSet<String> {
    root.descendants()
        .filter_map(ElementRef::wrap)
        .filter_map(|element| element.attr("id").and_then(sanitize_id))
        .collect()
}

fn render_node(node: NodeRef<'_, Node>, context: &RenderContext<'_>, output: &mut String) {
    match node.value() {
        Node::Text(text) => output.push_str(&escape_xml_text(text)),
        Node::Element(_) => {
            let Some(element) = ElementRef::wrap(node) else {
                return;
            };
            let name = element.value().name();

            if is_active_or_unsafe_element(name) {
                return;
            }

            if name == "a" {
                render_anchor(element, context, output);
                return;
            }

            if name == "img" {
                render_image_alt(element, context, output);
                return;
            }

            let Some(tag) = safe_xhtml_tag(name) else {
                render_children(element, context, output);
                return;
            };

            if matches!(tag, "br" | "hr") {
                output.push('<');
                output.push_str(tag);
                render_id_attr(element, output);
                output.push_str(" />");
                return;
            }

            output.push('<');
            output.push_str(tag);
            render_id_attr(element, output);
            if matches!(tag, "th" | "td") {
                render_scope_attr(element, output);
            }
            output.push('>');
            render_children(element, context, output);
            output.push_str("</");
            output.push_str(tag);
            output.push('>');
        }
        _ => {}
    }
}

fn render_children(element: ElementRef<'_>, context: &RenderContext<'_>, output: &mut String) {
    for child in element.children() {
        render_node(child, context, output);
    }
}

fn render_anchor(element: ElementRef<'_>, context: &RenderContext<'_>, output: &mut String) {
    let mut child_html = String::new();
    render_children(element, context, &mut child_html);

    if child_html.trim().is_empty() {
        return;
    }

    if let Some(href) = element
        .attr("href")
        .and_then(|href| safe_href(href, context.source_url, &context.ids))
    {
        output.push_str("<a href=\"");
        output.push_str(&escape_xml_attr(&href));
        output.push_str("\">");
        output.push_str(&child_html);
        output.push_str("</a>");
    } else {
        output.push_str(&child_html);
    }
}

fn render_image_alt(element: ElementRef<'_>, context: &RenderContext<'_>, output: &mut String) {
    let Some(alt) = element
        .attr("alt")
        .map(|alt| sanitize_metadata_value(alt, ""))
        .filter(|alt| !alt.is_empty())
    else {
        return;
    };

    let _include_images = context.include_images;
    output.push_str("<span>");
    output.push_str(&escape_xml_text(&alt));
    output.push_str("</span>");
}

fn render_id_attr(element: ElementRef<'_>, output: &mut String) {
    if let Some(id) = element.attr("id").and_then(sanitize_id) {
        output.push_str(" id=\"");
        output.push_str(&escape_xml_attr(&id));
        output.push('"');
    }
}

fn render_scope_attr(element: ElementRef<'_>, output: &mut String) {
    if let Some(scope) = element
        .attr("scope")
        .filter(|scope| matches!(*scope, "row" | "col" | "rowgroup" | "colgroup"))
    {
        output.push_str(" scope=\"");
        output.push_str(scope);
        output.push('"');
    }
}

fn is_active_or_unsafe_element(name: &str) -> bool {
    matches!(
        name,
        "script"
            | "style"
            | "form"
            | "input"
            | "button"
            | "select"
            | "option"
            | "textarea"
            | "iframe"
            | "object"
            | "embed"
            | "applet"
            | "canvas"
            | "video"
            | "audio"
            | "source"
            | "track"
            | "meta"
            | "link"
    )
}

fn safe_xhtml_tag(name: &str) -> Option<&'static str> {
    match name {
        "h1" => Some("h1"),
        "h2" => Some("h2"),
        "h3" => Some("h3"),
        "h4" => Some("h4"),
        "h5" => Some("h5"),
        "h6" => Some("h6"),
        "p" => Some("p"),
        "strong" | "b" => Some("strong"),
        "em" | "i" => Some("em"),
        "u" => Some("u"),
        "blockquote" => Some("blockquote"),
        "ol" => Some("ol"),
        "ul" => Some("ul"),
        "li" => Some("li"),
        "pre" => Some("pre"),
        "code" => Some("code"),
        "table" => Some("table"),
        "caption" => Some("caption"),
        "thead" => Some("thead"),
        "tbody" => Some("tbody"),
        "tfoot" => Some("tfoot"),
        "tr" => Some("tr"),
        "th" => Some("th"),
        "td" => Some("td"),
        "br" => Some("br"),
        "hr" => Some("hr"),
        "span" => Some("span"),
        _ => None,
    }
}

fn safe_href(raw_href: &str, source_url: &Url, ids: &HashSet<String>) -> Option<String> {
    let href = raw_href.trim();
    if href.is_empty() || href.chars().any(char::is_control) {
        return None;
    }

    if let Some(fragment) = href.strip_prefix('#') {
        let id = sanitize_id(fragment)?;
        return ids.contains(&id).then(|| format!("#{id}"));
    }

    if let Ok(url) = Url::parse(href) {
        return match url.scheme() {
            "http" | "https" | "mailto" => Some(href.to_string()),
            _ => None,
        };
    }

    let resolved = source_url.join(href).ok()?;
    let fragment = resolved.fragment().and_then(sanitize_id)?;
    if same_document(source_url, &resolved) && ids.contains(&fragment) {
        Some(format!("#{fragment}"))
    } else {
        None
    }
}

fn same_document(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
        && left.path() == right.path()
        && left.query() == right.query()
}

fn sanitize_id(raw: &str) -> Option<String> {
    let mut id = String::new();
    for character in raw.trim().chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':' | '.') {
            id.push(character);
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }

    let mut id = id.trim_matches('-').to_string();
    if id.is_empty() {
        return None;
    }

    if !id
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
    {
        id.insert_str(0, "id-");
    }

    Some(id)
}

fn visible_text_for_children(root: ElementRef<'_>) -> String {
    let mut text = String::new();
    for child in root.children() {
        collect_visible_text(child, &mut text);
    }
    text
}

fn collect_visible_text(node: NodeRef<'_, Node>, output: &mut String) {
    match node.value() {
        Node::Text(text) => {
            output.push_str(text);
            output.push(' ');
        }
        Node::Element(_) => {
            let Some(element) = ElementRef::wrap(node) else {
                return;
            };
            if is_active_or_unsafe_element(element.value().name()) {
                return;
            }
            if element.value().name() == "img" {
                if let Some(alt) = element.attr("alt") {
                    output.push_str(alt);
                    output.push(' ');
                }
                return;
            }
            for child in element.children() {
                collect_visible_text(child, output);
            }
        }
        _ => {}
    }
}

fn chapter_document(language: &str, title: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" lang="{language}" xml:lang="{language}">
<head>
  <meta charset="utf-8" />
  <title>{title}</title>
</head>
<body>
  <section id="chapter-1">
    {body}
  </section>
</body>
</html>
"#,
        language = escape_xml_attr(language),
        title = escape_xml_text(title),
        body = body
    )
}
