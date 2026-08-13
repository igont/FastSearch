//! Structural Rust/Python symbols from one named, replaceable code root.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

use crate::domain::SearchQuery;
use crate::domain::{
    CanonicalRecord, ContentHash, ErrorKind, FastSearchError, FileHash, LogicalRootId, RecordKind,
    RootedSourceLocator, SourceLocator, SourceSnapshot,
};
use crate::ports::{SourcePort, SymbolPort};

const MAX_SOURCE_BYTES: u64 = 64 * 1024;
const MAX_FILES: usize = 1_024;
const MAX_DEPTH: usize = 16;
const MAX_NODES: usize = 16_384;

/// A read-only source that emits only complete structural snapshots for an explicit named root.
#[derive(Debug)]
pub struct SymbolSource {
    root_id: LogicalRootId,
    root: PathBuf,
}

impl SymbolSource {
    #[must_use]
    pub fn new(root_id: LogicalRootId, root: impl Into<PathBuf>) -> Self {
        Self {
            root_id,
            root: root.into(),
        }
    }

    pub fn snapshots(&self) -> Result<Vec<SourceSnapshot>, FastSearchError> {
        let root = canonical_root(&self.root)?;
        let mut files = Vec::new();
        collect_files(&root, &root, 0, &mut files)?;
        let mut ordered_files = files
            .into_iter()
            .map(|path| Ok((normalized_relative(&root, &path)?, path)))
            .collect::<Result<Vec<_>, FastSearchError>>()?;
        ordered_files.sort_by(|left, right| left.0.cmp(&right.0));
        ordered_files
            .into_iter()
            .map(|(_, path)| self.parse_file(&root, &path))
            .collect()
    }

    fn parse_file(&self, root: &Path, path: &Path) -> Result<SourceSnapshot, FastSearchError> {
        let locator = normalized_relative(root, path)?;
        ensure_bounded_file(path)?;
        let bytes = fs::read(path).map_err(|e| source_failure("read code file", e))?;
        let text = std::str::from_utf8(&bytes).map_err(|_| invalid("code file must be UTF-8"))?;
        let (language_name, language, query_text) = language_for(path)?;
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|_| invalid("load code grammar"))?;
        let tree = parser
            .parse(text, None)
            .ok_or_else(|| invalid("parser produced no tree"))?;
        if tree.root_node().has_error() {
            return Err(invalid("parse error publishes no partial symbols"));
        }
        let mut stack = vec![tree.root_node()];
        let mut visited = 0;
        while let Some(node) = stack.pop() {
            visited += 1;
            if visited > MAX_NODES {
                return Err(invalid("parse tree exceeds configured node limit"));
            }
            let mut cursor = node.walk();
            stack.extend(node.children(&mut cursor));
        }
        let query = Query::new(&language, query_text)
            .map_err(|_| invalid("symbol query incompatible with grammar"))?;
        let mut cursor = QueryCursor::new();
        let mut captures = cursor.captures(&query, tree.root_node(), bytes.as_slice());
        let mut records = Vec::new();
        while let Some((matched, index)) = captures.next() {
            let declaration = matched.captures[*index].node;
            let name = declaration
                .child_by_field_name("name")
                .and_then(|node| node.utf8_text(&bytes).ok())
                .ok_or_else(|| invalid("declaration has no UTF-8 name"))?;
            let kind = structural_kind(language_name, declaration.kind())
                .ok_or_else(|| invalid("unexpected declaration capture"))?;
            let selector = format!("{language_name}:{kind}:{name}:{}", declaration.start_byte());
            let code_locator = SourceLocator::code_symbol(locator.clone(), selector.clone())?;
            let id =
                RootedSourceLocator::new(self.root_id.clone(), code_locator.clone())?.stable_id();
            let mut metadata = BTreeMap::new();
            metadata.insert("language".to_owned(), language_name.to_owned());
            metadata.insert("structural_kind".to_owned(), kind.to_owned());
            metadata.insert(
                "fact_kind".to_owned(),
                "structural source symbol".to_owned(),
            );
            metadata.insert(
                "start_byte".to_owned(),
                declaration.start_byte().to_string(),
            );
            let content = format!("{language_name} {kind} {name}");
            records.push(CanonicalRecord::new(
                id,
                RecordKind::CodeSymbol,
                code_locator,
                name,
                content.clone(),
                metadata,
                Vec::new(),
                ContentHash::parse(hex_digest(content.as_bytes()))?,
            )?);
        }
        records.sort_by(|a, b| a.id().cmp(b.id()));
        Ok(SourceSnapshot::for_root(
            self.root_id.clone(),
            SourceLocator::whole_file(locator)?,
            FileHash::parse(hex_digest(&bytes))?,
            records,
        ))
    }
}

