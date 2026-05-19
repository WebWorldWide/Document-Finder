//! arXiv — STEM preprints. Atom XML API. ≥3s pagination delay required by ToS.
//!
//! Uses quick-xml's event reader directly. The serde adapter can't handle the
//! repeated <link> elements, namespace-prefixed elements (arxiv:comment,
//! opensearch:totalResults, etc.), or <title type="html"> attributes in the feed.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::sync::Arc;
use std::time::Duration;

use super::{Document, Source};

const BASE: &str = "https://export.arxiv.org/api/query";
const PAGINATION_DELAY: Duration = Duration::from_secs(3);

pub struct ArxivSource {
    client: Arc<reqwest::Client>,
}

impl ArxivSource {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self { client }
    }
}

#[derive(Default)]
struct EntryBuilder {
    id: Option<String>,
    title: String,
    summary: String,
    published: Option<String>,
    authors: Vec<String>,
    links: Vec<(Option<String>, Option<String>, Option<String>)>,
}

fn strip_ns(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .rposition(|&b| b == b':')
        .map(|i| &bytes[i + 1..])
        .unwrap_or(bytes)
}

fn attr_val(e: &quick_xml::events::BytesStart, name: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if strip_ns(a.key.as_ref()) == name {
            a.unescape_value().ok().map(|v| v.into_owned())
        } else {
            None
        }
    })
}

fn collect_links(
    e: &quick_xml::events::BytesStart,
) -> (Option<String>, Option<String>, Option<String>) {
    (
        attr_val(e, b"href"),
        attr_val(e, b"title"),
        attr_val(e, b"type"),
    )
}

/// Parse an arXiv Atom feed into entry builders using the event-based reader.
/// This handles: repeated <link> elements, namespace-prefixed elements,
/// attributed <title type="html">, and nested markup in <summary>.
fn parse_feed(xml: &str) -> Vec<EntryBuilder> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut entries: Vec<EntryBuilder> = Vec::new();
    let mut current: Option<EntryBuilder> = None;
    let mut active_field: Option<&'static str> = None;
    let mut field_depth: usize = 0;
    let mut current_depth: usize = 0;
    let mut text_buf = String::new();
    let mut buf = Vec::new();

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                current_depth += 1;
                let qname = e.name();
                let local = strip_ns(qname.as_ref());

                if active_field.is_none() {
                    if current.is_none() {
                        if local == b"entry" {
                            current = Some(EntryBuilder::default());
                        }
                    } else {
                        match local {
                            b"link" => {
                                if let Some(entry) = &mut current {
                                    entry.links.push(collect_links(e));
                                }
                            }
                            b"id" => {
                                text_buf.clear();
                                active_field = Some("id");
                                field_depth = current_depth;
                            }
                            b"title" => {
                                text_buf.clear();
                                active_field = Some("title");
                                field_depth = current_depth;
                            }
                            b"summary" => {
                                text_buf.clear();
                                active_field = Some("summary");
                                field_depth = current_depth;
                            }
                            b"published" => {
                                text_buf.clear();
                                active_field = Some("published");
                                field_depth = current_depth;
                            }
                            b"name" => {
                                text_buf.clear();
                                active_field = Some("name");
                                field_depth = current_depth;
                            }
                            _ => {}
                        }
                    }
                }
            }

            Ok(Event::Empty(ref e)) => {
                let qname = e.name();
                if strip_ns(qname.as_ref()) == b"link" {
                    if let Some(entry) = &mut current {
                        entry.links.push(collect_links(e));
                    }
                }
            }

            Ok(Event::Text(ref e)) => {
                if active_field.is_some() {
                    if let Ok(t) = e.unescape() {
                        text_buf.push_str(&t);
                    }
                }
            }

            Ok(Event::CData(ref e)) => {
                if active_field.is_some() {
                    if let Ok(t) = std::str::from_utf8(e.as_ref()) {
                        text_buf.push_str(t);
                    }
                }
            }

            Ok(Event::End(ref e)) => {
                let qname = e.name();
                let local = strip_ns(qname.as_ref());

                if let Some(field) = active_field {
                    if current_depth == field_depth {
                        let value = text_buf.trim().to_string();
                        if let Some(entry) = &mut current {
                            match field {
                                "id" => entry.id = Some(value),
                                "title" => entry.title = value,
                                "summary" => entry.summary = value,
                                "published" => entry.published = Some(value),
                                "name" if !value.is_empty() => entry.authors.push(value),
                                _ => {}
                            }
                        }
                        active_field = None;
                        text_buf.clear();
                    }
                } else if local == b"entry" {
                    if let Some(builder) = current.take() {
                        entries.push(builder);
                    }
                }

                current_depth = current_depth.saturating_sub(1);
            }

            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::warn!("arXiv XML read error: {}", e);
                break;
            }
            _ => {}
        }
    }
    entries
}

