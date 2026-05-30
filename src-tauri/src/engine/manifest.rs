//! Persistence logic for the `manifest.json` file which tracks all documents within a library.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub query: String,
    #[serde(default)]
    pub documents: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    #[serde(flatten)]
    pub doc: DocumentExt,
    pub local_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_error: Option<String>,
}

/// Same shape as `Document` but with an `abstract` JSON key (not `abstract_`).
/// Stored separately so the wire JSON keeps Python-compatible field names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentExt {
    pub title: String,
    pub url: String,
    pub source: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    #[serde(default, rename = "abstract", skip_serializing_if = "Option::is_none")]
    pub abstract_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
}

// NOTE: `Manifest` is read (never written) by the library export path in
// `commands.rs`, which deserializes a legacy `manifest.json` directly via
// serde. The app itself persists to SQLite, so there are no `save`/`load`
// helpers here anymore.
