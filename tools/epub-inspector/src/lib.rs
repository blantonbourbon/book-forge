use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};

use serde::Serialize;
use zip::ZipArchive;

#[derive(Debug, Serialize)]
pub struct InspectionReport {
    pub path: String,
    pub ok: bool,
    pub entry_count: usize,
    pub entries: Vec<String>,
    pub required_entries: RequiredEntries,
    pub package: Option<PackageReport>,
    pub xhtml: Vec<XhtmlReport>,
    pub external_references: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct RequiredEntries {
    pub mimetype: bool,
    pub container_xml: bool,
    pub package_document: bool,
    pub navigation_document: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct PackageReport {
    pub path: String,
    pub metadata: MetadataReport,
    pub manifest: Vec<ManifestItem>,
    pub spine: Vec<SpineItem>,
    pub content_chapters: Vec<ContentChapter>,
    pub nav_entries: Vec<NavEntry>,
}

#[derive(Debug, Default, Serialize)]
pub struct MetadataReport {
    pub title: String,
    pub author: String,
    pub language: String,
    pub description: String,
    pub identifier: String,
    pub modified: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestItem {
    pub id: String,
    pub href: String,
    pub path: String,
    pub media_type: String,
    pub properties: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SpineItem {
    pub idref: String,
    pub href: String,
}

#[derive(Debug, Serialize)]
pub struct ContentChapter {
    pub idref: String,
    pub href: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct NavEntry {
    pub href: String,
    pub label: String,
}

#[derive(Debug, Serialize)]
pub struct XhtmlReport {
    pub path: String,
    pub ids: Vec<String>,
    pub hrefs: Vec<String>,
    pub srcs: Vec<String>,
}

pub fn inspect_epub(path: &Path) -> InspectionReport {
    let mut report = InspectionReport {
        path: path.display().to_string(),
        ok: false,
        entry_count: 0,
        entries: Vec::new(),
        required_entries: RequiredEntries::default(),
        package: None,
        xhtml: Vec::new(),
        external_references: Vec::new(),
        errors: Vec::new(),
    };

    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            report.errors.push(format!("could not open EPUB: {error}"));
            return report;
        }
    };

    let mut archive = match ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(error) => {
            report
                .errors
                .push(format!("could not read ZIP container: {error}"));
            return report;
        }
    };

    let mut contents = HashMap::<String, Vec<u8>>::new();
    for index in 0..archive.len() {
        match archive.by_index(index) {
            Ok(mut entry) => {
                let name = entry.name().replace('\\', "/");
                report.entries.push(name.clone());
                if entry.is_file() {
                    let mut bytes = Vec::new();
                    if let Err(error) = entry.read_to_end(&mut bytes) {
                        report
                            .errors
                            .push(format!("could not read ZIP entry {name}: {error}"));
                    } else {
                        contents.insert(name, bytes);
                    }
                }
            }
            Err(error) => report
                .errors
                .push(format!("could not inspect ZIP entry {index}: {error}")),
        }
    }

    report.entries.sort();
    report.entry_count = report.entries.len();
    report.required_entries.mimetype = report.entries.iter().any(|entry| entry == "mimetype");
    report.required_entries.container_xml = report
        .entries
        .iter()
        .any(|entry| entry == "META-INF/container.xml");

    inspect_required_container_files(&mut report, &contents);

    if let Some(container_xml) = read_text_entry(&contents, "META-INF/container.xml", &mut report)
        && let Some(package_path) = parse_container_xml(&container_xml, &mut report)
    {
        report.required_entries.package_document = report.entries.contains(&package_path);
        if !report.required_entries.package_document {
            report
                .errors
                .push(format!("package document {package_path} is missing"));
        } else if let Some(package_xml) = read_text_entry(&contents, &package_path, &mut report) {
            let package = inspect_package(&package_path, &package_xml, &contents, &mut report);
            report.package = Some(package);
        }
    }

    report.ok = report.errors.is_empty();
    report
}

fn inspect_required_container_files(
    report: &mut InspectionReport,
    contents: &HashMap<String, Vec<u8>>,
) {
    if !report.required_entries.mimetype {
        report
            .errors
            .push("missing required mimetype entry".to_string());
    } else if contents
        .get("mimetype")
        .is_none_or(|bytes| bytes != b"application/epub+zip")
    {
        report
            .errors
            .push("mimetype entry must contain application/epub+zip".to_string());
    }

    if !report.required_entries.container_xml {
        report
            .errors
            .push("missing required META-INF/container.xml entry".to_string());
    }
}

fn read_text_entry(
    contents: &HashMap<String, Vec<u8>>,
    path: &str,
    report: &mut InspectionReport,
) -> Option<String> {
    let bytes = contents.get(path)?;
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_string()),
        Err(error) => {
            report.errors.push(format!(
                "entry {path} is not valid UTF-8 XML/XHTML: {error}"
            ));
            None
        }
    }
}

