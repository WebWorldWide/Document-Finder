//! Model Context Protocol server: lets Claude (Desktop / Code) or any other
//! MCP client search and read the user's Document Finder libraries directly.
//!
//! Started by launching this same binary with `--mcp`; speaks JSON-RPC 2.0 over
//! newline-delimited stdio (the MCP stdio transport). Read-only: it can list,
//! search and read libraries under the configured library root, nothing else.
//! This is what replaces on-device AI models — the user's own AI does the
//! reasoning, Document Finder supplies clean, cited sources.

use crate::engine::library;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub const MCP_ARG: &str = "--mcp";
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// File (in the app config dir) where the app records the library root so
/// this headless process can find it.
pub fn root_marker_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(crate::APP_IDENTIFIER).join("library-root"))
}

pub fn save_library_root(root: &Path) {
    if let Some(p) = root_marker_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, root.to_string_lossy().as_bytes());
    }
}

fn library_root() -> Option<PathBuf> {
    root_marker_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| p.is_dir())
        .or_else(|| {
            crate::util::path_safety::documents_or_home_dir()
                .ok()
                .map(|d| d.join("Document Finder").join("library"))
        })
}

/// Resolve a library given by folder name or absolute path, confined to root.
fn resolve_library(root: &Path, which: &str) -> Result<PathBuf, String> {
    let candidate = if Path::new(which).is_absolute() {
        PathBuf::from(which)
    } else {
        root.join(which)
    };
    let p = crate::util::path_safety::safe_within_root(&candidate, root)
        .map_err(|_| format!("No library named '{which}'. Call list_libraries first."))?;
    if !p.join("library.db").exists() {
        return Err(format!("'{which}' is not a Document Finder library."));
    }
    Ok(p)
}

fn tools() -> Value {
    json!([
        {
            "name": "list_libraries",
            "description": "List the user's Document Finder research libraries (one per search). Returns each library's name, original search query, and document count.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "search_library",
            "description": "Full-text search over the extracted text of downloaded documents. Returns matching passages with document ids, section headings and page numbers. Omit `library` to search every library.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words to search for"},
                    "library": {"type": "string", "description": "Library name from list_libraries (optional)"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10}
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "list_documents",
            "description": "List the documents in one library with ids, citations and sizes, best matches first.",
            "inputSchema": {
                "type": "object",
                "properties": {"library": {"type": "string"}},
                "required": ["library"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "get_document",
            "description": "Read a document's clean Markdown text (with a metadata header). Long documents are truncated to `max_chars`; use get_passages to page through them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "library": {"type": "string"},
                    "doc_id": {"type": "integer"},
                    "max_chars": {"type": "integer", "minimum": 1000, "maximum": 400000, "default": 60000}
                },
                "required": ["library", "doc_id"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "get_passages",
            "description": "Read consecutive ~800-token passages of a document, starting at passage `from` (0-based).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "library": {"type": "string"},
                    "doc_id": {"type": "integer"},
                    "from": {"type": "integer", "minimum": 0, "default": 0},
                    "count": {"type": "integer", "minimum": 1, "maximum": 20, "default": 5}
                },
                "required": ["library", "doc_id"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true}
        }
    ])
}

fn text_result(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}]})
}

fn error_result(msg: String) -> Value {
    json!({"content": [{"type": "text", "text": msg}], "isError": true})
}

fn arg_str<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

