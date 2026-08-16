//! Versioned read-only `.cfmap.md` admission.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::domain::RelatedQuery;
use crate::domain::{
    CanonicalRecord, ContentHash, ErrorKind, FastSearchError, FileHash, LogicalRootId, RecordKind,
    RootedSourceLocator, SourceLocator, SourceSnapshot, StableId,
};
use crate::ports::{CodeMapPort, SourcePort};

const HEADER_START: &str = "---\n";
const HEADER_END: &str = "---\n";
const MAX_MAP_BYTES: u64 = 1_048_576;
const MAX_MAP_FILES: usize = 1_024;
const MAX_MAP_DEPTH: usize = 16;

/// Read-only source for a single configured map root.
#[derive(Debug)]
pub struct CodeMapSource {
    root: PathBuf,
    root_id: Option<LogicalRootId>,
}

/// In-memory projection of explicit map facts. It never follows a relation recursively.
#[derive(Debug)]
pub struct CodeMapRelated {
    records: BTreeMap<StableId, CanonicalRecord>,
}

impl CodeMapRelated {
    pub fn new(
        records: impl IntoIterator<Item = CanonicalRecord>,
    ) -> Result<Self, FastSearchError> {
        let mut indexed = BTreeMap::new();
        for record in records {
            if indexed.insert(record.id().clone(), record).is_some() {
                return Err(FastSearchError::new(
                    ErrorKind::DuplicateStableId,
                    "map relation projection contains duplicate stable IDs",
                ));
            }
        }
        Ok(Self { records: indexed })
    }
}

impl CodeMapPort for CodeMapRelated {
    fn related_maps(&self, query: &RelatedQuery) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        let source = self.records.get(query.id()).ok_or_else(|| {
            FastSearchError::new(ErrorKind::NotFound, "related map source is not present")
        })?;
        if source.kind() != RecordKind::CodeMap {
            return Err(FastSearchError::new(
                ErrorKind::NotFound,
                "related navigation requires a code map source",
            ));
        }
        if source
            .metadata()
            .get("state")
            .is_some_and(|state| state == "STALE")
        {
            return Err(FastSearchError::new(
                ErrorKind::StateFailure,
                "stale code map cannot publish related navigation",
            ));
        }
        let mut target_ids = BTreeSet::new();
        target_ids.extend(source.relations().iter().cloned());
        target_ids
            .into_iter()
            .map(|id| {
                let target = self.records.get(&id).ok_or_else(|| {
                    FastSearchError::new(ErrorKind::NotFound, "explicit map relation is dangling")
                })?;
                related_with_provenance(target, source.id())
            })
            .collect()
    }
}

impl CodeMapSource {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            root_id: None,
        }
    }

    #[must_use]
    pub fn new_named(root_id: LogicalRootId, root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            root_id: Some(root_id),
        }
    }

    /// Validates every map before returning a complete, deterministic snapshot set.
    pub fn snapshots(&self) -> Result<Vec<SourceSnapshot>, FastSearchError> {
        let root = canonical_root(&self.root)?;
        let mut paths = Vec::new();
        collect_map_paths(&root, &root, 0, &mut paths)?;
        paths.sort();
        paths
            .into_iter()
            .map(|path| parse_map_file(&root, &path, self.root_id.as_ref()))
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
    relations: Vec<StableId>,
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
            if !excluded_directory(&path) {
                collect_map_paths(root, &path, depth + 1, paths)?;
            }
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

fn excluded_directory(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        matches!(
            name.to_str(),
            Some(
                ".fastsearch"
                    | ".git"
                    | ".venv"
                    | "build"
                    | "dist"
                    | "node_modules"
                    | "target"
                    | "vendor"
            )
        )
    })
}

fn parse_map_file(
    root: &Path,
    path: &Path,
    root_id: Option<&LogicalRootId>,
) -> Result<SourceSnapshot, FastSearchError> {
    let locator = path
        .strip_prefix(root)
        .map_err(|_| invalid("map escapes configured root"))?
        .to_string_lossy()
        .replace('\\', "/");
    ensure_bounded_map(path)?;
    let bytes = fs::read(path).map_err(|error| source_failure("read map", error))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid("map must be UTF-8"))?;
    let parsed = parse_map_text(&locator, text)?;
    let state = match (parsed.mode, parsed.source.as_deref()) {
        (MapMode::Auto, Some(source)) => match referenced_source_state(root, source)? {
            ReferencedSourceState::Current => "CURRENT",
            ReferencedSourceState::Missing => "STALE",
        },
        _ => "CURRENT",
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
    let id = match root_id {
        Some(root_id) => {
            RootedSourceLocator::new(root_id.clone(), locator_value.clone())?.stable_id()
        }
        None => StableId::parse(format!("cfmap-v1:{locator}"))?,
    };
    let record = CanonicalRecord::new(
        id,
        RecordKind::CodeMap,
        locator_value.clone(),
        map_title(&parsed.body)?,
        parsed.body,
        metadata,
        parsed.relations,
        ContentHash::parse(digest.clone())?,
    )?;
    let file_hash = FileHash::parse(digest)?;
    Ok(match root_id {
        Some(root_id) => {
            SourceSnapshot::for_root(root_id.clone(), locator_value, file_hash, vec![record])
        }
        None => SourceSnapshot::new(locator_value, file_hash, vec![record]),
    })
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
    let relations = body
        .lines()
        .filter_map(|line| line.strip_prefix("@related "))
        .map(|value| StableId::parse(value.to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    if body
        .lines()
        .any(|line| line.starts_with("@related") && !line.starts_with("@related "))
    {
        return Err(invalid("map relation must use @related <stable-id>"));
    }
    Ok(ParsedMap {
        mode,
        source,
        body,
        relations,
    })
}

enum ReferencedSourceState {
    Current,
    Missing,
}

fn referenced_source_state(
    root: &Path,
    source: &str,
) -> Result<ReferencedSourceState, FastSearchError> {
    let relative = source.split('#').next().unwrap_or_default();
    if relative.is_empty()
        || !Path::new(relative)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(invalid(
            "AUTO cfmap source must be a contained relative locator",
        ));
    }
    let candidate = root.join(relative);
    if !candidate.exists() {
        return Ok(ReferencedSourceState::Missing);
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|error| source_failure("canonicalize AUTO cfmap source", error))?;
    if canonical.strip_prefix(root).is_err() {
        return Err(invalid("AUTO cfmap source escapes configured root"));
    }
    if !canonical.is_file() {
        return Err(invalid("AUTO cfmap source must be a file"));
    }
    Ok(ReferencedSourceState::Current)
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

fn related_with_provenance(
    target: &CanonicalRecord,
    source_id: &StableId,
) -> Result<CanonicalRecord, FastSearchError> {
    let mut metadata = target.metadata().clone();
    metadata.insert(
        "relation_provenance".to_owned(),
        format!("explicit-map:{}", source_id.as_str()),
    );
    CanonicalRecord::new(
        target.id().clone(),
        target.kind(),
        target.locator().clone(),
        target.title(),
        target.searchable_content(),
        metadata,
        target.relations().to_vec(),
        target.content_hash().clone(),
    )
}

fn invalid(message: impl Into<String>) -> FastSearchError {
    FastSearchError::new(ErrorKind::InvalidContent, message)
}
fn source_failure(context: &str, error: std::io::Error) -> FastSearchError {
    FastSearchError::new(ErrorKind::SourceFailure, format!("{context}: {error}"))
}
