//! Versioned `.cfmap.md` admission and safe AUTO-region regeneration.

use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::domain::{
    CanonicalRecord, ContentHash, ErrorKind, FastSearchError, FileHash, RecordKind, SourceLocator,
    SourceSnapshot, StableId,
};
use crate::ports::SourcePort;

const HEADER_START: &str = "---\n";
const HEADER_END: &str = "---\n";
const AUTO_START: &str = "<!-- cfmap:auto:start -->";
const AUTO_END: &str = "<!-- cfmap:auto:end -->";
const MAX_MAP_BYTES: u64 = 1_048_576;
const MAX_DERIVED_BODY_BYTES: usize = 65_536;
const MAX_MAP_FILES: usize = 1_024;
const MAX_MAP_DEPTH: usize = 16;

/// Result of a regeneration request. CURATED files are deliberately untouched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegenerationOutcome {
    UpdatedAuto,
    PreservedCurated,
}

/// Read-only source and bounded writer for a single configured map root.
#[derive(Debug)]
pub struct CodeMapSource {
    root: PathBuf,
}

impl CodeMapSource {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Validates every map before returning a complete, deterministic snapshot set.
    pub fn snapshots(&self) -> Result<Vec<SourceSnapshot>, FastSearchError> {
        let root = canonical_root(&self.root)?;
        let mut paths = Vec::new();
        collect_map_paths(&root, &root, 0, &mut paths)?;
        paths.sort();
        paths
            .into_iter()
            .map(|path| parse_map_file(&root, &path))
            .collect()
    }

    /// Replaces only the delimited AUTO body after full validation; invalid input never writes.
    pub fn regenerate(
        &self,
        locator: &str,
        derived_body: &str,
    ) -> Result<RegenerationOutcome, FastSearchError> {
        if derived_body.contains(AUTO_START) || derived_body.contains(AUTO_END) {
            return Err(invalid(
                "derived map body must not contain cfmap region markers",
            ));
        }
        if derived_body.len() > MAX_DERIVED_BODY_BYTES {
            return Err(invalid("derived map body exceeds the configured limit"));
        }
        let root = canonical_root(&self.root)?;
        let path = contained_map_path(&root, locator)?;
        let original =
            fs::read_to_string(&path).map_err(|error| source_failure("read map", error))?;
        let parsed = parse_map_text(locator, &original)?;
        if parsed.mode == MapMode::Curated {
            return Ok(RegenerationOutcome::PreservedCurated);
        }
        let (start, end) = auto_region(&original)?;
        let replacement = format!("{AUTO_START}\n{derived_body}\n{AUTO_END}");
        let updated = format!("{}{}{}", &original[..start], replacement, &original[end..]);
        // The complete replacement is constructed and validated before the prior authority is touched.
        parse_map_text(locator, &updated)?;
        replace_map_atomically(&path, updated.as_bytes())?;
        Ok(RegenerationOutcome::UpdatedAuto)
    }
}

impl SourcePort for CodeMapSource {
    fn records(&self) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        Ok(self
            .snapshots()?
            .into_iter()
            .flat_map(|snapshot| snapshot.records().to_vec())
            .collect())
    }

    fn snapshot(&self) -> Result<Vec<SourceSnapshot>, FastSearchError> {
        self.snapshots()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MapMode {
    Auto,
    Curated,
}

struct ParsedMap {
    mode: MapMode,
    source: Option<String>,
    body: String,
}

fn canonical_root(root: &Path) -> Result<PathBuf, FastSearchError> {
    let root = root
        .canonicalize()
        .map_err(|error| source_failure("canonicalize map root", error))?;
    if root.is_dir() {
        Ok(root)
    } else {
        Err(invalid("map root must be a directory"))
    }
}

fn collect_map_paths(
    root: &Path,
    directory: &Path,
    depth: usize,
    paths: &mut Vec<PathBuf>,
) -> Result<(), FastSearchError> {
    if depth > MAX_MAP_DEPTH {
        return Err(invalid("map directory depth exceeds the configured limit"));
    }
    for entry in
        fs::read_dir(directory).map_err(|error| source_failure("read map directory", error))?
    {
        let entry = entry.map_err(|error| source_failure("read map directory entry", error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| source_failure("read map file type", error))?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_map_paths(root, &path, depth + 1, paths)?;
        } else if file_type.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".cfmap.md"))
        {
            let canonical = path
                .canonicalize()
                .map_err(|error| source_failure("canonicalize map", error))?;
            if canonical.strip_prefix(root).is_err() {
                return Err(invalid("map escapes configured root"));
            }
            if paths.len() == MAX_MAP_FILES {
                return Err(invalid("map file count exceeds the configured limit"));
            }
            ensure_bounded_map(&canonical)?;
            paths.push(canonical);
        }
    }
    Ok(())
}

