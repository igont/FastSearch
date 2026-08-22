use std::{fs, path::PathBuf};

use fastsearch::{
    application::{ProductionRuntime, WorkspaceStore, embedding_model_cache_status},
    domain::EmbeddingModelId,
};
use serde_json::json;

const MODELS: [EmbeddingModelId; 3] = [
    EmbeddingModelId::SnowflakeArcticEmbedLV2,
    EmbeddingModelId::MultilingualE5Large,
    EmbeddingModelId::NomicEmbedTextV2Moe,
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = PathBuf::from(std::env::args_os().nth(1).ok_or("fixture root required")?);
    let workspace = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .ok_or("workspace root required")?,
    );
    let evidence = PathBuf::from(std::env::args_os().nth(3).ok_or("evidence path required")?);
    if workspace.exists() {
        return Err("acceptance workspace must not already exist".into());
    }
    fs::create_dir_all(workspace.join("corpus"))?;
    fs::create_dir_all(workspace.join(".fastsearch").join("local"))?;
    copy_files(&fixture.join("corpus"), &workspace.join("corpus"))?;
    fs::copy(
        fixture.join("workspace/.fastsearch/workspace.toml"),
        workspace.join(".fastsearch/workspace.toml"),
    )?;

    let product_home = if let Some(value) = std::env::var_os("FASTSEARCH_HOME") {
        let value = PathBuf::from(value);
        // SAFETY: this single-threaded preparation process has not started model workers yet.
        unsafe { std::env::set_var("HF_HOME", value.join("models").join("huggingface")) };
        value
    } else {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or("LOCALAPPDATA is unavailable")?
            .join("FastSearch")
    };
    let product_models = product_home.join("models");
    fs::create_dir_all(&product_models)?;
    let catalog = product_models.join("model-manifest.json");
    let source_catalog = fixture.join("model-manifest.json");
    if catalog.exists() && fs::read(&catalog)? != fs::read(&source_catalog)? {
        return Err("existing product model catalog differs from the pinned fixture".into());
    }
    if !catalog.exists() {
        fs::copy(&source_catalog, &catalog)?;
    }

    let store = WorkspaceStore::open(&workspace)?;
    let mut runtime = ProductionRuntime::open(store.production_config())?;
    let state = runtime.index()?;
    let mut partitions = Vec::new();
    for model in MODELS {
        let cache = embedding_model_cache_status(model)?;
        if !cache.ready() {
            return Err(format!("model cache {} is not ready", model.slug()).into());
        }
        let status = runtime.build_model_partition(model, cache.root())?;
        let metrics = runtime
            .model_partition_metrics(model)?
            .ok_or("partition metrics missing")?;
        partitions.push(json!({
            "model": model.slug(),
            "freshness": format!("{:?}", status.freshness()),
            "state_generation": status.state_generation(),
            "projection_generation": status.projection_generation(),
            "size_bytes": metrics.size_bytes(),
            "build_duration_ms": metrics.build_duration_ms()
        }));
    }
    fs::write(
        evidence,
        serde_json::to_vec_pretty(&json!({
            "schema": 1,
            "gate": "G-RESOURCE@A5",
            "preparation": "explicit-outside-mcp",
            "workspace": workspace,
            "state_generation": state.state_generation(),
            "partitions": partitions
        }))?,
    )?;
    Ok(())
}

fn copy_files(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            fs::create_dir_all(&destination)?;
            copy_files(&entry.path(), &destination)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}
