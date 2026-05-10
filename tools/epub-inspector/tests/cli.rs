use std::process::Command;

#[test]
fn help_documents_stable_command_shape() {
    let output = Command::new(env!("CARGO_BIN_EXE_inspect-epub"))
        .arg("--help")
        .output()
        .expect("inspect-epub binary should run");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help output should be UTF-8");
    assert!(stdout.contains("usage: inspect-epub [--json] <epub-file>"));
}