fn contained_map_path(root: &Path, locator: &str) -> Result<PathBuf, FastSearchError> {
    let relative = Path::new(locator);
    if !locator.ends_with(".cfmap.md")
        || relative.is_absolute()
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(invalid("map locator must be a contained .cfmap.md path"));
    }
    let candidate = root
        .join(relative)
        .canonicalize()
        .map_err(|error| source_failure("canonicalize map", error))?;
    if candidate.strip_prefix(root).is_err() {
        return Err(invalid("map locator escapes configured root"));
    }
    ensure_bounded_map(&candidate)?;
    Ok(candidate)
}

fn parse_map_file(root: &Path, path: &Path) -> Result<SourceSnapshot, FastSearchError> {
    let locator = path
        .strip_prefix(root)
        .map_err(|_| invalid("map escapes configured root"))?
        .to_string_lossy()
        .replace('\\', "/");
    ensure_bounded_map(path)?;
    let bytes = fs::read(path).map_err(|error| source_failure("read map", error))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid("map must be UTF-8"))?;
    let parsed = parse_map_text(&locator, text)?;
    let state = if parsed.mode == MapMode::Auto
        && parsed
            .source
            .as_ref()
            .is_some_and(|source| !referenced_source_exists(root, source))
    {
        "STALE"
    } else {
        "CURRENT"
    };
    let mut metadata = BTreeMap::new();
    metadata.insert("schema".to_owned(), "cfmap-v1".to_owned());
    metadata.insert(
        "mode".to_owned(),
        if parsed.mode == MapMode::Auto {
            "AUTO"
        } else {
            "CURATED"
        }
        .to_owned(),
    );
    metadata.insert("state".to_owned(), state.to_owned());
    if let Some(source) = parsed.source {
        metadata.insert("source".to_owned(), source);
    }
    let locator_value = SourceLocator::whole_file(locator.clone())?;
    let digest = hex_digest(&bytes);
    let record = CanonicalRecord::new(
        StableId::parse(format!("cfmap-v1:{locator}"))?,
        RecordKind::CodeMap,
        locator_value.clone(),
        map_title(&parsed.body)?,
        parsed.body,
        metadata,
        Vec::new(),
        ContentHash::parse(digest.clone())?,
    )?;
    Ok(SourceSnapshot::new(
        locator_value,
        FileHash::parse(digest)?,
        vec![record],
    ))
}

