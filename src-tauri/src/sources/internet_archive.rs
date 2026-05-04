//! Internet Archive — millions of books, esp. humanities & public domain.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use super::{get_with_retry, Document, Source};

const SEARCH: &str = "https://archive.org/advancedsearch.php";

pub struct InternetArchiveSource {
    client: Arc<reqwest::Client>,
}

impl InternetArchiveSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct Resp {
    #[serde(default)]
    response: Inner,
}

#[derive(Debug, Default, Deserialize)]
struct Inner {
    #[serde(default)]
    docs: Vec<Value>,
}

fn coerce_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn coerce_string_list(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

#[async_trait]
impl Source for InternetArchiveSource {
    fn name(&self) -> &'static str {
        "internet_archive"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        // IA's format field uses values like "Text PDF" — `format:pdf` matched
        // very few items. Filter to texts only and let the metadata lookup
        // confirm a PDF exists.
        let q = format!("{} AND mediatype:texts", keywords.join(" AND "));
        stream::unfold((1u32, 0usize, false), move |(page, yielded, done)| {
            let client = client.clone();
            let q = q.clone();
            async move {
                if done || yielded >= limit {
                    return None;
                }
                let per_page: usize = 50;
                let params = [
                    ("q", q),
                    ("fl[]", "identifier".to_string()),
                    ("fl[]", "title".to_string()),
                    ("fl[]", "creator".to_string()),
                    ("fl[]", "year".to_string()),
                    ("fl[]", "description".to_string()),
                    ("fl[]", "format".to_string()),
                    ("rows", per_page.to_string()),
                    ("page", page.to_string()),
                    ("output", "json".to_string()),
                    ("sort[]", "downloads desc".to_string()),
                ];
                let resp = match get_with_retry(&client, SEARCH, &params).await {
                    Ok(r) => r,
                    Err(e) => {
                        return Some((
                            Err(anyhow::anyhow!("archive.org search failed: {e}")),
                            (page, yielded, true),
                        ));
                    }
                };
                let data: Resp = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => {
                        return Some((
                            Err(anyhow::anyhow!("archive.org returned malformed JSON: {e}")),
                            (page, yielded, true),
                        ));
                    }
                };
                let docs_in = data.response.docs;
                if docs_in.is_empty() {
                    return None;
                }
                let n = docs_in.len();
                let next_done = n < per_page;
                let mut docs = Vec::with_capacity(n);
                for d in docs_in {
                    let Some(ident) = d.get("identifier").and_then(coerce_string) else {
                        continue;
                    };
                    // Filter to items whose format list mentions PDF — avoids
                    // hammering /download with 404s for items that only have
                    // image scans or DjVu.
                    let formats = d
                        .get("format")
                        .map(coerce_string_list)
                        .unwrap_or_default()
                        .join(" ")
                        .to_lowercase();
                    if !formats.contains("pdf") {
                        continue;
                    }
                    let pdf_url = format!("https://archive.org/download/{0}/{0}.pdf", ident);
                    let authors = d.get("creator").map(coerce_string_list).unwrap_or_default();
                    let desc = d.get("description").and_then(coerce_string);
                    let title = d
                        .get("title")
                        .and_then(coerce_string)
                        .unwrap_or_else(|| ident.clone());
                    let year = d.get("year").and_then(coerce_string);
                    docs.push(Document {
                        title,
                        url: pdf_url,
                        source: "internet_archive".to_string(),
                        authors,
                        year,
                        abstract_: desc,
                        identifier: Some(ident),
                    });
                }
                let yielded_now = yielded + docs.len();
                Some((Ok(docs), (page + 1, yielded_now, next_done)))
            }
        })
        .flat_map(|res: anyhow::Result<Vec<Document>>| match res {
            Ok(docs) => stream::iter(docs.into_iter().map(Ok).collect::<Vec<_>>()).boxed(),
            Err(e) => stream::iter(vec![Err(e)]).boxed(),
        })
        .take(limit)
        .boxed()
    }
}
