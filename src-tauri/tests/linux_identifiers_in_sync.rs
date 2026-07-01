//! Asserts the three Linux identifier strings this app uses — the lowercase
//! run-log/state-dir name, the Tauri app identifier, and the Flatpak app id —
//! stay correct everywhere they're hardcoded outside the Rust crate itself
//! (a shell script and a YAML manifest can't be cross-checked by the
//! compiler). A rename of any one of these that misses a sibling location
//! breaks `scripts/uninstall.sh` silently (it stops finding the right
//! directories) without breaking a `cargo build`.

use std::fs;
use std::path::PathBuf;

fn project_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at src-tauri/ when running tests.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    project_root().join("..")
}

/// Lowercase, hyphenated: the Linux run-log/state-dir folder name.
const STATE_DIR_NAME: &str = "document-finder";
/// Lowercase, reverse-DNS, no hyphen: the Tauri `identifier` (drives
/// app_data_dir()/app_local_data_dir()/app_config_dir()/app_cache_dir()).
const TAURI_IDENTIFIER: &str = "com.webworldwide.documentfinder";
/// Mixed-case, reverse-DNS: the Flatpak app id.
const FLATPAK_APP_ID: &str = "com.webworldwide.DocumentFinder";

#[test]
fn runlog_state_dir_matches_uninstall_sh() {
    let runlog = project_root().join("src/engine/runlog.rs");
    let src =
        fs::read_to_string(&runlog).unwrap_or_else(|e| panic!("read {}: {e}", runlog.display()));
    assert!(
        src.contains(&format!("\"{STATE_DIR_NAME}\"")),
        "src-tauri/src/engine/runlog.rs no longer contains the literal \"{STATE_DIR_NAME}\" \
         in its Linux state_dir() join. If you renamed it, also update \
         scripts/uninstall.sh's XDG_STATE_HOME rm_if line and any other reference \
         to this identifier."
    );

    let uninstall = repo_root().join("scripts/uninstall.sh");
    let src = fs::read_to_string(&uninstall)
        .unwrap_or_else(|e| panic!("read {}: {e}", uninstall.display()));
    assert!(
        src.contains(&format!("/{STATE_DIR_NAME}\"")),
        "scripts/uninstall.sh's XDG_STATE_HOME line no longer matches \"{STATE_DIR_NAME}\" \
         from src-tauri/src/engine/runlog.rs. Update one to match the other."
    );
}

#[test]
fn tauri_identifier_matches_uninstall_sh() {
    let conf = project_root().join("tauri.conf.json");
    let src = fs::read_to_string(&conf).unwrap_or_else(|e| panic!("read {}: {e}", conf.display()));
    let json: serde_json::Value =
        serde_json::from_str(&src).unwrap_or_else(|e| panic!("parse {}: {e}", conf.display()));
    let identifier = json["identifier"]
        .as_str()
        .unwrap_or_else(|| panic!("no string \"identifier\" field in {}", conf.display()));
    assert_eq!(
        identifier, TAURI_IDENTIFIER,
        "src-tauri/tauri.conf.json's \"identifier\" changed. Also update \
         scripts/uninstall.sh's `id=` line and any other reference to this \
         identifier (it drives app_data_dir()/app_local_data_dir()/\
         app_config_dir()/app_cache_dir() resolution)."
    );

    let uninstall = repo_root().join("scripts/uninstall.sh");
    let src = fs::read_to_string(&uninstall)
        .unwrap_or_else(|e| panic!("read {}: {e}", uninstall.display()));
    assert!(
        src.contains(&format!("id=\"{TAURI_IDENTIFIER}\"")),
        "scripts/uninstall.sh's `id=` line no longer matches tauri.conf.json's \
         \"identifier\" ({TAURI_IDENTIFIER}). Update one to match the other."
    );
}

#[test]
fn flatpak_app_id_matches_uninstall_sh() {
    let manifest = repo_root().join("packaging/flatpak/com.webworldwide.DocumentFinder.yml");
    let src = fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
    assert!(
        src.lines()
            .any(|l| l.trim() == format!("id: {FLATPAK_APP_ID}")),
        "packaging/flatpak/com.webworldwide.DocumentFinder.yml's `id:` field \
         no longer matches \"{FLATPAK_APP_ID}\". Also update scripts/uninstall.sh's \
         Flatpak rm_if line (~/.var/app/{FLATPAK_APP_ID}) and any other reference."
    );

    let uninstall = repo_root().join("scripts/uninstall.sh");
    let src = fs::read_to_string(&uninstall)
        .unwrap_or_else(|e| panic!("read {}: {e}", uninstall.display()));
    assert!(
        src.contains(FLATPAK_APP_ID),
        "scripts/uninstall.sh no longer references the Flatpak app id \
         \"{FLATPAK_APP_ID}\". Update it to match packaging/flatpak/com.webworldwide.DocumentFinder.yml's `id:` field."
    );
}
