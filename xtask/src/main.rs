//! Repository automation (`cargo xtask <command>`, alias in `.cargo/config.toml`).
//!
//! `check-deps` enforces the architecture rules (docs/ARCHITECTURE.md §2,
//! CONTRIBUTING.md rule 1):
//! 1. Project crates only depend on the project crates the architecture allows.
//! 2. Only `app` uses the per-OS crates, and only behind a `cfg(target_os)` target;
//!    OS binding crates stay out of the OS-independent crates.
//! 3. `cfg(target_os)` / `cfg(windows)` / `cfg(unix)` appear in source only in
//!    `crates/platform-*` and `crates/app/src/platform_init.rs`.

#![allow(clippy::print_stdout)] // a command-line tool

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;

/// Project crate → project crates it may depend on (normal and build dependencies).
const ALLOWED: &[(&str, &[&str])] = &[
    (
        "desktop-pilot",
        &[
            "dp-core",
            "dp-llm",
            "dp-config",
            "dp-tools",
            "dp-platform",
            "dp-voice",
            "dp-platform-windows",
            "dp-platform-macos",
            "dp-platform-linux",
        ],
    ),
    ("dp-core", &["dp-llm", "dp-platform", "dp-config"]),
    ("dp-tools", &["dp-platform", "dp-core"]),
    ("dp-llm", &["dp-config"]),
    ("dp-config", &[]),
    ("dp-platform", &[]),
    ("dp-platform-windows", &["dp-platform"]),
    ("dp-platform-macos", &["dp-platform"]),
    ("dp-platform-linux", &["dp-platform"]),
    ("dp-test-support", &["dp-core", "dp-llm", "dp-platform"]),
    ("dp-voice", &[]),
    ("xtask", &[]),
];

const PLATFORM_IMPLS: &[&str] = &[
    "dp-platform-windows",
    "dp-platform-macos",
    "dp-platform-linux",
];

/// Crates that must stay free of OS-specific code.
const OS_FREE: &[&str] = &[
    "dp-core",
    "dp-llm",
    "dp-tools",
    "dp-config",
    "dp-platform",
    "dp-voice",
];

/// OS binding crates (a name equal to one of these or starting with "<name>-").
const OS_CRATES: &[&str] = &[
    "windows",
    "windows-sys",
    "winapi",
    "objc2",
    "block2",
    "core-foundation",
    "core-graphics",
    "cocoa",
    "x11",
    "x11rb",
    "xcb",
    "wayland-client",
    "ashpd",
    "zbus",
    "atspi",
    "global-hotkey",
    "tray-icon",
    "muda",
];

fn is_os_crate(name: &str) -> bool {
    OS_CRATES
        .iter()
        .any(|c| name == *c || name.starts_with(&format!("{c}-")))
}

/// `cargo metadata` for the workspace members only.
fn metadata() -> Result<Value, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata output: {e}"))
}

/// Rules 1 and 2 on the dependency graph. Returns the violations.
fn check_graph(meta: &Value) -> Vec<String> {
    let allowed: BTreeMap<&str, BTreeSet<&str>> = ALLOWED
        .iter()
        .map(|(k, v)| (*k, v.iter().copied().collect()))
        .collect();
    let mut problems = Vec::new();
    let packages = meta["packages"].as_array().cloned().unwrap_or_default();
    let members: BTreeSet<String> = packages
        .iter()
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect();
    for pkg in &packages {
        let name = pkg["name"].as_str().unwrap_or_default();
        let Some(ok) = allowed.get(name) else {
            problems.push(format!(
                "{name}: not in the architecture rules (add it to xtask ALLOWED)"
            ));
            continue;
        };
        for dep in pkg["dependencies"].as_array().into_iter().flatten() {
            let dep_name = dep["name"].as_str().unwrap_or_default();
            let dev = dep["kind"].as_str() == Some("dev");
            if members.contains(dep_name) && !dev && !ok.contains(dep_name) {
                problems.push(format!("{name} must not depend on {dep_name}"));
            }
            if dev
                && members.contains(dep_name)
                && dep_name != "dp-test-support"
                && !ok.contains(dep_name)
            {
                problems.push(format!(
                    "{name}: dev-dependency on {dep_name} is not allowed"
                ));
            }
            if PLATFORM_IMPLS.contains(&dep_name)
                && dep["target"]
                    .as_str()
                    .is_none_or(|t| !t.contains("target_os"))
            {
                problems.push(format!(
                    "{name} uses {dep_name} without a cfg(target_os) target"
                ));
            }
            if OS_FREE.contains(&name) && is_os_crate(dep_name) {
                problems.push(format!("{name} must not use the OS crate {dep_name}"));
            }
        }
    }
    problems
}

