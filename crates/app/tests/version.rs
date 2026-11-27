//! `herdr-frontend --version` prints the released version: the one changesets keeps in the root
//! package.json and `scripts/sync-version` copies into the workspace Cargo.toml (`docs/release.md`).

use std::path::Path;
use std::process::Command;

#[test]
fn version_prints_the_released_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let package: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("package.json")).unwrap()).unwrap();
    let released = package["version"]
        .as_str()
        .expect("package.json has a version");

    let output = Command::new(env!("CARGO_BIN_EXE_herdr-frontend"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("herdr-frontend {released}\n")
    );
}
