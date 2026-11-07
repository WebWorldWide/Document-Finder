//! HTML → Markdown with main-content selection.
//!
//! Picks the page's main content (`<article>`, `<main>`, `[role=main]`, or
//! the densest text container), drops chrome (nav, header, footer, aside,
//! forms, scripts), and renders headings, paragraphs, lists, quotes, code
//! and simple tables as Markdown. Used for saved HTML documents and EPUB
//! chapters.

use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node, Selector};

const SKIP_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "nav", "header", "footer", "aside", "form",
    "button", "iframe", "select", "input", "textarea", "canvas", "video", "audio",
];

/// Class/id fragments that mark boilerplate containers.
const BOILERPLATE_HINTS: &[&str] = &[
    "cookie",
    "banner",
    "sidebar",
    "share",
    "social",
    "newsletter",
    "subscribe",
    "advert",
    "promo",
    "breadcrumb",
    "related",
    "comment",
    "footer",
    "header",
    "menu",
    "navbar",
    "popup",
    "modal",
];

fn is_boilerplate(el: &ElementRef) -> bool {
    let v = el.value();
    if SKIP_TAGS.contains(&v.name()) {
        return true;
    }
    if v.attr("aria-hidden") == Some("true") || v.attr("hidden").is_some() {
        return true;
    }
    if matches!(
        v.attr("role"),
        Some("navigation" | "banner" | "contentinfo" | "complementary")
    ) {
        return true;
    }
    let hint = format!(
        "{} {}",
        v.attr("class").unwrap_or(""),
        v.attr("id").unwrap_or("")
    )
    .to_lowercase();
    // Only treat as boilerplate when the container is not obviously content.
    !hint.contains("content")
        && !hint.contains("article")
        && BOILERPLATE_HINTS.iter().any(|h| hint.contains(h))
}

fn text_len(el: ElementRef) -> usize {
    el.text().map(|t| t.trim().len()).sum()
}

/// Choose the element that holds the main content.
fn main_root(doc: &Html) -> Option<ElementRef<'_>> {
    for sel in [
        "article",
        "main",
        "[role=main]",
        "#content",
        ".content",
        "#main",
    ] {
        if let Ok(s) = Selector::parse(sel) {
            if let Some(best) = doc.select(&s).max_by_key(|e| text_len(*e)) {
                if text_len(best) > 400 {
                    return Some(best);
                }
            }
        }
    }
    // Fallback: the div/section whose *direct* paragraph children hold the
    // most text. Direct children only, so deeply nested markup stays linear
    // (counting every descendant paragraph per container is quadratic).
    let c_sel = Selector::parse("div, section, td").ok()?;
    let best = doc
        .select(&c_sel)
        .take(20_000)
        .map(|e| {
            let p: usize = e
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|c| c.value().name() == "p")
                .map(text_len)
                .sum();
            (e, p)
        })
        .max_by_key(|(_, p)| *p);
    if let Some((e, p)) = best {
        if p > 400 {
            return Some(e);
        }
    }
    Selector::parse("body")
        .ok()
        .and_then(|s| doc.select(&s).next())
}

struct Writer {
    out: String,
    line: String,
    list_depth: usize,
    depth: usize,
}

impl Writer {
    fn push_text(&mut self, t: &str) {
        let collapsed: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            if !t.is_empty() && !self.line.is_empty() && !self.line.ends_with(' ') {
                self.line.push(' ');
            }
            return;
        }
        if t.starts_with(char::is_whitespace) && !self.line.is_empty() && !self.line.ends_with(' ')
        {
            self.line.push(' ');
        }
        self.line.push_str(&collapsed);
        if t.ends_with(char::is_whitespace) {
            self.line.push(' ');
        }
    }
    fn block(&mut self) {
        let l = self.line.trim();
        if !l.is_empty() {
            self.out.push_str(l);
            self.out.push_str("\n\n");
        }
        self.line.clear();
    }
}

/// Deepest DOM nesting we descend into. Untrusted HTML can nest thousands of
/// levels, and a stack overflow can't be caught by `catch_unwind`.
const MAX_DEPTH: usize = 200;

fn walk(node: NodeRef<'_, Node>, w: &mut Writer) {
    w.depth += 1;
    if w.depth <= MAX_DEPTH {
        walk_inner(node, w);
    }
    w.depth -= 1;
}

