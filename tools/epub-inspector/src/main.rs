use std::{path::PathBuf, process::ExitCode};

use book_forge_epub_inspector::inspect_epub;

const USAGE: &str = "usage: inspect-epub [--json] <epub-file>";

fn main() -> ExitCode {
    let mut json = false;
    let mut path = None::<PathBuf>;

    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--json" => json = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            value if value.starts_with('-') => {
                eprintln!("unknown option: {value}");
                eprintln!("{USAGE}");
                return ExitCode::from(64);
            }
            value => {
                if path.replace(PathBuf::from(value)).is_some() {
                    eprintln!("only one EPUB file may be inspected");
                    eprintln!("{USAGE}");
                    return ExitCode::from(64);
                }
            }
        }
    }

    let Some(path) = path else {
        eprintln!("{USAGE}");
        return ExitCode::from(64);
    };

    let report = inspect_epub(&path);

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(output) => println!("{output}"),
            Err(error) => {
                eprintln!("could not serialize inspection report: {error}");
                return ExitCode::from(70);
            }
        }
    } else {
        println!("EPUB: {}", report.path);
        println!("OK: {}", report.ok);
        println!("Entries: {}", report.entry_count);
        for error in &report.errors {
            println!("ERROR: {error}");
        }
    }

    if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}
