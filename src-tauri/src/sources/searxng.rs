use std::sync::Arc;
use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use serde::Deserialize;

use super::{Document, Source};

pub struct SearXNGSource {
    client: Arc<reqwest::Client>,
    instance_url: String,
}

impl SearXNGSource {
    pub fn new(client: Arc<reqwest::Client>, instance_url: String) -> Self {
        Self { client, instance_url }
    }
}

#[derive(Deserialize)]
struct SearxResult {
    url: String,
    title: String,
    content: Option<String>,
}

#[derive(Deserialize)]
struct SearxResponse {
    results: Vec<SearxResult>,
}

fn looks_like_doc(url: &str) -> bool {
    let u = url.to_lowercase();
    u.ends_with(".pdf")
        || u.ends_with(".epub")
        || u.contains("/pdf/")
        || u.contains("filetype=pdf")
        || u.contains("/download/")
}

#[async_trait]
impl Source for SearXNGSource {
    fn name(&self) -> &'static str {
        "searxng"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let q = format!("{} filetype:pdf OR filetype:epub", keywords.join(" "));
        let base_url = format!("{}/search", self.instance_url);
        let client = self.client.clone();
        let pages = ((limit / 10).max(1) + 1).min(5);

        let mut docs: Vec<anyhow::Result<Document>> = Vec::new();

        for page in 1..=pages {
            if docs.len() >= limit {
                break;
            }
            let resp = client
                .get(&base_url)
                .query(&[
                    ("q", q.as_str()),
                    ("format", "json"),
                    ("categories", "files,general"),
                    ("pageno", &page.to_string()),
                ])
                .send()
                .await;

            match resp {
                Ok(r) => match r.json::<SearxResponse>().await {
                    Ok(body) => {
                        for result in body.results {
                            if looks_like_doc(&result.url) {
                                docs.push(Ok(Document {
                                    title: result.title,
                                    url: result.url,
                                    source: "searxng".to_string(),
                                    authors: vec![],
                                    year: None,
                                    abstract_: result.content,
                                    identifier: None,
                                }));
                            }
                            if docs.len() >= limit {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        docs.push(Err(e.into()));
                        break;
                    }
                },
                Err(e) => {
                    docs.push(Err(e.into()));
                    break;
                }
            }
        }

        Box::pin(stream::iter(docs))
    }
}