fn walk_inner(node: NodeRef<'_, Node>, w: &mut Writer) {
    match node.value() {
        Node::Text(t) => w.push_text(t),
        Node::Element(_) => {
            let Some(el) = ElementRef::wrap(node) else {
                return;
            };
            if is_boilerplate(&el) {
                return;
            }
            let name = el.value().name();
            match name {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    w.block();
                    let level = name[1..].parse::<usize>().unwrap_or(2).clamp(1, 6);
                    let text: String = el.text().collect::<Vec<_>>().join(" ");
                    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !text.is_empty() {
                        w.out
                            .push_str(&format!("{} {}\n\n", "#".repeat(level.max(2)), text));
                    }
                }
                "p" | "div" | "section" | "figure" | "figcaption" | "dd" | "dt" | "address" => {
                    w.block();
                    for c in node.children() {
                        walk(c, w);
                    }
                    w.block();
                }
                "br" => w.line.push(' '),
                "li" => {
                    w.block();
                    w.line
                        .push_str(&"  ".repeat(w.list_depth.saturating_sub(1)));
                    w.line.push_str("- ");
                    for c in node.children() {
                        walk(c, w);
                    }
                    w.block();
                }
                "ul" | "ol" => {
                    w.block();
                    w.list_depth += 1;
                    for c in node.children() {
                        walk(c, w);
                    }
                    w.list_depth -= 1;
                    w.block();
                }
                "blockquote" => {
                    w.block();
                    let mut inner = Writer {
                        out: String::new(),
                        line: String::new(),
                        list_depth: 0,
                        depth: 0,
                    };
                    for c in node.children() {
                        walk(c, &mut inner);
                    }
                    inner.block();
                    for para in inner.out.split("\n\n").filter(|p| !p.trim().is_empty()) {
                        w.out.push_str(&format!("> {}\n\n", para.trim()));
                    }
                }
                "pre" => {
                    w.block();
                    let code: String = el.text().collect();
                    if !code.trim().is_empty() {
                        w.out
                            .push_str(&format!("```\n{}\n```\n\n", code.trim_end()));
                    }
                }
                "tr" => {
                    w.block();
                    let cells: Vec<String> = el
                        .children()
                        .filter_map(ElementRef::wrap)
                        .filter(|c| matches!(c.value().name(), "td" | "th"))
                        .map(|c| c.text().collect::<Vec<_>>().join(" "))
                        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                        .collect();
                    if cells.iter().any(|c| !c.is_empty()) {
                        w.out.push_str(&format!("| {} |\n", cells.join(" | ")));
                    }
                }
                "table" => {
                    w.block();
                    for c in node.children() {
                        walk(c, w);
                    }
                    w.out.push('\n');
                }
                "img" => {
                    if let Some(alt) = el.value().attr("alt").filter(|a| !a.trim().is_empty()) {
                        w.push_text(&format!(" [image: {}] ", alt.trim()));
                    }
                }
                _ => {
                    for c in node.children() {
                        walk(c, w);
                    }
                }
            }
        }
        _ => {
            for c in node.children() {
                walk(c, w);
            }
        }
    }
}

/// Convert an HTML document to Markdown, keeping only the main content.
pub fn html_to_markdown(html: &str) -> String {
    let doc = Html::parse_document(html);
    let mut w = Writer {
        out: String::new(),
        line: String::new(),
        list_depth: 0,
        depth: 0,
    };
    if let Some(root) = main_root(&doc) {
        for c in root.children() {
            walk(c, &mut w);
        }
    }
    w.block();
    let mut md = w.out;
    while md.contains("\n\n\n") {
        md = md.replace("\n\n\n", "\n\n");
    }
    md.trim().to_string()
}

/// `<title>` of an HTML document, if present.
pub fn html_title(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("title").ok()?;
    let t: String = doc.select(&sel).next()?.text().collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_article_drops_chrome() {
        let body = "Real article text. ".repeat(40);
        let html = format!(
            "<html><body><nav>Home About</nav><div class='cookie-banner'>We use cookies</div>
             <article><h1>Big Title</h1><p>{body}</p><ul><li>One</li><li>Two</li></ul>
             <blockquote><p>Quoted</p></blockquote></article>
             <footer>Copyright</footer><script>var x=1;</script></body></html>"
        );
        let md = html_to_markdown(&html);
        assert!(md.starts_with("## Big Title"), "{md}");
        assert!(md.contains("Real article text."));
        assert!(md.contains("- One\n\n- Two"));
        assert!(md.contains("> Quoted"));
        assert!(!md.contains("Home About"));
        assert!(!md.contains("cookies"));
        assert!(!md.contains("Copyright"));
        assert!(!md.contains("var x"));
    }

    #[test]
    fn falls_back_to_densest_container_and_renders_tables() {
        let body = "Paragraph text here. ".repeat(30);
        let html = format!(
            "<body><div id='x'><p>{body}</p><table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table></div></body>"
        );
        let md = html_to_markdown(&html);
        assert!(md.contains("| A | B |"));
        assert!(md.contains("| 1 | 2 |"));
    }

    #[test]
    fn deeply_nested_html_does_not_overflow() {
        let html = format!(
            "{}deep text{}",
            "<div>".repeat(20_000),
            "</div>".repeat(20_000)
        );
        let _ = html_to_markdown(&html);
    }

    #[test]
    fn title_extraction() {
        assert_eq!(
            html_title("<html><head><title> Hello\n World </title></head></html>").as_deref(),
            Some("Hello World")
        );
    }
}
