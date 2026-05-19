//! Asserts that the Tauri command allowlist (`permissions/app.toml`) stays
//! in sync with the commands actually registered via
//! `tauri::generate_handler!` in `src/lib.rs`.
//!
//! Tauri 2 silently denies any frontend `invoke()` call whose target command
//! is not on a permission's allowlist — the user only sees a runtime
//! "Command X not allowed by ACL" message. This test catches the
//! divergence at `cargo test` time so the bug never reaches the user.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn project_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at src-tauri/ when running tests.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Pull command idents from inside `tauri::generate_handler![...]`.
/// Resilient to whitespace, line breaks, the `commands::` path prefix, and
/// trailing commas. Comments inside the macro are ignored.
fn registered_commands() -> BTreeSet<String> {
    let lib_rs = project_root().join("src").join("lib.rs");
    let src =
        fs::read_to_string(&lib_rs).unwrap_or_else(|e| panic!("read {}: {}", lib_rs.display(), e));

    // Locate the macro invocation start. The codebase has exactly one
    // `tauri::generate_handler!` call.
    let start_marker = "tauri::generate_handler![";
    let start = src
        .find(start_marker)
        .unwrap_or_else(|| panic!("could not find `{}` in {}", start_marker, lib_rs.display()))
        + start_marker.len();
    let rest = &src[start..];
    let end = rest.find(']').unwrap_or_else(|| {
        panic!(
            "unterminated generate_handler! macro in {}",
            lib_rs.display()
        )
    });
    let inside = &rest[..end];

    // Strip any line comments that might appear inside the macro args.
    let cleaned: String = inside
        .lines()
        .map(|line| line.split("//").next().unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");

    cleaned
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            // Drop the `commands::` (or any other path) prefix; we only care
            // about the bare ident the frontend invokes by name.
            s.rsplit("::").next().unwrap_or(s).trim().to_string()
        })
        .collect()
}

/// Pull the `commands.allow = [...]` strings from `permissions/app.toml`.
fn allowed_commands() -> BTreeSet<String> {
    let toml_path = project_root().join("permissions").join("app.toml");
    let src = fs::read_to_string(&toml_path)
        .unwrap_or_else(|e| panic!("read {}: {}", toml_path.display(), e));

    // Find the array literal after `commands.allow`.
    let start_marker = "commands.allow";
    let after_key = src
        .find(start_marker)
        .unwrap_or_else(|| panic!("no `commands.allow` in {}", toml_path.display()));
    let rest = &src[after_key..];
    let bracket = rest
        .find('[')
        .unwrap_or_else(|| panic!("no `[` after commands.allow in {}", toml_path.display()));
    let end = rest[bracket..]
        .find(']')
        .unwrap_or_else(|| panic!("unterminated commands.allow in {}", toml_path.display()));
    let inside = &rest[bracket + 1..bracket + end];

    let mut out = BTreeSet::new();
    for raw in inside.split(',') {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Strip surrounding "..." or '...'.
        let unquoted = trimmed
            .trim_start_matches(['"', '\''])
            .trim_end_matches(['"', '\'']);
        if !unquoted.is_empty() {
            out.insert(unquoted.to_string());
        }
    }
    out
}

#[test]
fn permissions_match_generate_handler() {
    let registered = registered_commands();
    let allowed = allowed_commands();

    let missing_from_toml: Vec<&String> = registered.difference(&allowed).collect();
    let extra_in_toml: Vec<&String> = allowed.difference(&registered).collect();

    if !missing_from_toml.is_empty() || !extra_in_toml.is_empty() {
        let mut msg = String::from(
            "permissions/app.toml is out of sync with tauri::generate_handler! in src/lib.rs.\n",
        );
        if !missing_from_toml.is_empty() {
            msg.push_str(&format!(
                "  Registered in Rust but missing from app.toml (frontend calls will fail with \"Command X not allowed by ACL\"):\n    - {}\n",
                missing_from_toml
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n    - ")
            ));
        }
        if !extra_in_toml.is_empty() {
            msg.push_str(&format!(
                "  Listed in app.toml but not registered in Rust (dead allowlist entries):\n    - {}\n",
                extra_in_toml
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n    - ")
            ));
        }
        msg.push_str("\nFix by editing src-tauri/permissions/app.toml so commands.allow matches the macro contents exactly.");
        panic!("{}", msg);
    }
}