fn parse_map_text(locator: &str, text: &str) -> Result<ParsedMap, FastSearchError> {
    let rest = text
        .strip_prefix(HEADER_START)
        .ok_or_else(|| invalid("cfmap requires v1 frontmatter"))?;
    let header_end = rest
        .find(HEADER_END)
        .ok_or_else(|| invalid("cfmap frontmatter is not terminated"))?;
    let header = &rest[..header_end];
    let body = rest[header_end + HEADER_END.len()..].to_owned();
    let mut fields = BTreeMap::new();
    for line in header.lines() {
        let (key, value) = line
            .split_once(": ")
            .ok_or_else(|| invalid("cfmap frontmatter requires key: value"))?;
        if key.is_empty() || value.is_empty() || fields.insert(key, value).is_some() {
            return Err(invalid("cfmap frontmatter has invalid or duplicate field"));
        }
    }
    if fields.get("cfmap") != Some(&"v1") {
        return Err(invalid("unsupported cfmap schema"));
    }
    let mode = match fields.get("mode").copied() {
        Some("AUTO") => MapMode::Auto,
        Some("CURATED") => MapMode::Curated,
        _ => return Err(invalid("cfmap mode must be AUTO or CURATED")),
    };
    if fields
        .keys()
        .any(|key| !matches!(*key, "cfmap" | "mode" | "source" | "generation"))
    {
        return Err(invalid("unsupported cfmap frontmatter field"));
    }
    if let Some(generation) = fields.get("generation") {
        generation
            .parse::<u64>()
            .map_err(|_| invalid("cfmap generation must be an unsigned integer"))?;
    }
    let source = fields.get("source").map(|value| (*value).to_owned());
    if mode == MapMode::Auto && source.is_none() {
        return Err(invalid("AUTO cfmap requires source"));
    }
    if mode == MapMode::Curated && (source.is_some() || fields.contains_key("generation")) {
        return Err(invalid(
            "CURATED cfmap cannot declare generated source or generation",
        ));
    }
    map_title(&body)?;
    if mode == MapMode::Auto {
        auto_region(text)?;
    }
    if locator.is_empty() {
        return Err(invalid("map locator must not be empty"));
    }
    Ok(ParsedMap { mode, source, body })
}

fn auto_region(text: &str) -> Result<(usize, usize), FastSearchError> {
    if text.match_indices(AUTO_START).count() != 1 || text.match_indices(AUTO_END).count() != 1 {
        return Err(invalid("AUTO cfmap requires exactly one generated region"));
    }
    let start = text
        .find(AUTO_START)
        .ok_or_else(|| invalid("AUTO cfmap requires one generated region"))?;
    let after_start = start + AUTO_START.len();
    let end_start = text[after_start..]
        .find(AUTO_END)
        .map(|offset| after_start + offset)
        .ok_or_else(|| invalid("AUTO cfmap generated region is not terminated"))?;
    if text[..start].contains(AUTO_END) {
        return Err(invalid("AUTO cfmap requires exactly one generated region"));
    }
    Ok((start, end_start + AUTO_END.len()))
}

fn referenced_source_exists(root: &Path, source: &str) -> bool {
    let relative = source.split('#').next().unwrap_or_default();
    !relative.is_empty()
        && Path::new(relative)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && root.join(relative).is_file()
}

fn map_title(body: &str) -> Result<String, FastSearchError> {
    body.lines()
        .find_map(|line| line.strip_prefix("# "))
        .filter(|title| !title.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid("cfmap body requires a non-empty H1 title"))
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn ensure_bounded_map(path: &Path) -> Result<(), FastSearchError> {
    let metadata =
        fs::metadata(path).map_err(|error| source_failure("read map metadata", error))?;
    if metadata.len() > MAX_MAP_BYTES {
        return Err(invalid("map exceeds the configured size limit"));
    }
    Ok(())
}

fn replace_map_atomically(path: &Path, bytes: &[u8]) -> Result<(), FastSearchError> {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| invalid("map has no parent directory"))?;
    let temporary = parent.join(format!(
        ".cfmap-replace-{}-{}",
        std::process::id(),
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| source_failure("create map replacement", error))?;
        file.write_all(bytes)
            .map_err(|error| source_failure("write map replacement", error))?;
        file.sync_all()
            .map_err(|error| source_failure("sync map replacement", error))?;
        fs::rename(&temporary, path)
            .map_err(|error| source_failure("replace map atomically", error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn invalid(message: impl Into<String>) -> FastSearchError {
    FastSearchError::new(ErrorKind::InvalidContent, message)
}
fn source_failure(context: &str, error: std::io::Error) -> FastSearchError {
    FastSearchError::new(ErrorKind::SourceFailure, format!("{context}: {error}"))
}
