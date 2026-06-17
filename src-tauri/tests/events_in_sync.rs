//! Asserts that every `df:` event the Rust backend defines (`EV_*` constants in
//! `src/events.rs`) has a consumer on the TypeScript side — either in the unified
//! `listenAll` EVENTS array or a dedicated `listen("df:…")` call. A new backend
//! event with no frontend listener is silently dropped at runtime; this catches
//! that at `cargo test` time.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `df:<name>` literal that appears in `src/events.rs` (the `EV_*` consts).
fn rust_events() -> BTreeSet<String> {
    let path = project_root().join("src").join("events.rs");
    let src = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    df_literals(&src)
}

/// Pull every `df:<snake_name>` token out of a source string.
fn df_literals(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = src.as_bytes();
    let needle = b"df:";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let start = i + needle.len();
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_ascii_lowercase() || bytes[j] == b'_') {
                j += 1;
            }
            if j > start {
                out.insert(String::from_utf8_lossy(&bytes[start..j]).to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Bare event names in the `const EVENTS = [ … ]` array of `src/lib/events.ts`
/// (consumed by `listenAll` as `df:<name>`).
fn ts_listen_all_events(events_ts: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    const MARKER: &str = "const EVENTS = [";
    let Some(start) = events_ts.find(MARKER) else {
        panic!("could not find `const EVENTS = [` in src/lib/events.ts");
    };
    let rest = &events_ts[start + MARKER.len()..];
    let end = rest.find(']').expect("unterminated EVENTS array");
    for raw in rest[..end].split(',') {
        let t = raw.trim().trim_matches(|c| c == '"' || c == '\'');
        if !t.is_empty() && t.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            out.insert(t.to_string());
        }
    }
    out
}

/// Recursively collect every `.ts`/`.tsx` file under `dir`.
fn ts_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            ts_files(&p, out);
        } else if matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("ts") | Some("tsx")
        ) {
            out.push(p);
        }
    }
}

#[test]
fn every_rust_event_has_a_ts_consumer() {
    let rust = rust_events();
    assert!(!rust.is_empty(), "found no df: events in events.rs");

    let src_dir = project_root().join("..").join("src");
    let events_ts =
        fs::read_to_string(src_dir.join("lib").join("events.ts")).expect("read src/lib/events.ts");

    // Consumers: the listenAll EVENTS array + every `df:…` literal anywhere in
    // the TS tree (dedicated listeners like pipeline_stage / meta_search_health).
    let mut consumed = ts_listen_all_events(&events_ts);
    let mut files = Vec::new();
    ts_files(&src_dir, &mut files);
    for f in files {
        if let Ok(s) = fs::read_to_string(&f) {
            consumed.extend(df_literals(&s));
        }
    }

    let orphans: Vec<&String> = rust.difference(&consumed).collect();
    assert!(
        orphans.is_empty(),
        "Rust emits these df: events with no TypeScript consumer (they'd be silently dropped) — \
         add them to the EVENTS array in src/lib/events.ts or a dedicated listen():\n  - {}",
        orphans
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  - ")
    );
}
