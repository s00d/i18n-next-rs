//! Released together (DESIGN 8b.13): `i18n-next`, `tauri-plugin-i18n-next` and the npm
//! package `tauri-plugin-i18n-next-api` carry one version, and the plugin pins the crate
//! with `=x.y.z`. This test fails when they drift apart.
#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::Path;

fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
}

/// Value of the first `key = "..."` line after the `[section]` header.
fn toml_value(toml: &str, section: &str, key: &str) -> Option<String> {
    let body = toml.split(&format!("\n[{section}]\n")).nth(1)?;
    let body = body.split("\n[").next()?;
    body.lines().find_map(|l| {
        let rest = l.strip_prefix(key)?.trim_start().strip_prefix('=')?;
        let rest = rest.split('"').nth(1)?;
        Some(rest.to_owned())
    })
}

#[test]
fn versions_are_in_sync() {
    let version = env!("CARGO_PKG_VERSION");
    let root = fs::read_to_string(workspace().join("Cargo.toml")).unwrap();
    assert_eq!(
        toml_value(&root, "workspace.package", "version").as_deref(),
        Some(version),
        "[workspace.package] version"
    );
    assert_eq!(i18n_next::version(), version);

    let plugin_dir = workspace().join("crates/tauri-plugin-i18n-next");
    let plugin = fs::read_to_string(plugin_dir.join("Cargo.toml")).unwrap();
    let pin = format!("={version}");
    let dep = plugin
        .lines()
        .find(|l| l.starts_with("i18n-next "))
        .expect("plugin depends on i18n-next");
    assert!(
        dep.contains(&format!("version = \"{pin}\"")),
        "plugin must pin i18n-next with \"{pin}\": {dep}"
    );

    // Created with the guest-js package; checked once it exists.
    if let Ok(pkg) = fs::read_to_string(plugin_dir.join("package.json")) {
        let line = pkg
            .lines()
            .find(|l| l.trim_start().starts_with("\"version\""))
            .expect("package.json has a version");
        assert!(
            line.contains(&format!("\"{version}\"")),
            "package.json version differs from {version}: {line}"
        );
    }
}
