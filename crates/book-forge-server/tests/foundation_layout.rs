use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("server crate should be nested under crates/")
        .to_path_buf()
}

fn assert_exists(root: &Path, relative_path: &str) {
    assert!(
        root.join(relative_path).exists(),
        "expected repository path to exist: {relative_path}"
    );
}

#[test]
fn workspace_boundaries_are_present() {
    let root = repo_root();

    for path in [
        "Cargo.toml",
        "crates/book-forge-converter/Cargo.toml",
        "crates/book-forge-server/Cargo.toml",
        "frontend/package.json",
    ] {
        assert_exists(&root, path);
    }
}

#[test]
fn validation_scripts_match_service_command_names() {
    let root = repo_root();

    for command in ["install", "format", "lint", "typecheck", "test", "build"] {
        assert_exists(&root, &format!("scripts/commands/{command}"));
    }
}

#[test]
fn deterministic_fixture_sets_cover_validation_scenarios() {
    let root = repo_root();

    for path in [
        "fixtures/html/single-page/index.html",
        "fixtures/html/crawl-graph/index.html",
        "fixtures/html/images/index.html",
        "fixtures/html/unsafe-html/index.html",
        "fixtures/html/failed-resources/index.html",
        "fixtures/html/redirects/routes.json",
        "fixtures/html/oversized-slow/routes.json",
        "fixtures/html/semantic-content/index.html",
        "fixtures/images/logo.svg",
    ] {
        assert_exists(&root, path);
    }
}

#[test]
fn epub_inspector_has_stable_entrypoint() {
    let root = repo_root();

    for path in ["scripts/inspect-epub", "tools/epub-inspector/Cargo.toml"] {
        assert_exists(&root, path);
    }
}
