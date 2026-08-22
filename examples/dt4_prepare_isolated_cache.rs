use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("model manifest required")?,
    );
    let target_home = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .ok_or("target product home required")?,
    );
    if target_home.exists() {
        return Err("target product home must not already exist".into());
    }
    let local_product =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA required")?)
            .join("FastSearch");
    let hf_hub = PathBuf::from(std::env::var_os("USERPROFILE").ok_or("USERPROFILE required")?)
        .join(".cache/huggingface/hub");
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    fs::create_dir_all(target_home.join("models"))?;
    fs::copy(
        &manifest_path,
        target_home.join("models/model-manifest.json"),
    )?;

    for model in manifest["models"].as_array().ok_or("models")? {
        let slug = model["slug"].as_str().ok_or("slug")?;
        let repository = model["repository"].as_str().ok_or("repository")?;
        let revision = model["revision"].as_str().ok_or("revision")?;
        let repository_dir = format!("models--{}", repository.replace('/', "--"));
        let source = if matches!(slug, "nomic-embed-text-v2-moe" | "qwen3-reranker-0.6b") {
            hf_hub
                .join(&repository_dir)
                .join("snapshots")
                .join(revision)
        } else {
            local_product
                .join("models")
                .join(slug)
                .join("runtime")
                .join(&repository_dir)
                .join("snapshots")
                .join(revision)
        };
        let target = if slug == "qwen3-reranker-0.6b" {
            target_home.join("models").join(slug).join(revision)
        } else if slug == "nomic-embed-text-v2-moe" {
            target_home
                .join("models/huggingface/hub")
                .join(&repository_dir)
                .join("snapshots")
                .join(revision)
        } else {
            target_home
                .join("models")
                .join(slug)
                .join("runtime")
                .join(&repository_dir)
                .join("snapshots")
                .join(revision)
        };
        copy_tree(&source, &target)?;
        if slug == "multilingual-e5-large" {
            let fastembed_repository = target_home.join("models/huggingface").join(&repository_dir);
            copy_tree(
                &source,
                &fastembed_repository.join("snapshots").join(revision),
            )?;
            fs::create_dir_all(fastembed_repository.join("refs"))?;
            fs::write(fastembed_repository.join("refs/main"), revision)?;
        }
        if slug != "qwen3-reranker-0.6b" {
            fs::create_dir_all(target_home.join("models").join(slug))?;
            fs::write(
                target_home.join("models").join(slug).join(".ready"),
                revision,
            )?;
        }
        if slug != "qwen3-reranker-0.6b" {
            let repository_root = if slug == "nomic-embed-text-v2-moe" {
                target_home
                    .join("models/huggingface/hub")
                    .join(&repository_dir)
            } else {
                target_home
                    .join("models")
                    .join(slug)
                    .join("runtime")
                    .join(&repository_dir)
            };
            let refs = repository_root.join("refs");
            fs::create_dir_all(&refs)?;
            fs::write(refs.join("main"), revision)?;
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if !source.is_dir() {
        return Err(format!("pinned source is unavailable: {}", source.display()).into());
    }
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            return Err(format!("symlink source rejected: {}", entry.path().display()).into());
        }
        let destination = target.join(entry.file_name());
        if metadata.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else if metadata.is_file() {
            fs::copy(entry.path(), destination)?;
        } else {
            return Err(format!("unsupported source: {}", entry.path().display()).into());
        }
    }
    Ok(())
}