fn parse_container_xml(container_xml: &str, report: &mut InspectionReport) -> Option<String> {
    let document = match roxmltree::Document::parse(container_xml) {
        Ok(document) => document,
        Err(error) => {
            report.errors.push(format!(
                "META-INF/container.xml is not parseable XML: {error}"
            ));
            return None;
        }
    };

    let package_path = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "rootfile")
        .and_then(|node| node.attribute("full-path"))
        .map(str::to_string);

    if package_path.is_none() {
        report
            .errors
            .push("container.xml does not name a package rootfile".to_string());
    }

    package_path
}

fn inspect_package(
    package_path: &str,
    package_xml: &str,
    contents: &HashMap<String, Vec<u8>>,
    report: &mut InspectionReport,
) -> PackageReport {
    let document = match roxmltree::Document::parse(package_xml) {
        Ok(document) => document,
        Err(error) => {
            report
                .errors
                .push(format!("{package_path} is not parseable OPF XML: {error}"));
            return PackageReport {
                path: package_path.to_string(),
                ..PackageReport::default()
            };
        }
    };

    let metadata = inspect_metadata(&document, report);
    let manifest = inspect_manifest(package_path, &document, contents, report);
    let spine = inspect_spine(&document, &manifest, report);
    let nav_entries = inspect_nav_entries(&manifest, contents, report);
    let content_chapters = inspect_content_chapters(&spine, &manifest, report);

    compare_nav_to_spine(&nav_entries, &content_chapters, report);
    inspect_xhtml_references(&manifest, contents, report);

    PackageReport {
        path: package_path.to_string(),
        metadata,
        manifest,
        spine,
        content_chapters,
        nav_entries,
    }
}

fn inspect_metadata(
    document: &roxmltree::Document<'_>,
    report: &mut InspectionReport,
) -> MetadataReport {
    let metadata = MetadataReport {
        title: first_text(document, "title"),
        author: first_text(document, "creator"),
        language: first_text(document, "language"),
        description: first_text(document, "description"),
        identifier: first_text(document, "identifier"),
        modified: document
            .descendants()
            .find(|node| {
                node.is_element()
                    && node.tag_name().name() == "meta"
                    && node.attribute("property") == Some("dcterms:modified")
            })
            .and_then(|node| node.text())
            .unwrap_or_default()
            .trim()
            .to_string(),
    };

    if metadata.title.is_empty() {
        report
            .errors
            .push("OPF metadata missing dc:title".to_string());
    }
    if metadata.language.is_empty() {
        report
            .errors
            .push("OPF metadata missing dc:language".to_string());
    }
    if metadata.identifier.is_empty() {
        report
            .errors
            .push("OPF metadata missing dc:identifier".to_string());
    }
    if metadata.modified.is_empty() {
        report
            .errors
            .push("OPF metadata missing dcterms:modified".to_string());
    }

    metadata
}

