use std::{fs::File, path::Path};

use serde::Serialize;
use zip::ZipArchive;

#[derive(Debug, Serialize)]
pub struct InspectionReport {
    pub path: String,
    pub ok: bool,
    pub entry_count: usize,
    pub entries: Vec<String>,
    pub required_entries: RequiredEntries,
    pub errors: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct RequiredEntries {
    pub mimetype: bool,
    pub container_xml: bool,
}

pub fn inspect_epub(path: &Path) -> InspectionReport {
    let mut report = InspectionReport {
        path: path.display().to_string(),
        ok: false,
        entry_count: 0,
        entries: Vec::new(),
        required_entries: RequiredEntries::default(),
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

    for index in 0..archive.len() {
        match archive.by_index(index) {
            Ok(entry) => report.entries.push(entry.name().replace('\\', "/")),
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

    if !report.required_entries.mimetype {
        report
            .errors
            .push("missing required mimetype entry".to_string());
    }

    if !report.required_entries.container_xml {
        report
            .errors
            .push("missing required META-INF/container.xml entry".to_string());
    }

    report.ok = report.errors.is_empty();
    report
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
    fn accepts_minimal_epub_container() {
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
    }
}
