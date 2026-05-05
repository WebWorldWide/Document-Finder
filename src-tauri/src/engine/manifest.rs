//! Persistence logic for the `manifest.json` file which tracks all documents within a library.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::sources::Document;

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

impl From<&Document> for DocumentExt {
    fn from(d: &Document) -> Self {
        Self {
            title: d.title.clone(),
            url: d.url.clone(),
            source: d.source.clone(),
            authors: d.authors.clone(),
            year: d.year.clone(),
            abstract_: d.abstract_.clone(),
            identifier: d.identifier.clone(),
        }
    }
}

pub fn load(path: &Path, query: &str) -> Manifest {
    if !path.exists() {
        return Manifest {
            query: query.to_string(),
            documents: Vec::new(),
        };
    }
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| Manifest {
            query: query.to_string(),
            documents: Vec::new(),
        }),
        Err(_) => Manifest {
            query: query.to_string(),
            documents: Vec::new(),
        },
    }
}

pub fn save(path: &Path, manifest: &Manifest) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(manifest)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
