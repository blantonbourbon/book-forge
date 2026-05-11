use std::collections::{HashMap, HashSet};

use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Selector, node::Node};
use url::Url;

use crate::{
    ConversionError, ConversionOptions, ConversionWarning,
    metadata::{SanitizedMetadata, sanitize_metadata_value},
    text::{collapse_whitespace, escape_xml_attr, escape_xml_text, strip_html_tags},
    url_tools::{normalize_page_url, normalize_resource_url},
};

#[derive(Debug, Clone)]
pub(crate) struct Chapter {
    pub(crate) title: String,
    pub(crate) xhtml: String,
    pub(crate) warnings: Vec<ConversionWarning>,
}

#[derive(Debug, Clone)]
pub(crate) struct ChapterAnalysis {
    pub(crate) title: String,
    pub(crate) ids: HashSet<String>,
    pub(crate) links: Vec<String>,
    pub(crate) images: Vec<String>,
}

pub(crate) struct LinkRewriteContext<'a> {
    pub(crate) chapter_paths: &'a HashMap<String, String>,
    pub(crate) chapter_ids: &'a HashMap<String, HashSet<String>>,
}

pub(crate) struct ImageRewriteContext<'a> {
    pub(crate) packaged_paths: &'a HashMap<String, String>,
}

pub(crate) fn analyze_chapter(
    html: &str,
    _source_url: &Url,
    metadata: &SanitizedMetadata,
) -> Result<ChapterAnalysis, ConversionError> {
    let document = Html::parse_document(html);
    let root = select_reading_root(&document).ok_or(ConversionError::NoReadableContent)?;
    let ids = collect_ids(root);

    let visible_text = collapse_whitespace(&visible_text_for_children(root));
    if visible_text.is_empty() {
        return Err(ConversionError::NoReadableContent);
    }

    let title = first_heading(root)
        .or_else(|| document_title(&document))
        .unwrap_or_else(|| metadata.title.clone());
    let title = sanitize_metadata_value(&title, &metadata.title);

    Ok(ChapterAnalysis {
        title,
        ids,
        links: collect_attribute_values(root, "a", "href"),
        images: collect_attribute_values(root, "img", "src"),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_chapter(
    html: &str,
    source_url: &Url,
    metadata: &SanitizedMetadata,
    options: &ConversionOptions,
    chapter_number: usize,
    title: &str,
    link_rewrites: Option<&LinkRewriteContext<'_>>,
    image_rewrites: Option<&ImageRewriteContext<'_>>,
) -> Result<Chapter, ConversionError> {
    let document = Html::parse_document(html);
    let root = select_reading_root(&document).ok_or(ConversionError::NoReadableContent)?;
    let context = RenderContext {
        source_url,
        ids: collect_ids(root),
        include_images: options.include_images,
        link_rewrites,
        image_rewrites,
    };

    let mut body = String::new();
    for child in root.children() {
        render_node(child, &context, &mut body);
    }

    if collapse_whitespace(&strip_html_tags(&body)).is_empty() {
        return Err(ConversionError::NoReadableContent);
    }

    let xhtml = chapter_document(&metadata.language, title, chapter_number, &body);

    Ok(Chapter {
        title: title.to_string(),
        xhtml,
        warnings: Vec::new(),
    })
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
    link_rewrites: Option<&'a LinkRewriteContext<'a>>,
    image_rewrites: Option<&'a ImageRewriteContext<'a>>,
}

fn collect_ids(root: ElementRef<'_>) -> HashSet<String> {
    root.descendants()
        .filter_map(ElementRef::wrap)
        .filter_map(|element| element.attr("id").and_then(sanitize_id))
        .collect()
}

fn collect_attribute_values(root: ElementRef<'_>, selector: &str, attribute: &str) -> Vec<String> {
    let selector = Selector::parse(selector).expect("static selector should parse");
    root.select(&selector)
        .filter_map(|element| element.attr(attribute))
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
        .map(str::to_string)
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

    if let Some(href) = element.attr("href").and_then(|href| {
        safe_href(
            href,
            context.source_url,
            &context.ids,
            context.link_rewrites,
        )
    }) {
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
    let alt = element
        .attr("alt")
        .map(|alt| sanitize_metadata_value(alt, ""))
        .unwrap_or_default();

    if context.include_images
        && let Some(src) = element
            .attr("src")
            .and_then(|src| safe_image_src(src, context.source_url, context.image_rewrites))
    {
        output.push_str("<img src=\"");
        output.push_str(&escape_xml_attr(&src));
        output.push_str("\" alt=\"");
        output.push_str(&escape_xml_attr(&alt));
        output.push_str("\" />");
        return;
    }

    if alt.is_empty() {
        return;
    }

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

fn safe_href(
    raw_href: &str,
    source_url: &Url,
    ids: &HashSet<String>,
    link_rewrites: Option<&LinkRewriteContext<'_>>,
) -> Option<String> {
    let href = raw_href.trim();
    if href.is_empty() || href.chars().any(char::is_control) {
        return None;
    }

    if let Some(fragment) = href.strip_prefix('#') {
        let id = sanitize_id(fragment)?;
        return ids.contains(&id).then(|| format!("#{id}"));
    }

    if let Some(rewrites) = link_rewrites {
        return safe_crawl_href(href, source_url, ids, rewrites);
    }

    if let Ok(url) = Url::parse(href) {
        return match url.scheme() {
            "http" | "https" if same_document(source_url, &url) => url
                .fragment()
                .and_then(sanitize_id)
                .filter(|fragment| ids.contains(fragment))
                .map(|fragment| format!("#{fragment}")),
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

fn safe_crawl_href(
    href: &str,
    source_url: &Url,
    ids: &HashSet<String>,
    rewrites: &LinkRewriteContext<'_>,
) -> Option<String> {
    if let Ok(url) = Url::parse(href) {
        return match url.scheme() {
            "http" | "https" => rewrite_http_href(&url, source_url, ids, rewrites),
            "mailto" => Some(href.to_string()),
            _ => None,
        };
    }

    let resolved = source_url.join(href).ok()?;
    match resolved.scheme() {
        "http" | "https" => rewrite_http_href(&resolved, source_url, ids, rewrites),
        _ => None,
    }
}

fn rewrite_http_href(
    resolved: &Url,
    source_url: &Url,
    ids: &HashSet<String>,
    rewrites: &LinkRewriteContext<'_>,
) -> Option<String> {
    let target_key = normalize_page_url(resolved);
    let current_key = normalize_page_url(source_url);

    let Some(target_path) = rewrites.chapter_paths.get(&target_key) else {
        return Some(resolved.to_string());
    };

    let fragment = resolved.fragment().and_then(sanitize_id);
    let fragment = fragment.filter(|fragment| {
        if target_key == current_key {
            ids.contains(fragment)
        } else {
            rewrites
                .chapter_ids
                .get(&target_key)
                .is_some_and(|target_ids| target_ids.contains(fragment))
        }
    });

    if target_key == current_key {
        return fragment.map(|fragment| format!("#{fragment}"));
    }

    Some(match fragment {
        Some(fragment) => format!("{target_path}#{fragment}"),
        None => target_path.clone(),
    })
}

fn safe_image_src(
    raw_src: &str,
    source_url: &Url,
    image_rewrites: Option<&ImageRewriteContext<'_>>,
) -> Option<String> {
    let src = raw_src.trim();
    if src.is_empty() || src.chars().any(char::is_control) {
        return None;
    }

    let rewrites = image_rewrites?;
    let resolved = Url::parse(src).or_else(|_| source_url.join(src)).ok()?;
    if !matches!(resolved.scheme(), "http" | "https") {
        return None;
    }

    rewrites
        .packaged_paths
        .get(&normalize_resource_url(&resolved))
        .cloned()
}

fn same_document(left: &Url, right: &Url) -> bool {
    normalize_page_url(left) == normalize_page_url(right)
}

pub(crate) fn sanitize_id(raw: &str) -> Option<String> {
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

fn chapter_document(language: &str, title: &str, chapter_number: usize, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" lang="{language}" xml:lang="{language}">
<head>
  <meta charset="utf-8" />
  <title>{title}</title>
</head>
<body>
  <section id="chapter-{chapter_number}">
    {body}
  </section>
</body>
</html>
"#,
        language = escape_xml_attr(language),
        title = escape_xml_text(title),
        chapter_number = chapter_number,
        body = body
    )
}