/// Text that marks OS-specific conditional code (built so this file does not match).
fn os_cfg_markers() -> [String; 4] {
    [
        format!("{}_os", "target"),
        format!("{}_family", "target"),
        format!("cfg({})", "windows"),
        format!("cfg({})", "unix"),
    ]
}

/// Whether OS-specific code is allowed in this source file.
fn may_have_os_code(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    s.starts_with("crates/platform-")
        || s == "crates/app/src/platform_init.rs"
        || s.starts_with("xtask/")
}

/// Rule 3 on the given source text. Returns (line number, line) of violations.
fn os_cfg_lines(text: &str) -> Vec<(usize, String)> {
    let markers = os_cfg_markers();
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            let code = line.split("//").next().unwrap_or_default();
            markers.iter().any(|m| code.contains(m.as_str()))
        })
        .map(|(i, line)| (i + 1, line.trim().to_owned()))
        .collect()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name != "target" && name != ".git" {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn check_sources(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    let mut problems = Vec::new();
    for file in files {
        let rel = file.strip_prefix(root).unwrap_or(&file).to_path_buf();
        if may_have_os_code(&rel) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (line, code) in os_cfg_lines(&text) {
            problems.push(format!(
                "{}:{line}: OS-specific cfg outside platform-* / platform_init.rs: {code}",
                rel.display()
            ));
        }
    }
    problems
}

fn check_deps() -> ExitCode {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut problems = match metadata() {
        Ok(meta) => check_graph(&meta),
        Err(e) => {
            println!("check-deps: {e}");
            return ExitCode::FAILURE;
        }
    };
    problems.extend(check_sources(&root));
    if problems.is_empty() {
        println!("check-deps: OK (dependency rules and OS-code boundaries hold)");
        ExitCode::SUCCESS
    } else {
        for p in &problems {
            println!("check-deps: {p}");
        }
        println!("check-deps: {} problem(s)", problems.len());
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("check-deps") => check_deps(),
        _ => {
            println!("usage: cargo xtask check-deps");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_os_cfg_in_code_but_not_in_comments() {
        let marker = format!("#[cfg({}_os = \"macos\")]", "target");
        let text = format!("fn a() {{}}\n{marker}\nfn b() {{}}\n// {marker}\n");
        let found = os_cfg_lines(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 2);
    }

    #[test]
    fn os_code_allowed_only_in_platform_crates() {
        assert!(may_have_os_code(Path::new(
            "crates/platform-macos/src/lib.rs"
        )));
        assert!(may_have_os_code(Path::new(
            "crates/app/src/platform_init.rs"
        )));
        assert!(!may_have_os_code(Path::new("crates/app/src/gui/mod.rs")));
        assert!(!may_have_os_code(Path::new("crates/platform/src/lib.rs")));
        assert!(!may_have_os_code(Path::new("crates/voice/src/stt.rs")));
    }

    #[test]
    fn graph_rules_catch_bad_edges() {
        let meta = serde_json::json!({"packages": [
            {"name": "dp-llm", "dependencies": [{"name": "dp-core", "kind": null}]},
            {"name": "dp-core", "dependencies": [{"name": "windows", "kind": null}]},
            {"name": "desktop-pilot", "dependencies": [
                {"name": "dp-platform-macos", "kind": null, "target": null}
            ]},
            {"name": "dp-platform-macos", "dependencies": []}
        ]});
        let problems = check_graph(&meta);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("dp-llm must not depend on dp-core"))
        );
        assert!(problems.iter().any(|p| p.contains("OS crate windows")));
        assert!(
            problems
                .iter()
                .any(|p| p.contains("without a cfg(target_os)"))
        );
    }

    #[test]
    fn os_crate_names() {
        assert!(is_os_crate("objc2-app-kit"));
        assert!(is_os_crate("windows"));
        assert!(!is_os_crate("tokio"));
        assert!(!is_os_crate("cpal"));
    }
}
