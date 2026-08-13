use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;

use crate::domain::{ErrorKind, FastSearchError};

const EXCLUDED_DIRECTORIES: &[&str] = &[
    ".agents",
    ".cfknowledge",
    ".git",
    ".obsidian",
    "build",
    "generated",
    "service",
    "target",
    "vendor",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScannedSourceKind {
    Markdown,
    Tsv,
}

/// Filesystem-admitted, bounded UTF-8 content for a format parser.
#[derive(Debug)]
pub(super) struct ScannedSource {
    pub(super) locator: String,
    pub(super) bytes: Vec<u8>,
    pub(super) kind: ScannedSourceKind,
}

pub(super) fn scan_sources(root: &Path) -> Result<Vec<ScannedSource>, FastSearchError> {
    let root = root
        .canonicalize()
        .map_err(|error| source_failure("canonicalize source root", error))?;
    if !root.is_dir() {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "source root must be a directory",
        ));
    }

    let mut sources = Vec::new();
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_global(false)
        .git_exclude(false)
        .git_ignore(true)
        .require_git(false)
        .follow_links(false);
    for entry in builder
        .filter_entry(|entry| {
            !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_dir())
                || !EXCLUDED_DIRECTORIES
                    .iter()
                    .any(|excluded| entry.file_name() == OsStr::new(excluded))
        })
        .build()
    {
        let entry = entry.map_err(|error| {
            FastSearchError::new(
                ErrorKind::SourceFailure,
                format!("walk source tree: {error}"),
            )
        })?;
        if entry.depth() == 0
            || !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        if let Some(kind) = source_kind(entry.path()) {
            sources.push(read_source(&root, entry.into_path(), kind)?);
        }
    }
    sources.sort_by(|left, right| left.locator.cmp(&right.locator));
    Ok(sources)
}

fn read_source(
    root: &Path,
    path: PathBuf,
    kind: ScannedSourceKind,
) -> Result<ScannedSource, FastSearchError> {
    let canonical_path = path
        .canonicalize()
        .map_err(|error| source_failure("canonicalize source file", error))?;
    let relative = canonical_path.strip_prefix(root).map_err(|_| {
        FastSearchError::new(
            ErrorKind::SourceFailure,
            "source file escapes configured root",
        )
    })?;
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "source locator is not a contained relative path",
        ));
    }
    let bytes =
        fs::read(&canonical_path).map_err(|error| source_failure("read source file", error))?;
    std::str::from_utf8(&bytes)
        .map_err(|_| FastSearchError::new(ErrorKind::SourceFailure, "source file is not UTF-8"))?;
    let locator = relative.to_string_lossy().replace('\\', "/");

    Ok(ScannedSource {
        locator,
        bytes,
        kind,
    })
}

fn source_kind(path: &Path) -> Option<ScannedSourceKind> {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".cfmap.md"))
    {
        return None;
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("md") => Some(ScannedSourceKind::Markdown),
        Some("tsv") => Some(ScannedSourceKind::Tsv),
        _ => None,
    }
}

fn source_failure(context: &str, error: std::io::Error) -> FastSearchError {
    FastSearchError::new(ErrorKind::SourceFailure, format!("{context}: {error}"))
}
