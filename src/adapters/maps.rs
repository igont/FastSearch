//! Versioned read-only `.cfmap.md` admission.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::domain::{
    CanonicalRecord, ContentHash, ErrorKind, FastSearchError, FileHash, RecordKind, SourceLocator,
    SourceSnapshot, StableId,
};
use crate::ports::SourcePort;

const HEADER_START: &str = "---\n";
const HEADER_END: &str = "---\n";
const MAX_MAP_BYTES: u64 = 1_048_576;
const MAX_MAP_FILES: usize = 1_024;
const MAX_MAP_DEPTH: usize = 16;

/// Read-only source for a single configured map root.
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
            .is_some_and(|source| !referenced_source_is_current(root, source))
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
    if let Some(source) = &source {
        let locator = source.split('#').next().unwrap_or_default();
        if locator.is_empty()
            || !Path::new(locator)
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(invalid(
                "AUTO cfmap source must be a contained relative locator",
            ));
        }
    }
    if mode == MapMode::Curated && (source.is_some() || fields.contains_key("generation")) {
        return Err(invalid(
            "CURATED cfmap cannot declare generated source or generation",
        ));
    }
    map_title(&body)?;
    if locator.is_empty() {
        return Err(invalid("map locator must not be empty"));
    }
    Ok(ParsedMap { mode, source, body })
}

fn referenced_source_is_current(root: &Path, source: &str) -> bool {
    let relative = source.split('#').next().unwrap_or_default();
    if relative.is_empty()
        || !Path::new(relative)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return false;
    }
    let candidate = root.join(relative);
    candidate.is_file()
        && candidate
            .canonicalize()
            .is_ok_and(|canonical| canonical.strip_prefix(root).is_ok())
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

fn invalid(message: impl Into<String>) -> FastSearchError {
    FastSearchError::new(ErrorKind::InvalidContent, message)
}
fn source_failure(context: &str, error: std::io::Error) -> FastSearchError {
    FastSearchError::new(ErrorKind::SourceFailure, format!("{context}: {error}"))
}
