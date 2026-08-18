//! Exercises a real E5 Small partition twice, changing exactly one document.
//!
//! Usage:
//! `cargo run --release --example incremental_e5_smoke -- C:\path\to\multilingual-e5-small\<revision>`

use std::{
    fs,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use fastsearch::{
    application::{ProductionConfig, ProductionRuntime},
    domain::EmbeddingModelId,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model_root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: incremental_e5_smoke <e5-small-model-root>")?,
    );
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let workspace = std::env::temp_dir().join(format!("fastsearch-incremental-e5-{nonce}"));
    let docs = workspace.join("docs");
    let code = workspace.join("code");
    fs::create_dir_all(&docs)?;
    fs::create_dir_all(&code)?;
    for number in 0..384 {
        fs::write(
            docs.join(format!("note-{number}.md")),
            format!(
                "# Note {number}\n\nStable searchable content for incremental E5 smoke testing."
            ),
        )?;
    }
    let config = ProductionConfig::for_workspace(
        &workspace,
        vec![("docs".to_owned(), docs.clone())],
        vec![("code".to_owned(), code)],
        workspace.join(".fastsearch/local"),
    )
    .with_embedding_model(EmbeddingModelId::MultilingualE5Small, model_root);
    let (initial, update) = {
        let mut runtime = ProductionRuntime::open(config)?;
        let initial_started = Instant::now();
        runtime.index()?;
        let initial = initial_started.elapsed();

        fs::write(
            docs.join("note-17.md"),
            "# Note 17\n\nOnly this document changed; all other vectors must be reused.",
        )?;
        let update_started = Instant::now();
        runtime.index()?;
        (initial, update_started.elapsed())
    };
    println!(
        "e5-small initial_ms={} incremental_ms={} documents=384 changed_files=1",
        initial.as_millis(),
        update.as_millis()
    );
    fs::remove_dir_all(workspace)?;
    Ok(())
}