fn call_tool(root: &Path, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "list_libraries" => {
            let libs = library::list_libraries(root);
            if libs.is_empty() {
                return Ok("No libraries yet. The user can create one by running a search in Document Finder.".into());
            }
            Ok(libs
                .iter()
                .map(|l| format!("- {} — \"{}\" ({} documents)", l.name, l.query, l.n_docs))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "search_library" => {
            let q = arg_str(args, "query").ok_or("`query` is required")?;
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .clamp(1, 50) as usize;
            let hits = match arg_str(args, "library") {
                Some(l) => library::search(&resolve_library(root, l)?, q, limit)?,
                None => library::search_all(root, q, limit),
            };
            if hits.is_empty() {
                return Ok(format!("No passages match \"{q}\"."));
            }
            Ok(hits
                .iter()
                .map(|h| {
                    let lib = Path::new(&h.library)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let page = match (h.page_start, h.page_end) {
                        (Some(a), Some(b)) if a != b => format!(", pp. {a}–{b}"),
                        (Some(a), _) => format!(", p. {a}"),
                        _ => String::new(),
                    };
                    format!(
                        "• {} [library: {lib}, doc_id: {}{}{}]\n  {}",
                        h.title,
                        h.doc_id,
                        h.heading
                            .as_ref()
                            .map(|s| format!(", section: {s}"))
                            .unwrap_or_default(),
                        page,
                        h.snippet.replace('\n', " ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n"))
        }
        "list_documents" => {
            let lib = resolve_library(
                root,
                arg_str(args, "library").ok_or("`library` is required")?,
            )?;
            let docs = library::list_docs(&lib)?;
            Ok(docs
                .iter()
                .map(|d| {
                    format!(
                        "- doc_id {}: {}{}",
                        d.id,
                        library::citation(d),
                        if d.md_path.is_none() {
                            " (no extracted text)"
                        } else {
                            ""
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "get_document" => {
            let lib = resolve_library(
                root,
                arg_str(args, "library").ok_or("`library` is required")?,
            )?;
            let id = args
                .get("doc_id")
                .and_then(Value::as_i64)
                .ok_or("`doc_id` is required")?;
            let max = args
                .get("max_chars")
                .and_then(Value::as_u64)
                .unwrap_or(60_000)
                .clamp(1000, 400_000) as usize;
            let doc = library::get_doc(&lib, id)?.ok_or("No document with that doc_id.")?;
            library::read_markdown(&doc, max).ok_or_else(|| {
                format!(
                    "No extracted text for this document{}. Citation: {}",
                    doc.extract_error
                        .as_ref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default(),
                    library::citation(&doc)
                )
            })
        }
        "get_passages" => {
            let lib = resolve_library(
                root,
                arg_str(args, "library").ok_or("`library` is required")?,
            )?;
            let id = args
                .get("doc_id")
                .and_then(Value::as_i64)
                .ok_or("`doc_id` is required")?;
            let from = args.get("from").and_then(Value::as_i64).unwrap_or(0).max(0);
            let count = args
                .get("count")
                .and_then(Value::as_i64)
                .unwrap_or(5)
                .clamp(1, 20);
            let doc = library::get_doc(&lib, id)?.ok_or("No document with that doc_id.")?;
            let chunks = library::chunks(&lib, id, from, count)?;
            if chunks.is_empty() {
                return Ok("No more passages.".into());
            }
            let mut out = format!("{}\n\n", library::citation(&doc));
            for c in chunks {
                out.push_str(&format!(
                    "### Passage {}{}{}\n\n{}\n\n",
                    c.ord,
                    c.heading.map(|h| format!(" — {h}")).unwrap_or_default(),
                    c.page_start
                        .map(|p| format!(" (p. {p})"))
                        .unwrap_or_default(),
                    c.text
                ));
            }
            Ok(out)
        }
        other => Err(format!("Unknown tool: {other}")),
    }
}

/// Handle one JSON-RPC message; `None` for notifications.
pub fn handle(root: Option<&Path>, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = id?; // notifications (no id) get no response
    let result = match method {
        "initialize" => {
            let asked = msg
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            let version = if PROTOCOL_VERSIONS.contains(&asked) {
                asked
            } else {
                PROTOCOL_VERSIONS[0]
            };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "document-finder", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Search and read the user's Document Finder research libraries. Start with list_libraries or search_library, then read documents with get_document or get_passages. Cite documents by their citation line."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = msg
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = msg
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            Ok(match root {
                None => error_result("Document Finder's library folder couldn't be found.".into()),
                Some(r) => match call_tool(r, name, &args) {
                    Ok(t) => text_result(t),
                    Err(e) => error_result(e),
                },
            })
        }
        "resources/list" => Ok(json!({"resources": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        _ => Err(json!({"code": -32601, "message": format!("Method not found: {method}")})),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    })
}

/// Run the stdio server until stdin closes. Logs go to stderr only.
pub fn serve_stdio() -> ! {
    let root = library_root();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let out: Vec<Value> = batch
                    .iter()
                    .filter_map(|m| handle(root.as_deref(), m))
                    .collect();
                (!out.is_empty()).then_some(Value::Array(out))
            }
            Ok(msg) => handle(root.as_deref(), &msg),
            Err(e) => Some(json!({
                "jsonrpc": "2.0", "id": null,
                "error": {"code": -32700, "message": format!("Parse error: {e}")}
            })),
        };
        if let Some(r) = reply {
            let _ = writeln!(stdout, "{r}");
            let _ = stdout.flush();
        }
    }
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_and_list_tools() {
        let r = handle(
            None,
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        )
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r["result"]["serverInfo"]["name"], "document-finder");
        let t = handle(None, &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        let names: Vec<&str> = t["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"search_library") && names.contains(&"get_document"));
        assert!(handle(
            None,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none());
        let bad = handle(None, &json!({"jsonrpc":"2.0","id":3,"method":"nope"})).unwrap();
        assert_eq!(bad["error"]["code"], -32601);
    }

    #[test]
    fn tools_refuse_paths_outside_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let r = handle(
            Some(&root),
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                    "params":{"name":"list_documents","arguments":{"library":"../../etc"}}}),
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        let ok = handle(
            Some(&root),
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_libraries","arguments":{}}}),
        )
        .unwrap();
        assert!(ok["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("No libraries yet"));
    }
}
