use std::io::{Cursor, Write};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{
    ConversionError,
    html::Chapter,
    metadata::SanitizedMetadata,
    text::{escape_xml_attr, escape_xml_text},
};

const NAV_PATH: &str = "EPUB/nav.xhtml";
const PACKAGE_PATH: &str = "EPUB/package.opf";

#[derive(Debug, Clone)]
pub(crate) struct EpubResource {
    pub(crate) path: String,
    pub(crate) media_type: String,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn generate_single_epub(
    metadata: &SanitizedMetadata,
    chapter: &Chapter,
) -> Result<Vec<u8>, ConversionError> {
    generate_epub(metadata, std::slice::from_ref(chapter), &[])
}

pub(crate) fn generate_epub(
    metadata: &SanitizedMetadata,
    chapters: &[Chapter],
    resources: &[EpubResource],
) -> Result<Vec<u8>, ConversionError> {
    let cursor = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(cursor);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("mimetype", stored)?;
    zip.write_all(b"application/epub+zip")?;

    zip.start_file("META-INF/container.xml", deflated)?;
    zip.write_all(container_xml().as_bytes())?;

    zip.start_file(PACKAGE_PATH, deflated)?;
    zip.write_all(package_opf(metadata, chapters, resources).as_bytes())?;

    zip.start_file(NAV_PATH, deflated)?;
    zip.write_all(nav_xhtml(&metadata.language, chapters).as_bytes())?;

    for (index, chapter) in chapters.iter().enumerate() {
        zip.start_file(format!("EPUB/{}", chapter_href(index)), deflated)?;
        zip.write_all(chapter.xhtml.as_bytes())?;
    }

    for resource in resources {
        zip.start_file(format!("EPUB/{}", resource.path), deflated)?;
        zip.write_all(&resource.bytes)?;
    }

    Ok(zip.finish()?.into_inner())
}

fn container_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#
}

fn package_opf(
    metadata: &SanitizedMetadata,
    chapters: &[Chapter],
    resources: &[EpubResource],
) -> String {
    let description = if metadata.description.is_empty() {
        String::new()
    } else {
        format!(
            "    <dc:description>{}</dc:description>\n",
            escape_xml_text(&metadata.description)
        )
    };

    let mut manifest_items = String::new();
    for index in 0..chapters.len() {
        manifest_items.push_str(&format!(
            "    <item id=\"chapter-{number}\" href=\"{href}\" media-type=\"application/xhtml+xml\"/>\n",
            number = index + 1,
            href = escape_xml_attr(&chapter_href(index))
        ));
    }
    for (index, resource) in resources.iter().enumerate() {
        manifest_items.push_str(&format!(
            "    <item id=\"resource-{number}\" href=\"{href}\" media-type=\"{media_type}\"/>\n",
            number = index + 1,
            href = escape_xml_attr(&resource.path),
            media_type = escape_xml_attr(&resource.media_type)
        ));
    }

    let mut spine_items = String::new();
    for index in 0..chapters.len() {
        spine_items.push_str(&format!(
            "    <itemref idref=\"chapter-{number}\"/>\n",
            number = index + 1
        ));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id" xml:lang="{language}">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">{identifier}</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:creator id="creator">{author}</dc:creator>
    <dc:language>{language}</dc:language>
{description}    <meta property="dcterms:modified">{modified}</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
{manifest_items}  </manifest>
  <spine>
{spine_items}  </spine>
</package>
"#,
        language = escape_xml_attr(&metadata.language),
        identifier = escape_xml_text(&metadata.identifier),
        title = escape_xml_text(&metadata.title),
        author = escape_xml_text(&metadata.author),
        description = description,
        modified = escape_xml_text(&metadata.modified),
        manifest_items = manifest_items,
        spine_items = spine_items
    )
}

fn nav_xhtml(language: &str, chapters: &[Chapter]) -> String {
    let mut entries = String::new();
    for (index, chapter) in chapters.iter().enumerate() {
        entries.push_str(&format!(
            "      <li><a href=\"{href}\">{chapter_title}</a></li>\n",
            href = escape_xml_attr(&chapter_href(index)),
            chapter_title = escape_xml_text(&chapter.title)
        ));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="{language}" xml:lang="{language}">
<head>
  <meta charset="utf-8" />
  <title>Table of Contents</title>
</head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>Table of Contents</h1>
    <ol>
{entries}    </ol>
  </nav>
</body>
</html>
"#,
        language = escape_xml_attr(language),
        entries = entries
    )
}

pub(crate) fn chapter_href(index: usize) -> String {
    format!("chapters/chapter-{}.xhtml", index + 1)
}