fn first_text(document: &roxmltree::Document<'_>, local_name: &str) -> String {
    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == local_name)
        .and_then(|node| node.text())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn inspect_manifest(
    package_path: &str,
    document: &roxmltree::Document<'_>,
    contents: &HashMap<String, Vec<u8>>,
    report: &mut InspectionReport,
) -> Vec<ManifestItem> {
    let mut ids = HashSet::new();
    let mut hrefs = HashSet::new();
    let mut items = Vec::new();

    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "item")
    {
        let id = node.attribute("id").unwrap_or_default().trim().to_string();
        let href = node
            .attribute("href")
            .unwrap_or_default()
            .trim()
            .to_string();
        let media_type = node
            .attribute("media-type")
            .unwrap_or_default()
            .trim()
            .to_string();
        let properties = node
            .attribute("properties")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();

        if id.is_empty() {
            report.errors.push("manifest item missing id".to_string());
        } else if !ids.insert(id.clone()) {
            report.errors.push(format!("duplicate manifest id {id:?}"));
        }

        if href.is_empty() {
            report
                .errors
                .push(format!("manifest item {id:?} missing href"));
        }

        let Some(path) = resolve_epub_path(package_path, &href) else {
            report
                .errors
                .push(format!("manifest href {href:?} cannot be resolved safely"));
            continue;
        };

        if !hrefs.insert(path.clone()) {
            report
                .errors
                .push(format!("duplicate manifest href {path:?}"));
        }

        if !contents.contains_key(&path) {
            report.errors.push(format!(
                "manifest item {id:?} points to missing file {path}"
            ));
        }

        items.push(ManifestItem {
            id,
            href,
            path,
            media_type,
            properties,
        });
    }

    if items.is_empty() {
        report.errors.push("OPF manifest is empty".to_string());
    }

    items
}

fn inspect_spine(
    document: &roxmltree::Document<'_>,
    manifest: &[ManifestItem],
    report: &mut InspectionReport,
) -> Vec<SpineItem> {
    let mut spine = Vec::new();
    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "itemref")
    {
        let idref = node
            .attribute("idref")
            .unwrap_or_default()
            .trim()
            .to_string();
        match manifest.iter().find(|item| item.id == idref) {
            Some(item) => spine.push(SpineItem {
                idref,
                href: item.href.clone(),
            }),
            None => report
                .errors
                .push(format!("spine idref {idref:?} has no manifest item")),
        }
    }

    if spine.is_empty() {
        report.errors.push("OPF spine is empty".to_string());
    }

    spine
}

fn inspect_nav_entries(
    manifest: &[ManifestItem],
    contents: &HashMap<String, Vec<u8>>,
    report: &mut InspectionReport,
) -> Vec<NavEntry> {
    let nav_items = manifest
        .iter()
        .filter(|item| item.properties.iter().any(|property| property == "nav"))
        .collect::<Vec<_>>();

    if nav_items.is_empty() {
        report
            .errors
            .push("manifest is missing an EPUB navigation document".to_string());
        return Vec::new();
    }
    if nav_items.len() > 1 {
        report
            .errors
            .push("manifest contains multiple navigation documents".to_string());
    }

    let nav_item = nav_items[0];
    report.required_entries.navigation_document = contents.contains_key(&nav_item.path);
    let Some(nav_xml) = read_text_entry(contents, &nav_item.path, report) else {
        return Vec::new();
    };

    let document = match roxmltree::Document::parse(&nav_xml) {
        Ok(document) => document,
        Err(error) => {
            report.errors.push(format!(
                "navigation document {} is not parseable XHTML: {error}",
                nav_item.path
            ));
            return Vec::new();
        }
    };

    let mut entries = Vec::new();
    for nav in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "nav")
    {
        let is_toc = nav
            .attributes()
            .any(|attribute| attribute.name() == "type" && attribute.value() == "toc")
            || nav.attribute("role") == Some("doc-toc")
            || nav.attribute("id") == Some("toc");

        if !is_toc {
            continue;
        }

        for link in nav
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "a")
        {
            let href = link
                .attribute("href")
                .unwrap_or_default()
                .trim()
                .to_string();
            let label = collapse_whitespace(&descendant_text(link));
            if href.is_empty() {
                report
                    .errors
                    .push("navigation link missing href".to_string());
            }
            if label.is_empty() {
                report
                    .errors
                    .push(format!("navigation link {href:?} has an empty label"));
            }
            entries.push(NavEntry { href, label });
        }
        break;
    }

    if entries.is_empty() {
        report
            .errors
            .push("navigation document has no table-of-contents links".to_string());
    }

    entries
}

