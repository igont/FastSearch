//! Persistent FastSearch workspaces, source discovery and the machine-local catalog.

use std::{
    collections::BTreeSet,
    env, fs,
    fs::OpenOptions,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{EmbeddingModelId, ErrorKind, FastSearchError};

use super::production::ProductionConfig;

const WORKSPACE_SCHEMA: u32 = 1;
const CATALOG_SCHEMA: u32 = 1;
const MAX_DISCOVERY_DIRECTORIES: usize = 4_096;
const MAX_DISCOVERY_DEPTH: usize = 6;
const LOCAL_IGNORE: &str = "/local/\n";
static NEXT_ATOMIC_WRITE: AtomicU64 = AtomicU64::new(1);
const DEFAULT_EXCLUSIONS: &[&str] = &[
    ".agents",
    ".fastsearch",
    ".git",
    ".obsidian",
    ".venv",
    "build",
    "dist",
    "generated",
    "node_modules",
    "target",
    "vendor",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceRoot {
    id: String,
    path: String,
}

impl SourceRoot {
    fn admitted(contour: &str, path: String) -> Result<Self, FastSearchError> {
        validate_relative_root(&path)?;
        let mut hasher = Sha256::new();
        hasher.update(b"fastsearch-root-v1\0");
        hasher.update(contour.as_bytes());
        hasher.update(b"\0");
        hasher.update(path.as_bytes());
        let digest = format!("{:x}", hasher.finalize());
        Ok(Self {
            id: format!("{contour}-{}", &digest[..16]),
            path,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn resolve(&self, workspace_root: &Path) -> PathBuf {
        if self.path == "." {
            workspace_root.to_path_buf()
        } else {
            workspace_root.join(&self.path)
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct SourceContour {
    #[serde(default)]
    roots: Vec<SourceRoot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Exclusions {
    common: Vec<String>,
}

impl Default for Exclusions {
    fn default() -> Self {
        Self {
            common: DEFAULT_EXCLUSIONS
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceProfile {
    schema_version: u32,
    id: String,
    name: String,
    documentation: SourceContour,
    code: SourceContour,
    #[serde(default)]
    embedding_model: EmbeddingModelId,
    #[serde(default)]
    exclude: Exclusions,
}

impl WorkspaceProfile {
    pub fn from_roots(
        workspace_root: &Path,
        name: impl Into<String>,
        documentation: impl IntoIterator<Item = PathBuf>,
        code: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, FastSearchError> {
        let canonical_root = canonical_directory(workspace_root, "workspace root")?;
        let name = name.into();
        let name = if name.trim().is_empty() {
            workspace_display_name(&canonical_root)
        } else {
            name.trim().to_owned()
        };
        let documentation = admit_roots(&canonical_root, "documentation", documentation)?;
        let code = admit_roots(&canonical_root, "code", code)?;
        Ok(Self {
            schema_version: WORKSPACE_SCHEMA,
            id: workspace_id(&canonical_root),
            name,
            documentation: SourceContour {
                roots: documentation,
            },
            code: SourceContour { roots: code },
            embedding_model: EmbeddingModelId::default(),
            exclude: Exclusions::default(),
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn documentation_roots(&self) -> &[SourceRoot] {
        &self.documentation.roots
    }

    #[must_use]
    pub fn code_roots(&self) -> &[SourceRoot] {
        &self.code.roots
    }

    #[must_use]
    pub fn contour_count(&self) -> usize {
        usize::from(!self.documentation.roots.is_empty()) + usize::from(!self.code.roots.is_empty())
    }

    #[must_use]
    pub const fn embedding_model(&self) -> EmbeddingModelId {
        self.embedding_model
    }

    #[must_use]
    pub fn with_embedding_model(mut self, model: EmbeddingModelId) -> Self {
        self.embedding_model = model;
        self
    }

    #[must_use]
    pub fn exclusions(&self) -> &[String] {
        &self.exclude.common
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiscoveryReport {
    documentation: Vec<PathBuf>,
    code: Vec<PathBuf>,
}

impl DiscoveryReport {
    pub fn scan(workspace_root: &Path) -> Result<Self, FastSearchError> {
        let root = canonical_directory(workspace_root, "workspace root")?;
        let mut directories = Vec::new();
        collect_directories(&root, &root, 0, &mut directories)?;
        let mut document_candidates = BTreeSet::new();
        let mut code_candidates = BTreeSet::new();
        for directory in directories {
            let facts = inspect_directory(&directory)?;
            if facts.document_marker || facts.document_files > 0 {
                document_candidates.insert(directory.clone());
            }
            if facts.code_marker || facts.code_files > 0 {
                code_candidates.insert(directory);
            }
        }
        Ok(Self {
            documentation: collapse_candidates(&root, document_candidates, CandidateKind::Document),
            code: collapse_candidates(&root, code_candidates, CandidateKind::Code),
        })
    }

    #[must_use]
    pub fn documentation_roots(&self) -> &[PathBuf] {
        &self.documentation
    }

    #[must_use]
    pub fn code_roots(&self) -> &[PathBuf] {
        &self.code
    }

    pub fn into_profile(
        self,
        workspace_root: &Path,
        name: impl Into<String>,
    ) -> Result<WorkspaceProfile, FastSearchError> {
        WorkspaceProfile::from_roots(workspace_root, name, self.documentation, self.code)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogEntry {
    id: String,
    name: String,
    path: PathBuf,
    last_opened_unix_ms: u128,
    workspace_schema: u32,
}

impl CatalogEntry {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceCatalog {
    schema_version: u32,
    entries: Vec<CatalogEntry>,
}

impl Default for WorkspaceCatalog {
    fn default() -> Self {
        Self {
            schema_version: CATALOG_SCHEMA,
            entries: Vec::new(),
        }
    }
}

impl WorkspaceCatalog {
    pub fn load_default() -> Result<Self, FastSearchError> {
        Self::load(&catalog_path()?)
    }

    pub fn load(path: &Path) -> Result<Self, FastSearchError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = fs::read(path)
            .map_err(|error| failure(ErrorKind::StateFailure, "read workspace catalog", error))?;
        let catalog: Self = serde_json::from_slice(&bytes).map_err(|error| {
            FastSearchError::new(
                ErrorKind::InvalidContent,
                format!("parse workspace catalog: {error}"),
            )
        })?;
        if catalog.schema_version != CATALOG_SCHEMA {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "workspace catalog schema is not supported",
            ));
        }
        Ok(catalog)
    }

    pub fn save_default(&self) -> Result<(), FastSearchError> {
        self.save(&catalog_path()?)
    }

    pub fn save(&self, path: &Path) -> Result<(), FastSearchError> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| {
            FastSearchError::new(
                ErrorKind::StateFailure,
                format!("serialize workspace catalog: {error}"),
            )
        })?;
        atomic_write(path, &bytes)
    }

    pub fn register(
        &mut self,
        root: &Path,
        profile: &WorkspaceProfile,
    ) -> Result<(), FastSearchError> {
        let root = canonical_directory(root, "workspace root")?;
        self.entries.retain(|entry| entry.id != profile.id);
        self.entries.push(CatalogEntry {
            id: profile.id.clone(),
            name: profile.name.clone(),
            path: root,
            last_opened_unix_ms: now_unix_ms(),
            workspace_schema: profile.schema_version,
        });
        self.entries.sort_by(|left, right| {
            right
                .last_opened_unix_ms
                .cmp(&left.last_opened_unix_ms)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(())
    }

    #[must_use]
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    pub fn resolve_path(&self, path: &Path) -> Option<&CatalogEntry> {
        let canonical = path.canonicalize().ok()?;
        self.entries
            .iter()
            .filter(|entry| canonical.starts_with(&entry.path))
            .max_by_key(|entry| entry.path.components().count())
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        before != self.entries.len()
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceStore {
    root: PathBuf,
    profile: WorkspaceProfile,
}

impl WorkspaceStore {
    pub fn create(root: &Path, profile: WorkspaceProfile) -> Result<Self, FastSearchError> {
        let root = canonical_directory(root, "workspace root")?;
        validate_profile(&root, &profile)?;
        let namespace = root.join(".fastsearch");
        ensure_plain_directory(&namespace)?;
        ensure_plain_directory(&namespace.join("knowledge"))?;
        ensure_plain_directory(&namespace.join("knowledge").join("curated"))?;
        for path in [
            namespace.join("local"),
            namespace.join("local").join("index"),
            namespace.join("local").join("index").join("documents"),
            namespace.join("local").join("index").join("code"),
            namespace.join("local").join("index").join("cross"),
            namespace.join("local").join("index").join("vector"),
            namespace.join("local").join("knowledge"),
            namespace.join("local").join("knowledge").join("graph"),
            namespace.join("local").join("knowledge").join("candidates"),
            namespace.join("local").join("knowledge").join("revisions"),
            namespace.join("local").join("cache"),
            namespace.join("local").join("runtime"),
            namespace.join("local").join("experiments"),
            namespace.join("local").join("experiments").join("runs"),
        ] {
            ensure_plain_directory(&path)?;
        }
        atomic_write(&namespace.join(".gitignore"), LOCAL_IGNORE.as_bytes())?;
        let encoded = toml::to_string_pretty(&profile).map_err(|error| {
            FastSearchError::new(
                ErrorKind::StateFailure,
                format!("serialize workspace configuration: {error}"),
            )
        })?;
        atomic_write(&namespace.join("workspace.toml"), encoded.as_bytes())?;
        Ok(Self { root, profile })
    }

    pub fn open(root: &Path) -> Result<Self, FastSearchError> {
        let root = canonical_directory(root, "workspace root")?;
        let path = root.join(".fastsearch").join("workspace.toml");
        let text = fs::read_to_string(&path).map_err(|error| {
            failure(
                ErrorKind::SourceFailure,
                "read .fastsearch/workspace.toml",
                error,
            )
        })?;
        let profile: WorkspaceProfile = toml::from_str(&text).map_err(|error| {
            FastSearchError::new(
                ErrorKind::InvalidContent,
                format!("parse .fastsearch/workspace.toml: {error}"),
            )
        })?;
        validate_profile(&root, &profile)?;
        Ok(Self { root, profile })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn profile(&self) -> &WorkspaceProfile {
        &self.profile
    }

    #[must_use]
    pub fn local_root(&self) -> PathBuf {
        ordinary_windows_path(self.root.join(".fastsearch").join("local"))
    }

    #[must_use]
    pub fn production_config(&self) -> ProductionConfig {
        ProductionConfig::for_workspace(
            self.root.clone(),
            self.profile
                .documentation_roots()
                .iter()
                .map(|source| (source.id().to_owned(), source.resolve(&self.root)))
                .collect(),
            self.profile
                .code_roots()
                .iter()
                .map(|source| (source.id().to_owned(), source.resolve(&self.root)))
                .collect(),
            self.local_root(),
        )
    }

    /// Removes only disposable vector projections for one model or the whole
    /// model catalog. Canonical state and the shared lexical projection stay
    /// intact.
    pub fn clear_model_indexes(
        &self,
        model: Option<EmbeddingModelId>,
    ) -> Result<(), FastSearchError> {
        let vector_root = self.local_root().join("index").join("vector");
        let target = match model {
            Some(model) => vector_root.join(model.slug()),
            None => vector_root,
        };
        if target.exists() {
            fs::remove_dir_all(&target)
                .map_err(|error| failure(ErrorKind::StateFailure, "clear model index", error))?;
        }
        Ok(())
    }

    pub fn set_embedding_model(
        &mut self,
        model: EmbeddingModelId,
    ) -> Result<bool, FastSearchError> {
        if self.profile.embedding_model == model {
            return Ok(false);
        }
        self.profile.embedding_model = model;
        let encoded = toml::to_string_pretty(&self.profile).map_err(|error| {
            FastSearchError::new(
                ErrorKind::StateFailure,
                format!("serialize workspace configuration: {error}"),
            )
        })?;
        atomic_write(
            &self.root.join(".fastsearch").join("workspace.toml"),
            encoded.as_bytes(),
        )?;
        Ok(true)
    }

    pub fn record_embedding_experiment(
        &self,
        query: &str,
        hit_count: usize,
        latency_ms: u128,
        note: &str,
    ) -> Result<PathBuf, FastSearchError> {
        let directory = self
            .root
            .join(".fastsearch")
            .join("knowledge")
            .join("experiments");
        ensure_plain_directory(&directory)?;
        let path = directory.join("embedding-models.jsonl");
        let entry = serde_json::json!({
            "schema_version": 1,
            "recorded_unix_ms": now_unix_ms(),
            "workspace_id": self.profile.id(),
            "model": self.profile.embedding_model().slug(),
            "query": query,
            "hit_count": hit_count,
            "latency_ms": latency_ms,
            "note": note.trim(),
        });
        let mut output = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| failure(ErrorKind::StateFailure, "open experiment journal", error))?;
        writeln!(
            output,
            "{}",
            serde_json::to_string(&entry).expect("JSON value serializes")
        )
        .map_err(|error| failure(ErrorKind::StateFailure, "write experiment journal", error))?;
        output
            .sync_data()
            .map_err(|error| failure(ErrorKind::StateFailure, "sync experiment journal", error))?;
        Ok(path)
    }

    #[must_use]
    pub fn legacy_locations(&self) -> Vec<PathBuf> {
        [self.root.join(".cfknowledge"), self.root.join(".search")]
            .into_iter()
            .filter(|path| path.exists())
            .collect()
    }
}

fn admit_roots(
    workspace_root: &Path,
    contour: &str,
    roots: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<SourceRoot>, FastSearchError> {
    let mut admitted = BTreeSet::new();
    for root in roots {
        let canonical = canonical_directory(&root, "source root")?;
        let relative = canonical.strip_prefix(workspace_root).map_err(|_| {
            FastSearchError::new(
                ErrorKind::InvalidContent,
                "source root must be contained by workspace root",
            )
        })?;
        let relative = if relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            relative.to_string_lossy().replace('\\', "/")
        };
        admitted.insert(relative);
    }
    admitted
        .into_iter()
        .map(|path| SourceRoot::admitted(contour, path))
        .collect()
}

fn validate_profile(root: &Path, profile: &WorkspaceProfile) -> Result<(), FastSearchError> {
    if profile.schema_version != WORKSPACE_SCHEMA {
        return Err(FastSearchError::new(
            ErrorKind::InvalidContent,
            "workspace schema is not supported",
        ));
    }
    if profile.id.trim().is_empty() || profile.name.trim().is_empty() {
        return Err(FastSearchError::new(
            ErrorKind::InvalidContent,
            "workspace identity and name must not be blank",
        ));
    }
    let mut ids = BTreeSet::new();
    for source in profile
        .documentation
        .roots
        .iter()
        .chain(profile.code.roots.iter())
    {
        validate_relative_root(&source.path)?;
        if !ids.insert(source.id.as_str()) {
            return Err(FastSearchError::new(
                ErrorKind::DuplicateStableId,
                "workspace source root IDs must be unique",
            ));
        }
        canonical_directory(&source.resolve(root), "source root")?;
    }
    Ok(())
}

fn validate_relative_root(path: &str) -> Result<(), FastSearchError> {
    if path == "." {
        return Ok(());
    }
    let path = Path::new(path);
    if path.is_absolute()
        || path.as_os_str().is_empty()
        || path
            .components()
            .next()
            .is_some_and(|component| component.as_os_str() == ".fastsearch")
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(FastSearchError::new(
            ErrorKind::InvalidContent,
            "workspace source root must be a normalized relative path",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum CandidateKind {
    Document,
    Code,
}

fn collapse_candidates(
    workspace_root: &Path,
    candidates: BTreeSet<PathBuf>,
    kind: CandidateKind,
) -> Vec<PathBuf> {
    let mut ordered = candidates.into_iter().collect::<Vec<_>>();
    ordered.sort_by_key(|path| path.components().count());
    let root_is_candidate = ordered.iter().any(|path| path == workspace_root);
    if root_is_candidate {
        let preferred_children = ordered
            .iter()
            .filter(|path| *path != workspace_root)
            .filter(|path| preferred_directory(path, kind))
            .cloned()
            .collect::<Vec<_>>();
        if !preferred_children.is_empty() {
            return remove_nested(preferred_children);
        }
        return vec![workspace_root.to_path_buf()];
    }
    remove_nested(ordered)
}

fn remove_nested(candidates: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut selected = Vec::<PathBuf>::new();
    for candidate in candidates {
        if !selected.iter().any(|root| candidate.starts_with(root)) {
            selected.push(candidate);
        }
    }
    selected
}

fn preferred_directory(path: &Path, kind: CandidateKind) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match kind {
        CandidateKind::Document => {
            matches!(
                name.as_str(),
                "docs" | "doc" | "documentation" | "specifications" | "architecture" | "obsidian"
            ) || path.join(".obsidian").is_dir()
        }
        CandidateKind::Code => path.join(".git").is_dir() || has_code_manifest(path),
    }
}

#[derive(Default)]
struct DirectoryFacts {
    document_marker: bool,
    code_marker: bool,
    document_files: usize,
    code_files: usize,
}

fn inspect_directory(path: &Path) -> Result<DirectoryFacts, FastSearchError> {
    let mut facts = DirectoryFacts {
        document_marker: path.join(".obsidian").is_dir()
            || preferred_directory(path, CandidateKind::Document),
        code_marker: path.join(".git").is_dir() || has_code_manifest(path),
        ..DirectoryFacts::default()
    };
    for entry in fs::read_dir(path).map_err(|error| {
        failure(
            ErrorKind::SourceFailure,
            "inspect discovery directory",
            error,
        )
    })? {
        let entry = entry.map_err(|error| {
            failure(
                ErrorKind::SourceFailure,
                "inspect discovery directory entry",
                error,
            )
        })?;
        if !entry
            .file_type()
            .map_err(|error| failure(ErrorKind::SourceFailure, "inspect discovery file", error))?
            .is_file()
        {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(extension.as_str(), "md" | "tsv") {
            facts.document_files += 1;
        }
        if matches!(
            extension.as_str(),
            "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "cs" | "cpp" | "c" | "h"
        ) {
            facts.code_files += 1;
        }
    }
    Ok(facts)
}

fn has_code_manifest(path: &Path) -> bool {
    [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "CMakeLists.txt",
        "*.sln",
    ]
    .iter()
    .any(|name| {
        if *name == "*.sln" {
            fs::read_dir(path).ok().is_some_and(|entries| {
                entries.flatten().any(|entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "sln")
                })
            })
        } else {
            path.join(name).is_file()
        }
    })
}

fn collect_directories(
    workspace_root: &Path,
    directory: &Path,
    depth: usize,
    directories: &mut Vec<PathBuf>,
) -> Result<(), FastSearchError> {
    if directories.len() >= MAX_DISCOVERY_DIRECTORIES || depth > MAX_DISCOVERY_DEPTH {
        return Ok(());
    }
    directories.push(directory.to_path_buf());
    for entry in fs::read_dir(directory)
        .map_err(|error| failure(ErrorKind::SourceFailure, "walk workspace discovery", error))?
    {
        let entry = entry.map_err(|error| {
            failure(
                ErrorKind::SourceFailure,
                "walk workspace discovery entry",
                error,
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            failure(
                ErrorKind::SourceFailure,
                "inspect workspace discovery entry",
                error,
            )
        })?;
        if !file_type.is_dir() || file_type.is_symlink() || excluded_name(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        let canonical = path.canonicalize().map_err(|error| {
            failure(
                ErrorKind::SourceFailure,
                "canonicalize workspace discovery directory",
                error,
            )
        })?;
        if canonical.starts_with(workspace_root) {
            collect_directories(workspace_root, &canonical, depth + 1, directories)?;
        }
    }
    Ok(())
}

fn excluded_name(name: &std::ffi::OsStr) -> bool {
    DEFAULT_EXCLUSIONS
        .iter()
        .any(|excluded| name == std::ffi::OsStr::new(excluded))
}

fn workspace_id(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"fastsearch-workspace-v1\0");
    hasher.update(root.to_string_lossy().as_bytes());
    format!("workspace-{:x}", hasher.finalize())
}

fn workspace_display_name(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Workspace")
        .to_owned()
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, FastSearchError> {
    let canonical = path.canonicalize().map_err(|error| {
        failure(
            ErrorKind::SourceFailure,
            &format!("canonicalize {label}"),
            error,
        )
    })?;
    if !canonical.is_dir() {
        return Err(FastSearchError::new(
            ErrorKind::InvalidContent,
            format!("{label} must be a directory"),
        ));
    }
    Ok(canonical)
}

fn ensure_plain_directory(path: &Path) -> Result<(), FastSearchError> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            failure(ErrorKind::SourceFailure, "inspect workspace storage", error)
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "workspace storage path must be a plain directory",
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(FastSearchError::new(
                    ErrorKind::InvalidContent,
                    "workspace storage path must not be a reparse point",
                ));
            }
        }
        return Ok(());
    }
    fs::create_dir(path)
        .map_err(|error| failure(ErrorKind::StateFailure, "create workspace storage", error))
}

fn catalog_path() -> Result<PathBuf, FastSearchError> {
    Ok(product_home()?.join("catalog.json"))
}

pub(crate) fn product_home() -> Result<PathBuf, FastSearchError> {
    if let Some(explicit) = env::var_os("FASTSEARCH_HOME") {
        return Ok(PathBuf::from(explicit));
    }
    #[cfg(windows)]
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(local).join("FastSearch"));
    }
    #[cfg(not(windows))]
    {
        if let Some(xdg) = env::var_os("XDG_DATA_HOME") {
            return Ok(PathBuf::from(xdg).join("fastsearch"));
        }
        if let Some(home) = env::var_os("HOME") {
            return Ok(PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("fastsearch"));
        }
    }
    Err(FastSearchError::new(
        ErrorKind::StateFailure,
        "cannot determine FastSearch catalog directory",
    ))
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), FastSearchError> {
    let parent = path.parent().ok_or_else(|| {
        FastSearchError::new(ErrorKind::InvalidContent, "storage file has no parent")
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| failure(ErrorKind::StateFailure, "create storage parent", error))?;
    let temporary = atomic_temporary_path(parent, path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| failure(ErrorKind::StateFailure, "create atomic storage file", error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| failure(ErrorKind::StateFailure, "write atomic storage file", error))?;
    drop(file);
    replace_file(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

fn atomic_temporary_path(parent: &Path, target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("fastsearch");
    let sequence = NEXT_ATOMIC_WRITE.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ))
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<(), FastSearchError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(failure(
            ErrorKind::StateFailure,
            "replace atomic storage file",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> Result<(), FastSearchError> {
    fs::rename(source, target).map_err(|error| {
        failure(
            ErrorKind::StateFailure,
            "replace atomic storage file",
            error,
        )
    })
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(windows)]
fn ordinary_windows_path(path: PathBuf) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

#[cfg(not(windows))]
fn ordinary_windows_path(path: PathBuf) -> PathBuf {
    path
}

fn failure(kind: ErrorKind, context: &str, error: std::io::Error) -> FastSearchError {
    FastSearchError::new(kind, format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "fastsearch-workspace-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn workspace_round_trip_preserves_two_contours_and_ignores_only_local() {
        let temp = Temp::new();
        let docs_a = temp.0.join("documentation");
        let docs_b = temp.0.join("specifications");
        let code_a = temp.0.join("backend");
        let code_b = temp.0.join("frontend");
        for root in [&docs_a, &docs_b, &code_a, &code_b] {
            fs::create_dir_all(root).unwrap();
        }
        let profile =
            WorkspaceProfile::from_roots(&temp.0, "Product", [docs_a, docs_b], [code_a, code_b])
                .unwrap();
        let created = WorkspaceStore::create(&temp.0, profile.clone()).unwrap();
        let reopened = WorkspaceStore::open(&temp.0).unwrap();

        assert_eq!(created.profile(), reopened.profile());
        assert_eq!(reopened.profile().contour_count(), 2);
        assert_eq!(reopened.profile().documentation_roots().len(), 2);
        assert_eq!(reopened.profile().code_roots().len(), 2);
        assert_eq!(
            fs::read_to_string(temp.0.join(".fastsearch").join(".gitignore")).unwrap(),
            "/local/\n"
        );
        assert!(temp.0.join(".fastsearch/knowledge/curated").is_dir());
        assert!(temp.0.join(".fastsearch/local/index/documents").is_dir());
    }

    #[test]
    fn workspace_persists_model_selection_and_portable_experiment_journal() {
        let temp = Temp::new();
        let profile = WorkspaceProfile::from_roots(&temp.0, "Models", [], [])
            .unwrap()
            .with_embedding_model(EmbeddingModelId::Qwen3Embedding06B);
        let mut store = WorkspaceStore::create(&temp.0, profile).unwrap();
        assert_eq!(
            WorkspaceStore::open(&temp.0)
                .unwrap()
                .profile()
                .embedding_model(),
            EmbeddingModelId::Qwen3Embedding06B
        );

        assert!(
            store
                .set_embedding_model(EmbeddingModelId::MultilingualE5Base)
                .unwrap()
        );
        let journal = store
            .record_embedding_experiment("где routing", 7, 42, "первый результат верный")
            .unwrap();
        let line = fs::read_to_string(journal).unwrap();
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["model"], "multilingual-e5-base");
        assert_eq!(value["query"], "где routing");
        assert_eq!(value["latency_ms"], 42);
        assert!(temp.0.join(".fastsearch/knowledge/experiments").is_dir());
    }

    #[test]
    fn discovery_finds_documentation_and_code_as_two_contours() {
        let temp = Temp::new();
        fs::create_dir_all(temp.0.join("documentation")).unwrap();
        fs::create_dir_all(temp.0.join("backend/src")).unwrap();
        fs::write(temp.0.join("documentation/guide.md"), "# Guide\n\ntext").unwrap();
        fs::write(temp.0.join("backend/Cargo.toml"), "[package]\nname='x'").unwrap();
        fs::write(temp.0.join("backend/src/lib.rs"), "pub fn x() {}").unwrap();

        let report = DiscoveryReport::scan(&temp.0).unwrap();

        assert_eq!(
            report.documentation_roots(),
            [temp.0.join("documentation").canonicalize().unwrap()]
        );
        assert_eq!(
            report.code_roots(),
            [temp.0.join("backend").canonicalize().unwrap()]
        );
    }

    #[test]
    fn catalog_resolves_the_deepest_registered_workspace() {
        let temp = Temp::new();
        let nested = temp.0.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let outer = WorkspaceProfile::from_roots(&temp.0, "Outer", [], []).unwrap();
        let inner = WorkspaceProfile::from_roots(&nested, "Inner", [], []).unwrap();
        let mut catalog = WorkspaceCatalog::default();
        catalog.register(&temp.0, &outer).unwrap();
        catalog.register(&nested, &inner).unwrap();

        assert_eq!(
            catalog.resolve_path(&nested).map(CatalogEntry::name),
            Some("Inner")
        );
    }

    #[test]
    fn source_roots_must_remain_inside_workspace() {
        let workspace = Temp::new();
        let outside = Temp::new();
        let error = WorkspaceProfile::from_roots(&workspace.0, "Unsafe", [outside.0.clone()], [])
            .unwrap_err();

        assert_eq!(error.kind(), &ErrorKind::InvalidContent);
    }

    #[test]
    fn atomic_write_uses_a_distinct_temporary_path_for_each_call() {
        let temp = Temp::new();
        let target = temp.0.join("workspace.toml");

        let first = atomic_temporary_path(&temp.0, &target);
        let second = atomic_temporary_path(&temp.0, &target);

        assert_ne!(first, second);
        assert!(
            first
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".tmp")
        );
        assert!(
            second
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".tmp")
        );
    }
}
