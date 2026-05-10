use std::io::{Cursor, Write};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{
    ConversionError,
    html::Chapter,
    metadata::SanitizedMetadata,
    text::{escape_xml_attr, escape_xml_text},
};

const CHAPTER_PATH: &str = "EPUB/chapters/chapter-1.xhtml";
const NAV_PATH: &str = "EPUB/nav.xhtml";
const PACKAGE_PATH: &str = "EPUB/package.opf";

pub(crate) fn generate_single_epub(
    metadata: &SanitizedMetadata,
    chapter: &Chapter,
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
    zip.write_all(package_opf(metadata).as_bytes())?;

    zip.start_file(NAV_PATH, deflated)?;
    zip.write_all(nav_xhtml(&metadata.language, &chapter.title).as_bytes())?;

    zip.start_file(CHAPTER_PATH, deflated)?;
    zip.write_all(chapter.xhtml.as_bytes())?;

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

fn package_opf(metadata: &SanitizedMetadata) -> String {
    let description = if metadata.description.is_empty() {
        String::new()
    } else {
        format!(
            "    <dc:description>{}</dc:description>\n",
            escape_xml_text(&metadata.description)
        )
    };

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
    <item id="chapter-1" href="chapters/chapter-1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chapter-1"/>
  </spine>
</package>
"#,
        language = escape_xml_attr(&metadata.language),
        identifier = escape_xml_text(&metadata.identifier),
        title = escape_xml_text(&metadata.title),
        author = escape_xml_text(&metadata.author),
        description = description,
        modified = escape_xml_text(&metadata.modified)
    )
}

fn nav_xhtml(language: &str, chapter_title: &str) -> String {
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
      <li><a href="chapters/chapter-1.xhtml">{chapter_title}</a></li>
    </ol>
  </nav>
</body>
</html>
"#,
        language = escape_xml_attr(language),
        chapter_title = escape_xml_text(chapter_title)
    )
}
