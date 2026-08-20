use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::{
    adapters::source::SourceDecision,
    domain::{EmbeddingModelId, ErrorKind, FastSearchError},
};

use super::chunking::{CHUNKER_VERSION, ChunkEnvelope};

const PUBLISHED_MANIFEST: &str = "index/chunks/current.json";
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct InspectionManifest {
    schema_version: u32,
    mode: String,
    chunker_version: String,
    state_generation: Option<u64>,
    embedding_model: String,
    decisions: Vec<SourceDecision>,
    chunks: Vec<ChunkEnvelope>,
}

impl InspectionManifest {
    pub(crate) fn published(
        generation: u64,
        model: EmbeddingModelId,
        mut decisions: Vec<SourceDecision>,
        mut chunks: Vec<ChunkEnvelope>,
    ) -> Self {
        sort_manifest_entries(&mut decisions, &mut chunks);
        attach_file_hashes(&decisions, &mut chunks);
        Self {
            schema_version: 1,
            mode: "current".to_owned(),
            chunker_version: CHUNKER_VERSION.to_owned(),
            state_generation: Some(generation),
            embedding_model: model.slug().to_owned(),
            decisions,
            chunks,
        }
    }
}

fn sort_manifest_entries(decisions: &mut [SourceDecision], chunks: &mut [ChunkEnvelope]) {
    decisions.sort_by(|left, right| {
        left.root_id
            .cmp(&right.root_id)
            .then_with(|| left.path.cmp(&right.path))
    });
    chunks.sort_by(|left, right| {
        left.source_root_id
            .cmp(&right.source_root_id)
            .then_with(|| left.source_path.cmp(&right.source_path))
            .then_with(|| left.source_line_start.cmp(&right.source_line_start))
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
}

fn attach_file_hashes(decisions: &[SourceDecision], chunks: &mut [ChunkEnvelope]) {
    let hashes = decisions
        .iter()
        .filter(|decision| decision.included)
        .map(|decision| {
            (
                (decision.root_id.as_deref(), decision.path.as_str()),
                decision.source_hash.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for chunk in chunks {
        if let Some(hash) =
            hashes.get(&(chunk.source_root_id.as_deref(), chunk.source_path.as_str()))
        {
            chunk.source_hash = (*hash).to_owned();
        }
    }
}

#[derive(Clone, Debug)]
pub struct InspectionReport {
    path: PathBuf,
    included_files: usize,
    excluded_files: usize,
    chunks: usize,
}

impl InspectionReport {
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn display_path(&self) -> String {
        display_path(&self.path)
    }

    #[must_use]
    pub fn display_inputs_path(&self) -> String {
        display_path(&self.path.join("indexing-inputs.md"))
    }

    pub const fn included_files(&self) -> usize {
        self.included_files
    }

    pub const fn excluded_files(&self) -> usize {
        self.excluded_files
    }

    pub const fn chunks(&self) -> usize {
        self.chunks
    }
}

fn display_path(path: &Path) -> String {
    let value = path.display().to_string();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
    }
}

pub(crate) fn save_published(
    service_root: &Path,
    manifest: &InspectionManifest,
) -> Result<(), FastSearchError> {
    let path = service_root.join(PUBLISHED_MANIFEST);
    let bytes = serde_json::to_vec_pretty(manifest).map_err(inspection_failure)?;
    atomic_write(&path, &bytes)
}

pub(crate) fn load_published(service_root: &Path) -> Result<InspectionManifest, FastSearchError> {
    let path = service_root.join(PUBLISHED_MANIFEST);
    if !path.is_file() {
        return Err(FastSearchError::new(
            ErrorKind::StateFailure,
            "published chunk manifest is absent; run /index update first",
        ));
    }
    let bytes = fs::read(path).map_err(inspection_failure)?;
    serde_json::from_slice(&bytes).map_err(inspection_failure)
}

pub(crate) fn export(
    service_root: &Path,
    manifest: InspectionManifest,
    requested: Option<&Path>,
) -> Result<InspectionReport, FastSearchError> {
    let target = requested.map(Path::to_path_buf).unwrap_or_else(|| {
        service_root
            .join("inspections")
            .join(format!("chunks_{}", Local::now().format("%d-%m-%Y_%H-%M")))
    });
    if target.exists() {
        return Err(FastSearchError::new(
            ErrorKind::StateFailure,
            format!("inspection directory already exists: {}", target.display()),
        ));
    }
    let parent = target.parent().ok_or_else(|| {
        FastSearchError::new(ErrorKind::StateFailure, "inspection path has no parent")
    })?;
    fs::create_dir_all(parent).map_err(inspection_failure)?;
    let temporary = parent.join(format!(
        ".inspection-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    if temporary.exists() {
        fs::remove_dir_all(&temporary).map_err(inspection_failure)?;
    }
    fs::create_dir(&temporary).map_err(inspection_failure)?;
    let result = write_export(&temporary, &manifest);
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, &target) {
        let _ = fs::remove_dir_all(&temporary);
        return Err(inspection_failure(error));
    }
    let included_files = manifest
        .decisions
        .iter()
        .filter(|decision| decision.included)
        .count();
    Ok(InspectionReport {
        path: target,
        included_files,
        excluded_files: manifest.decisions.len() - included_files,
        chunks: manifest.chunks.len(),
    })
}

pub(crate) fn inspect_published(
    service_root: &Path,
    output: Option<&Path>,
) -> Result<InspectionReport, FastSearchError> {
    export(service_root, load_published(service_root)?, output)
}

fn write_export(root: &Path, manifest: &InspectionManifest) -> Result<(), FastSearchError> {
    write_indexing_inputs(root.join("indexing-inputs.md"), &manifest.chunks)?;
    let technical = root.join("technical");
    fs::create_dir(&technical).map_err(inspection_failure)?;
    write_json(technical.join("manifest.json"), manifest)?;
    write_json_lines(technical.join("admission.jsonl"), &manifest.decisions)?;
    write_json_lines(technical.join("chunks.jsonl"), &manifest.chunks)?;
    write_summary(technical.join("summary.md"), manifest)
}

fn write_indexing_inputs(path: PathBuf, chunks: &[ChunkEnvelope]) -> Result<(), FastSearchError> {
    let file = File::create(path).map_err(inspection_failure)?;
    let mut writer = BufWriter::new(file);
    write_inputs(
        &mut writer,
        chunks.iter().map(|chunk| chunk.lexical_input.as_str()),
    )?;
    writer.flush().map_err(inspection_failure)
}

fn write_inputs<'a>(
    writer: &mut impl Write,
    inputs: impl IntoIterator<Item = &'a str>,
) -> Result<(), FastSearchError> {
    let mut count = 0_usize;
    for (index, input) in inputs.into_iter().enumerate() {
        if index > 0 {
            writer.write_all(b"\n\n").map_err(inspection_failure)?;
        }
        writer
            .write_all(input.as_bytes())
            .map_err(inspection_failure)?;
        count += 1;
    }
    if count > 0 {
        writer.write_all(b"\n").map_err(inspection_failure)?;
    }
    Ok(())
}

fn write_summary(path: PathBuf, manifest: &InspectionManifest) -> Result<(), FastSearchError> {
    let included = manifest
        .decisions
        .iter()
        .filter(|item| item.included)
        .count();
    let excluded = manifest.decisions.len() - included;
    let mut kinds = BTreeMap::<String, usize>::new();
    for chunk in &manifest.chunks {
        *kinds.entry(chunk.kind.as_str().to_owned()).or_default() += 1;
    }
    let mut text = format!(
        "# Выгрузка чанкинга\n\nРежим: `{}`. Версия: `{}`. Модель: `{}`.\n\nДопущено файлов: {included}. Исключено файлов: {excluded}. Чанков: {}.\n",
        manifest.mode,
        manifest.chunker_version,
        manifest.embedding_model,
        manifest.chunks.len()
    );
    if let Some(generation) = manifest.state_generation {
        text.push_str(&format!(
            "\nПоколение опубликованного состояния: {generation}.\n"
        ));
    }
    text.push_str("\n## Состав чанков\n\n");
    for (kind, count) in kinds {
        text.push_str(&format!("- `{kind}`: {count}\n"));
    }
    fs::write(path, text.as_bytes()).map_err(inspection_failure)
}

fn write_json(path: PathBuf, value: &impl Serialize) -> Result<(), FastSearchError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(inspection_failure)?;
    fs::write(path, bytes).map_err(inspection_failure)
}

fn write_json_lines<T: Serialize>(path: PathBuf, values: &[T]) -> Result<(), FastSearchError> {
    let file = File::create(path).map_err(inspection_failure)?;
    let mut writer = BufWriter::new(file);
    for value in values {
        serde_json::to_writer(&mut writer, value).map_err(inspection_failure)?;
        writer.write_all(b"\n").map_err(inspection_failure)?;
    }
    writer.flush().map_err(inspection_failure)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), FastSearchError> {
    let parent = path.parent().ok_or_else(|| {
        FastSearchError::new(ErrorKind::StateFailure, "manifest path has no parent")
    })?;
    fs::create_dir_all(parent).map_err(inspection_failure)?;
    let temporary = parent.join(format!(
        ".manifest-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = File::create(&temporary).map_err(inspection_failure)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(inspection_failure)?;
    if path.exists() {
        fs::remove_file(path).map_err(inspection_failure)?;
    }
    fs::rename(temporary, path).map_err(inspection_failure)
}

fn inspection_failure(error: impl std::fmt::Display) -> FastSearchError {
    FastSearchError::new(
        ErrorKind::StateFailure,
        format!("chunk inspection: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_refuses_to_overwrite_an_existing_directory() {
        let root =
            std::env::temp_dir().join(format!("fastsearch-inspection-{}", std::process::id()));
        let target = root.join("existing");
        fs::create_dir_all(&target).unwrap();
        let manifest = InspectionManifest::published(
            1,
            EmbeddingModelId::MultilingualE5Small,
            Vec::new(),
            Vec::new(),
        );

        let error = export(&root, manifest, Some(&target)).unwrap_err();
        assert!(error.message().contains("already exists"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn indexing_inputs_are_plain_blocks_separated_by_one_empty_line() {
        let mut output = Vec::new();

        write_inputs(&mut output, ["Первый чанк", "Второй чанк"]).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Первый чанк\n\nВторой чанк\n"
        );
    }
}