fn inspect_content_chapters(
    spine: &[SpineItem],
    manifest: &[ManifestItem],
    report: &mut InspectionReport,
) -> Vec<ContentChapter> {
    let mut chapters = Vec::new();
    for itemref in spine {
        let Some(item) = manifest.iter().find(|item| item.id == itemref.idref) else {
            continue;
        };

        if item.properties.iter().any(|property| property == "nav") {
            continue;
        }

        if item.media_type != "application/xhtml+xml" {
            report.errors.push(format!(
                "spine item {} is not XHTML media type: {}",
                item.id, item.media_type
            ));
        }

        chapters.push(ContentChapter {
            idref: item.id.clone(),
            href: item.href.clone(),
            path: item.path.clone(),
        });
    }
    chapters
}

fn compare_nav_to_spine(
    nav_entries: &[NavEntry],
    content_chapters: &[ContentChapter],
    report: &mut InspectionReport,
) {
    if nav_entries.len() != content_chapters.len() {
        report.errors.push(format!(
            "navigation entry count {} does not match content spine chapter count {}",
            nav_entries.len(),
            content_chapters.len()
        ));
        return;
    }

    for (index, (nav_entry, chapter)) in nav_entries.iter().zip(content_chapters).enumerate() {
        if nav_entry.href != chapter.href {
            report.errors.push(format!(
                "navigation entry {index} href {:?} does not match spine href {:?}",
                nav_entry.href, chapter.href
            ));
        }
    }
}

fn inspect_xhtml_references(
    manifest: &[ManifestItem],
    contents: &HashMap<String, Vec<u8>>,
    report: &mut InspectionReport,
) {
    let xhtml_items = manifest
        .iter()
        .filter(|item| item.media_type == "application/xhtml+xml")
        .collect::<Vec<_>>();

    let mut id_map = HashMap::<String, HashSet<String>>::new();
    let mut reports = Vec::new();

    for item in &xhtml_items {
        let Some(xhtml) = read_text_entry(contents, &item.path, report) else {
            continue;
        };

        let Some(xhtml_report) = parse_xhtml(&item.path, &xhtml, report) else {
            continue;
        };
        id_map.insert(
            item.path.clone(),
            xhtml_report.ids.iter().cloned().collect::<HashSet<_>>(),
        );
        reports.push(xhtml_report);
    }

    let entry_names = contents.keys().cloned().collect::<HashSet<_>>();
    for xhtml_report in &reports {
        for reference in xhtml_report.hrefs.iter().chain(&xhtml_report.srcs) {
            inspect_reference(&xhtml_report.path, reference, &entry_names, &id_map, report);
        }
    }

    report.xhtml = reports;
}

fn parse_xhtml(path: &str, xhtml: &str, report: &mut InspectionReport) -> Option<XhtmlReport> {
    let document = match roxmltree::Document::parse(xhtml) {
        Ok(document) => document,
        Err(error) => {
            report
                .errors
                .push(format!("{path} is not parseable XHTML: {error}"));
            return None;
        }
    };

    let mut ids = Vec::new();
    let mut hrefs = Vec::new();
    let mut srcs = Vec::new();

    for node in document.descendants().filter(roxmltree::Node::is_element) {
        let tag_name = node.tag_name().name();
        if matches!(
            tag_name,
            "script" | "form" | "iframe" | "object" | "embed" | "applet"
        ) {
            report.errors.push(format!(
                "{path} contains unsafe active element <{tag_name}>"
            ));
        }

        for attribute in node.attributes() {
            let name = attribute.name();
            let value = attribute.value();
            if name.starts_with("on") {
                report
                    .errors
                    .push(format!("{path} contains event handler attribute {name}"));
            }
            if name == "id" {
                ids.push(value.to_string());
            }
            if name == "href" {
                if value
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("javascript:")
                {
                    report
                        .errors
                        .push(format!("{path} contains javascript: href"));
                }
                hrefs.push(value.to_string());
            }
            if name == "src" {
                srcs.push(value.to_string());
            }
        }
    }

    ids.sort();
    ids.dedup();

    Some(XhtmlReport {
        path: path.to_string(),
        ids,
        hrefs,
        srcs,
    })
}

