//! Dev tool: extract a document and print the Markdown + stats.
//!   cargo run --example extract -- path/to/file.pdf
use document_finder_lib::engine::extract;

fn main() {
    let path = std::env::args().nth(1).expect("usage: extract <file>");
    let t = std::time::Instant::now();
    match extract::extract(std::path::Path::new(&path), "Document") {
        Ok(ex) => {
            println!("{}", ex.markdown);
            eprintln!(
                "engine={} pages={:?} words={} tokens={} chunks={} lang={:?} in {:?}",
                ex.engine,
                ex.pages,
                ex.words,
                ex.tokens,
                ex.chunks.len(),
                ex.lang,
                t.elapsed()
            );
        }
        Err(e) => eprintln!("error: {e}"),
    }
}
