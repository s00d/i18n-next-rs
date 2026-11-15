//! Released together (DESIGN 8b.13): every workspace member, the npm package
//! `tauri-plugin-i18n-next-api` and `[workspace.package] version` carry one version, and
//! the plugin pins the crate with `=x.y.z`. This test fails when they drift apart.
//!
//! It reads the workspace around the crate (root Cargo.toml, the other members, the
//! plugin's package.json), so it is a workspace check, not a test of the published crate:
//! `exclude` in Cargo.toml keeps it out of the package, where those files do not exist.
#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::Path;

fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Body of the `[name]` table, up to the next table header.
fn section(toml: &str, name: &str) -> String {
    let header = format!("[{name}]");
    let mut lines = toml.lines().skip_while(|l| l.trim() != header);
    assert!(lines.next().is_some(), "no {header} table");
    let body: Vec<&str> = lines
        .take_while(|l| !l.trim_start().starts_with('['))
        .collect();
    body.join("\n")
}

/// Right-hand side of the `key = ...` line in `body`, trimmed.
fn value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    body.lines().find_map(|l| {
        let rest = l.trim_start().strip_prefix(key)?.trim_start();
        Some(rest.strip_prefix('=')?.trim())
    })
}

/// The members listed in `[workspace] members = [...]`.
fn members(root: &str) -> Vec<String> {
    let ws = section(root, "workspace");
    let list = ws.split("members").nth(1).expect("[workspace] members");
    let list = &list[list.find('[').unwrap() + 1..list.find(']').unwrap()];
    list.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[test]
fn versions_are_in_sync() {
    let version = env!("CARGO_PKG_VERSION");
    let quoted = format!("\"{version}\"");
    let root = read(&workspace().join("Cargo.toml"));
    assert_eq!(
        value(&section(&root, "workspace.package"), "version"),
        Some(quoted.as_str()),
        "[workspace.package] version"
    );
    assert_eq!(i18n_next::version(), version);

    // Every member takes the workspace version (or spells out the same one).
    let members = members(&root);
    assert!(
        members.iter().any(|m| m == "crates/tauri-plugin-i18n-next"),
        "members: {members:?}"
    );
    for member in &members {
        let toml = read(&workspace().join(member).join("Cargo.toml"));
        let package = section(&toml, "package");
        let ok = value(&package, "version.workspace") == Some("true")
            || value(&package, "version") == Some(quoted.as_str());
        assert!(
            ok,
            "{member}: [package] must use `version.workspace = true` (workspace version {version})"
        );
    }

    let plugin_dir = workspace().join("crates/tauri-plugin-i18n-next");
    let plugin = read(&plugin_dir.join("Cargo.toml"));
    let pin = format!("={version}");
    let dep = plugin
        .lines()
        .find(|l| l.starts_with("i18n-next "))
        .expect("plugin depends on i18n-next");
    assert!(
        dep.contains(&format!("version = \"{pin}\"")),
        "plugin must pin i18n-next with \"{pin}\": {dep}"
    );

    let pkg = read(&plugin_dir.join("package.json"));
    let line = pkg
        .lines()
        .find(|l| l.trim_start().starts_with("\"version\""))
        .expect("package.json has a version");
    assert!(
        line.contains(&quoted),
        "package.json version differs from {version}: {line}"
    );
}

/// Each crate ships plain copies of the root LICENSE and CHANGELOG.md, not symlinks: a
/// `core.symlinks=false` checkout (the Windows default) would package the link text.
/// The release flow (`.versionrc.json` postchangelog) refreshes the CHANGELOG copies.
#[test]
fn license_and_changelog_copies_match_the_root() {
    for file in ["LICENSE", "CHANGELOG.md"] {
        let root = fs::read(workspace().join(file)).unwrap();
        for krate in ["i18n-next", "tauri-plugin-i18n-next"] {
            let path = workspace().join("crates").join(krate).join(file);
            let copy = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(
                copy == root,
                "{} differs from the root {file}: copy the root file over it",
                path.display()
            );
        }
    }
}
