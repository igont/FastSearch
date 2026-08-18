//! Verifies the complete local provisioning path for one catalog model.
//!
//! Usage: `cargo run --release --example model_readiness -- <model-slug>`.

use fastsearch::{
    application::{ensure_embedding_model_with_progress, model_runtime_capabilities},
    domain::EmbeddingModelId,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let slug = std::env::args()
        .nth(1)
        .ok_or("usage: model_readiness <model-slug>")?;
    let model = EmbeddingModelId::parse(&slug).ok_or("unknown FastSearch model slug")?;
    let availability = ensure_embedding_model_with_progress(model, false, |progress| {
        eprintln!(
            "asset={} completed={} total={}",
            progress.asset(),
            progress.completed_bytes(),
            progress.total_bytes()
        );
    })?;
    let capabilities = model_runtime_capabilities(model)?;
    println!(
        "model={} downloaded={} cpu={:?} gpu={:?} gpu_detail={}",
        model.slug(),
        availability.downloaded(),
        capabilities.cpu(),
        capabilities.gpu(),
        capabilities.gpu_detail().unwrap_or("-")
    );
    Ok(())
}