fn inspect_reference(
    current_path: &str,
    reference: &str,
    entries: &HashSet<String>,
    id_map: &HashMap<String, HashSet<String>>,
    report: &mut InspectionReport,
) {
    let reference = reference.trim();
    if reference.is_empty() {
        report
            .errors
            .push(format!("{current_path} contains an empty reference"));
        return;
    }

    if is_permitted_external_reference(reference) {
        report.external_references.push(reference.to_string());
        return;
    }

    if has_unsupported_scheme(reference) {
        report.errors.push(format!(
            "{current_path} contains unsupported external reference {reference:?}"
        ));
        return;
    }

    let (path_part, fragment) = split_fragment(reference);
    let target_path = if path_part.is_empty() {
        current_path.to_string()
    } else if let Some(path) = resolve_epub_path(current_path, path_part) {
        path
    } else {
        report.errors.push(format!(
            "{current_path} reference {reference:?} cannot be resolved safely"
        ));
        return;
    };

    if !entries.contains(&target_path) {
        report.errors.push(format!(
            "{current_path} reference {reference:?} points to missing file {target_path}"
        ));
        return;
    }

    if let Some(fragment) = fragment {
        if fragment.is_empty() {
            report.errors.push(format!(
                "{current_path} reference {reference:?} has empty fragment"
            ));
            return;
        }
        if target_path.ends_with(".xhtml")
            && !id_map
                .get(&target_path)
                .is_some_and(|ids| ids.contains(fragment))
        {
            report.errors.push(format!(
                "{current_path} reference {reference:?} points to missing anchor {fragment:?} in {target_path}"
            ));
        }
    }
}

fn split_fragment(reference: &str) -> (&str, Option<&str>) {
    match reference.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (reference, None),
    }
}

fn is_permitted_external_reference(reference: &str) -> bool {
    let lower = reference.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

fn has_unsupported_scheme(reference: &str) -> bool {
    let Some(colon_index) = reference.find(':') else {
        return false;
    };
    let Some(slash_index) = reference.find('/') else {
        return true;
    };
    colon_index < slash_index
}

fn resolve_epub_path(base_file: &str, reference: &str) -> Option<String> {
    let (reference, _) = split_fragment(reference);
    if reference.is_empty() || reference.starts_with('/') || reference.contains('\\') {
        return None;
    }

    let mut parts = base_file
        .split('/')
        .take(base_file.split('/').count().saturating_sub(1))
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    for part in reference.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part.to_string()),
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn descendant_text(node: roxmltree::Node<'_, '_>) -> String {
    node.descendants()
        .filter_map(|descendant| descendant.text())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Cursor, Write},
        time::{SystemTime, UNIX_EPOCH},
    };

    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::inspect_epub;

    #[test]
    fn reports_missing_files_as_not_ok() {
        let report = inspect_epub(std::path::Path::new("does-not-exist.epub"));

        assert!(!report.ok);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("could not open"))
        );
    }

    #[test]
    fn accepts_minimal_inspectable_epub() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        writer.start_file("mimetype", options).unwrap();
        writer.write_all(b"application/epub+zip").unwrap();
        writer.add_directory("META-INF/", options).unwrap();
        writer
            .start_file("META-INF/container.xml", options)
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
            )
            .unwrap();
        writer.start_file("EPUB/package.opf", options).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">urn:test</dc:identifier>
    <dc:title>Test Book</dc:title>
    <dc:creator>Tester</dc:creator>
    <dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-05-10T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="chapter-1" href="chapters/chapter-1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chapter-1"/>
  </spine>
</package>"#,
            )
            .unwrap();
        writer.start_file("EPUB/nav.xhtml", options).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc" id="toc">
      <ol><li><a href="chapters/chapter-1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
            )
            .unwrap();
        writer
            .start_file("EPUB/chapters/chapter-1.xhtml", options)
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><section id="chapter-1"><h1>Chapter One</h1><p>Readable text.</p></section></body>
</html>"#,
            )
            .unwrap();

        let archive = writer.finish().unwrap().into_inner();
        let path = std::env::temp_dir().join(format!(
            "book-forge-inspector-{}.epub",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, archive).unwrap();

        let report = inspect_epub(&path);
        fs::remove_file(&path).unwrap();

        assert!(report.ok, "expected no errors, got {:?}", report.errors);
        assert!(report.required_entries.mimetype);
        assert!(report.required_entries.container_xml);
        assert!(report.required_entries.package_document);
        assert!(report.required_entries.navigation_document);
        assert_eq!(report.package.unwrap().content_chapters.len(), 1);
    }
}