fn builder_to_doc(entry: EntryBuilder) -> Option<Document> {
    let title = entry.title.trim().to_string();
    let summary = entry.summary.trim().to_string();
    let year = entry
        .published
        .as_deref()
        .and_then(|s| s.get(..4))
        .map(|s| s.to_string());

    let mut pdf_url: Option<String> = None;
    for (href, title_attr, type_attr) in &entry.links {
        let is_pdf =
            title_attr.as_deref() == Some("pdf") || type_attr.as_deref() == Some("application/pdf");
        if is_pdf {
            pdf_url = href.clone();
            break;
        }
    }
    if pdf_url.is_none() {
        if let Some(id) = entry.id.as_deref() {
            if let Some(aid) = id.rsplit('/').next() {
                pdf_url = Some(format!("https://arxiv.org/pdf/{}.pdf", aid));
            }
        }
    }

    let url = pdf_url?;
    Some(Document {
        title,
        url,
        source: "arxiv".to_string(),
        authors: entry.authors,
        year: year.filter(|s| !s.is_empty()),
        abstract_: if summary.is_empty() {
            None
        } else {
            Some(summary)
        },
        identifier: entry.id,
    })
}

#[async_trait]
impl Source for ArxivSource {
    fn name(&self) -> &'static str {
        "arxiv"
    }

    async fn search(
        &self,
        keywords: Vec<String>,
        limit: usize,
    ) -> BoxStream<'static, anyhow::Result<Document>> {
        let client = self.client.clone();
        stream::unfold((0usize, 0usize, false), move |(start, yielded, done)| {
            let client = client.clone();
            let keywords = keywords.clone();
            async move {
                if done || yielded >= limit {
                    return None;
                }
                let per_page = 100.min(limit.saturating_sub(yielded).max(1));
                let q = keywords
                    .iter()
                    .map(|k| format!("all:{}", k))
                    .collect::<Vec<_>>()
                    .join("+AND+");
                let body = match client
                    .get(BASE)
                    .query(&[
                        ("search_query", q.as_str()),
                        ("start", &start.to_string()),
                        ("max_results", &per_page.to_string()),
                    ])
                    .send()
                    .await
                {
                    Ok(r) => match r.error_for_status() {
                        Ok(r) => match r.text().await {
                            Ok(t) => t,
                            Err(e) => return Some((Err(e.into()), (start, yielded, true))),
                        },
                        Err(e) => return Some((Err(e.into()), (start, yielded, true))),
                    },
                    Err(e) => return Some((Err(e.into()), (start, yielded, true))),
                };

                let builders = parse_feed(&body);
                let n = builders.len();
                if n == 0 {
                    return None;
                }
                let docs: Vec<Document> = builders.into_iter().filter_map(builder_to_doc).collect();
                let next_done = n < per_page;
                if !next_done {
                    tokio::time::sleep(PAGINATION_DELAY).await;
                }
                Some((Ok(docs), (start + per_page, yielded + n, next_done)))
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:arxiv="http://arxiv.org/schemas/atom">
  <entry>
    <id>http://arxiv.org/abs/2401.00001v1</id>
    <updated>2024-01-01T00:00:00Z</updated>
    <published>2024-01-01T00:00:00Z</published>
    <title>Test Paper One</title>
    <summary>The first abstract.</summary>
    <author><name>Alice Author</name></author>
    <author><name>Bob Builder</name></author>
    <link href="http://arxiv.org/abs/2401.00001v1" rel="alternate" type="text/html"/>
    <link title="pdf" href="http://arxiv.org/pdf/2401.00001v1" rel="related" type="application/pdf"/>
  </entry>
  <entry>
    <id>http://arxiv.org/abs/2401.00002v2</id>
    <published>2023-12-15T00:00:00Z</published>
    <title>Second &amp; Quoted "Title"</title>
    <summary>Second abstract.</summary>
    <author><name>Carol Coder</name></author>
    <link href="http://arxiv.org/abs/2401.00002v2" rel="alternate" type="text/html"/>
  </entry>
</feed>"#;

    #[test]
    fn parses_two_entries_from_atom_feed() {
        let entries = parse_feed(SAMPLE_FEED);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Test Paper One");
        assert_eq!(entries[0].authors, vec!["Alice Author", "Bob Builder"]);
        assert_eq!(entries[1].title, "Second & Quoted \"Title\"");
        assert_eq!(entries[1].authors, vec!["Carol Coder"]);
    }

    #[test]
    fn builder_to_doc_prefers_pdf_link() {
        let entries = parse_feed(SAMPLE_FEED);
        let doc = builder_to_doc(entries.into_iter().next().unwrap()).unwrap();
        assert_eq!(doc.url, "http://arxiv.org/pdf/2401.00001v1");
        assert_eq!(doc.source, "arxiv");
        assert_eq!(doc.year.as_deref(), Some("2024"));
    }

    #[test]
    fn builder_to_doc_synthesizes_pdf_url_when_missing() {
        let entries = parse_feed(SAMPLE_FEED);
        // Second entry has no pdf link — should be synthesized from the id.
        let doc = builder_to_doc(entries.into_iter().nth(1).unwrap()).unwrap();
        assert_eq!(doc.url, "https://arxiv.org/pdf/2401.00002v2.pdf");
        assert_eq!(doc.year.as_deref(), Some("2023"));
    }

    #[test]
    fn parses_empty_feed_to_empty_vec() {
        let entries =
            parse_feed("<?xml version=\"1.0\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"/>");
        assert!(entries.is_empty());
    }
}
