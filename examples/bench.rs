//! Phase-by-phase indexing and warm-search benchmark.
//!
//! Usage: `cargo run --release --example bench -- <path>`

use std::path::Path;
use std::time::Instant;

use rayon::prelude::*;
use semeja::bm25::{enrich_for_bm25, Bm25Index};
use semeja::chunk::chunk_source;
use semeja::embed::{embed_chunks, load_model, CosineBackend};
use semeja::lang::{detect_language, get_extensions};
use semeja::tokenize::tokenize;
use semeja::walk::walk_files;
use semeja::SemejaIndex;

const QUERIES: &[&str] = &[
    "how are database queries built",
    "validate a form field",
    "render a template response",
    "authentication middleware",
    "serialize a model to json",
];

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: bench <path>");
    let path = Path::new(&path);
    let model = load_model(None).expect("load model");

    let t = Instant::now();
    let extensions = get_extensions(false, None);
    let files = walk_files(path, &extensions, None);
    let walk = ms(t);

    // Parallel read + chunk, mirroring create_index_from_path.
    let t = Instant::now();
    let chunks: Vec<_> = files
        .par_iter()
        .flat_map_iter(|f| {
            let bytes = std::fs::read(f).unwrap_or_default();
            let source = String::from_utf8_lossy(&bytes);
            let rel = f.strip_prefix(path).unwrap_or(f);
            chunk_source(&source, &rel.to_string_lossy(), detect_language(f).as_deref())
        })
        .collect();
    let chunk = ms(t);

    let t = Instant::now();
    let embeddings = embed_chunks(model.as_ref(), &chunks);
    let embed = ms(t);

    let t = Instant::now();
    let corpus: Vec<Vec<String>> =
        chunks.par_iter().map(|c| tokenize(&enrich_for_bm25(c))).collect();
    let bm25 = Bm25Index::build(&corpus);
    let bm25_build = ms(t);

    let t = Instant::now();
    let _ = CosineBackend::new(embeddings);
    let cosine = ms(t);
    let _ = bm25;

    println!("files={}  chunks={}", files.len(), chunks.len());
    println!("walk      {walk:8.1} ms");
    println!("chunk     {chunk:8.1} ms");
    println!("embed     {embed:8.1} ms");
    println!("bm25build {bm25_build:8.1} ms");
    println!("cosine    {cosine:8.1} ms");
    println!("TOTAL     {:8.1} ms", walk + chunk + embed + bm25_build + cosine);

    let index = SemejaIndex::from_path(path, None, None, false).expect("build index");
    let t = Instant::now();
    let runs = 20;
    for _ in 0..runs {
        for query in QUERIES {
            let _ = index.search(query, 5, "hybrid", None, &[], &[]).expect("search");
        }
    }
    println!("search    {:8.2} ms/query (warm)", ms(t) / (runs * QUERIES.len()) as f64);
}