impl SourcePort for SymbolSource {
    fn records(&self) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        Ok(self
            .snapshots()?
            .into_iter()
            .flat_map(|s| s.records().to_vec())
            .collect())
    }
    fn snapshot(&self) -> Result<Vec<SourceSnapshot>, FastSearchError> {
        self.snapshots()
    }
}

impl SymbolPort for SymbolSource {
    fn find_symbols(&self, query: &SearchQuery) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        let needle = query.text().to_lowercase();
        let mut found = self
            .records()?
            .into_iter()
            .filter(|r| r.title().to_lowercase().contains(&needle))
            .collect::<Vec<_>>();
        found.sort_by(|a, b| a.id().cmp(b.id()));
        Ok(found)
    }
}

fn language_for(path: &Path) -> Result<(&'static str, Language, &'static str), FastSearchError> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Ok((
            "rust",
            tree_sitter_rust::LANGUAGE.into(),
            "(function_item) @declaration\n(struct_item) @declaration",
        )),
        Some("py") => Ok((
            "python",
            tree_sitter_python::LANGUAGE.into(),
            "(function_definition) @declaration\n(class_definition) @declaration",
        )),
        _ => Err(invalid("unsupported code language")),
    }
}
fn structural_kind(language: &str, node: &str) -> Option<&'static str> {
    match (language, node) {
        ("rust", "function_item") => Some("function"),
        ("rust", "struct_item") => Some("struct"),
        ("python", "function_definition") => Some("function"),
        ("python", "class_definition") => Some("class"),
        _ => None,
    }
}
fn canonical_root(root: &Path) -> Result<PathBuf, FastSearchError> {
    let root = root
        .canonicalize()
        .map_err(|e| source_failure("canonicalize code root", e))?;
    root.is_dir()
        .then_some(root)
        .ok_or_else(|| invalid("code root must be a directory"))
}
fn collect_files(
    root: &Path,
    dir: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
) -> Result<(), FastSearchError> {
    if depth > MAX_DEPTH {
        return Err(invalid("code directory depth exceeds configured limit"));
    }
    for entry in fs::read_dir(dir).map_err(|e| source_failure("read code directory", e))? {
        let entry = entry.map_err(|e| source_failure("read code directory entry", e))?;
        let ty = entry
            .file_type()
            .map_err(|e| source_failure("read code file type", e))?;
        if ty.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ty.is_dir() {
            collect_files(root, &path, depth + 1, files)?;
        } else if ty.is_file() {
            if !matches!(path.extension().and_then(|e| e.to_str()), Some("rs" | "py")) {
                return Err(invalid("unsupported code language"));
            }
            let canonical = path
                .canonicalize()
                .map_err(|e| source_failure("canonicalize code file", e))?;
            if canonical.strip_prefix(root).is_err() {
                return Err(invalid("code file escapes configured root"));
            }
            if files.len() == MAX_FILES {
                return Err(invalid("code file count exceeds configured limit"));
            }
            files.push(canonical);
        }
    }
    Ok(())
}
fn ensure_bounded_file(path: &Path) -> Result<(), FastSearchError> {
    let length = fs::metadata(path)
        .map_err(|e| source_failure("read code file metadata", e))?
        .len();
    if length > MAX_SOURCE_BYTES {
        return Err(invalid("code file exceeds configured size limit"));
    }
    Ok(())
}
fn normalized_relative(root: &Path, path: &Path) -> Result<String, FastSearchError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| invalid("code file escapes configured root"))?;
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(invalid("code locator is not contained relative path"));
    }
    Ok(relative.to_string_lossy().replace('\\', "/"))
}
fn hex_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn invalid(message: impl Into<String>) -> FastSearchError {
    FastSearchError::new(ErrorKind::InvalidContent, message)
}
fn source_failure(context: &str, error: std::io::Error) -> FastSearchError {
    FastSearchError::new(ErrorKind::SourceFailure, format!("{context}: {error}"))
}
