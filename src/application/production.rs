//! The single full production composition for semantic and code navigation.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    adapters::{
        lexical::TantivyLexical,
        maps::{CodeMapRelated, CodeMapSource},
        source::FilesystemSource,
        state::SqliteStateStore,
        symbols::SymbolSource,
        vector::LocalE5Vector,
    },
    application::fusion::{ChannelCandidates, FusionCoordinator},
    domain::{
        BackendKind, CanonicalRecord, Capability, CapabilityStatus, ErrorKind, FastSearchError,
        IndexFreshness, LifecycleStatus, LogicalRootId, RelatedQuery, RetrievalChannel, SearchHit,
        SearchQuery, SearchResponse, StableId,
    },
    ports::{
        AgentSurface, CodeMapPort, LexicalRetrieval, SourcePort, StateChange, StateStore,
        SymbolPort, VectorRetrieval,
    },
};

const E5_IDENTITY: &str = "multilingual-e5-small@614241f";

/// Replaceable machine paths for the one full production composition.
#[derive(Clone, Debug)]
pub struct ProductionConfig {
    document_root: PathBuf,
    code_root: PathBuf,
    service_root: PathBuf,
    e5_root: Option<PathBuf>,
}

impl ProductionConfig {
    #[must_use]
    pub fn new(
        document_root: impl Into<PathBuf>,
        code_root: impl Into<PathBuf>,
        service_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            document_root: document_root.into(),
            code_root: code_root.into(),
            service_root: service_root.into(),
            e5_root: None,
        }
    }

    #[must_use]
    pub fn with_e5_root(mut self, e5_root: impl Into<PathBuf>) -> Self {
        self.e5_root = Some(e5_root.into());
        self
    }
}

/// Filesystem + SQLite + Tantivy + local E5 + maps + symbols + E1 fusion.
pub struct ProductionRuntime {
    service_root: PathBuf,
    documents: FilesystemSource,
    maps: CodeMapSource,
    symbols: SymbolSource,
    state: SqliteStateStore,
    lexical: TantivyLexical,
    vector: LocalE5Vector,
    vector_configured: bool,
}

impl std::fmt::Debug for ProductionRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductionRuntime")
            .finish_non_exhaustive()
    }
}

impl ProductionRuntime {
    pub fn open(config: ProductionConfig) -> Result<Self, FastSearchError> {
        let document_root = canonical_directory(&config.document_root, "document root")?;
        let code_root = canonical_directory(&config.code_root, "code root")?;
        fs::create_dir_all(&config.service_root)
            .map_err(|error| failure(ErrorKind::StateFailure, "create service root", error))?;
        let service_root = canonical_directory(&config.service_root, "service root")?;
        validate_service_containment(&document_root, &code_root, &service_root)?;
        let vector_configured = config.e5_root.is_some();
        let model_root = config
            .e5_root
            .unwrap_or_else(|| service_root.join("unconfigured-e5"));

        Ok(Self {
            service_root: service_root.clone(),
            documents: FilesystemSource::new(document_root.clone()),
            maps: CodeMapSource::new(document_root),
            symbols: SymbolSource::new(LogicalRootId::parse("code-fastsearch")?, code_root),
            state: SqliteStateStore::open(service_root.join("state.sqlite"))?,
            lexical: TantivyLexical::open(service_root.join("lexical"))?,
            vector: LocalE5Vector::open(model_root, E5_IDENTITY),
            vector_configured,
        })
    }

    pub fn index(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.project(false)
    }

    pub fn rebuild(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.project(true)
    }

    /// Creates an exact run-owned directory used by acceptance jobs and batch callers.
    pub fn record_run_marker(&self, marker: &str) -> Result<PathBuf, FastSearchError> {
        validate_marker(marker)?;
        let run = self.service_root.join("runs").join(marker);
        fs::create_dir_all(&run)
            .map_err(|error| failure(ErrorKind::StateFailure, "create run directory", error))?;
        fs::write(run.join("owner.marker"), marker.as_bytes())
            .map_err(|error| failure(ErrorKind::StateFailure, "write run marker", error))?;
        Ok(run)
    }

    /// Removes only a directory whose marker content exactly matches the requested run.
    pub fn cleanup_run(&self, marker: &str) -> Result<bool, FastSearchError> {
        validate_marker(marker)?;
        let run = self.service_root.join("runs").join(marker);
        let marker_path = run.join("owner.marker");
        if !marker_path.exists() {
            return Ok(false);
        }
        let observed = fs::read_to_string(&marker_path)
            .map_err(|error| failure(ErrorKind::StateFailure, "read run marker", error))?;
        if observed != marker {
            return Err(FastSearchError::new(
                ErrorKind::StateFailure,
                "run cleanup marker does not match the exact requested owner",
            ));
        }
        fs::remove_dir_all(&run).map_err(|error| {
            failure(ErrorKind::StateFailure, "remove exact run directory", error)
        })?;
        Ok(true)
    }

    fn project(&mut self, rebuild: bool) -> Result<LifecycleStatus, FastSearchError> {
        let mut snapshots = self.documents.snapshot()?;
        snapshots.extend(self.maps.snapshot()?);
        snapshots.extend(self.symbols.snapshot()?);
        let records = snapshots
            .iter()
            .flat_map(|snapshot| snapshot.records().iter().cloned())
            .collect::<Vec<_>>();
        let changes = self.state.reconcile_snapshots(&snapshots)?;
        let lexical = if rebuild {
            self.lexical
                .rebuild(&records, changes.durable_generation())?
        } else {
            let current = self.lexical.lifecycle_status();
            let unchanged = changes
                .changes()
                .iter()
                .all(|change| *change == StateChange::Unchanged);
            if unchanged
                && current.freshness() == IndexFreshness::Current
                && current.projection_generation() == Some(changes.durable_generation())
            {
                current
            } else {
                self.lexical
                    .apply_projection(&records, changes.durable_generation())?
            }
        };
        if self.vector_configured {
            let _vector_result = if rebuild {
                self.vector.rebuild(&records, changes.durable_generation())
            } else {
                self.vector.apply(&records, changes.durable_generation())
            };
        }
        Ok(lexical)
    }

    fn combined_records(&self) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        let mut records = self.maps.records()?;
        records.extend(self.symbols.records()?);
        Ok(records)
    }

    fn lifecycle_status(&self) -> LifecycleStatus {
        let state = self.state.lifecycle_status();
        let lexical = self.lexical.lifecycle_status();
        if state.freshness() == IndexFreshness::Degraded {
            return state;
        }
        if lexical.freshness() == IndexFreshness::Current
            && lexical.projection_generation() == Some(state.state_generation())
        {
            LifecycleStatus::new(
                IndexFreshness::Current,
                state.state_generation(),
                lexical.projection_generation(),
                lexical.detail(),
            )
        } else {
            LifecycleStatus::new(
                if lexical.freshness() == IndexFreshness::Degraded {
                    IndexFreshness::Degraded
                } else {
                    IndexFreshness::Stale
                },
                state.state_generation(),
                lexical.projection_generation(),
                lexical.detail(),
            )
        }
    }
}

impl AgentSurface for ProductionRuntime {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        let lexical = self.lexical.search(query)?;
        let mut grouped = BTreeMap::<u8, (RetrievalChannel, Vec<SearchHit>)>::new();
        for hit in lexical.hits() {
            let channel = hit.channel();
            grouped
                .entry(channel_order(channel))
                .or_insert_with(|| (channel, Vec::new()))
                .1
                .push(hit.clone());
        }
        let mut candidates = grouped
            .into_values()
            .map(|(channel, hits)| ChannelCandidates::new(channel, hits, lexical.freshness()))
            .collect::<Result<Vec<_>, _>>()?;

        if self.vector_configured
            && let Ok(vector) = self.vector.search(query)
        {
            candidates.push(ChannelCandidates::new(
                RetrievalChannel::Vector,
                vector.hits().to_vec(),
                vector.freshness(),
            )?);
        }
        let needle = query.text().to_lowercase();
        let maps = self
            .maps
            .records()?
            .into_iter()
            .filter(|record| {
                record.title().to_lowercase().contains(&needle)
                    || record.searchable_content().to_lowercase().contains(&needle)
            })
            .map(|record| SearchHit::new(record, RetrievalChannel::CodeMap, 0.0))
            .collect::<Vec<_>>();
        candidates.push(ChannelCandidates::new(
            RetrievalChannel::CodeMap,
            maps,
            IndexFreshness::Current,
        )?);
        let symbols = self
            .symbols
            .find_symbols(query)?
            .into_iter()
            .map(|record| SearchHit::new(record, RetrievalChannel::Symbol, 0.0))
            .collect::<Vec<_>>();
        candidates.push(ChannelCandidates::new(
            RetrievalChannel::Symbol,
            symbols,
            IndexFreshness::Current,
        )?);
        Ok(FusionCoordinator::fuse(query, candidates, &self.status()))
    }

    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
        self.state.get(id)
    }

    fn related(&self, query: &RelatedQuery) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        CodeMapRelated::new(self.combined_records()?)?.related_maps(query)
    }

    fn status(&self) -> Vec<CapabilityStatus> {
        vec![
            CapabilityStatus::available(Capability::Source, BackendKind::Real),
            CapabilityStatus::available(Capability::State, BackendKind::Real),
            CapabilityStatus::available(Capability::LexicalRetrieval, BackendKind::Real),
            if self.vector_configured {
                self.vector.capability_status()
            } else {
                CapabilityStatus::unavailable(
                    Capability::VectorRetrieval,
                    "local E5 root is not configured",
                )
            },
            CapabilityStatus::available(Capability::CodeMaps, BackendKind::Real),
            CapabilityStatus::available(Capability::Symbols, BackendKind::Real),
        ]
    }

    fn index_status(&self) -> LifecycleStatus {
        self.lifecycle_status()
    }
}

fn channel_order(channel: RetrievalChannel) -> u8 {
    match channel {
        RetrievalChannel::Exact => 0,
        RetrievalChannel::Lexical => 1,
        RetrievalChannel::Vector => 2,
        RetrievalChannel::CodeMap => 3,
        RetrievalChannel::Symbol => 4,
    }
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

fn validate_service_containment(
    documents: &Path,
    code: &Path,
    service: &Path,
) -> Result<(), FastSearchError> {
    let overlaps = service.starts_with(documents)
        || service.starts_with(code)
        || documents.starts_with(service)
        || code.starts_with(service);
    let reserved = service
        .components()
        .any(|component| component.as_os_str() == ".cfknowledge");
    if overlaps && !reserved {
        return Err(FastSearchError::new(
            ErrorKind::InvalidContent,
            "service root may overlap a source root only inside the reserved .cfknowledge zone",
        ));
    }
    Ok(())
}

fn validate_marker(marker: &str) -> Result<(), FastSearchError> {
    if marker.is_empty()
        || marker.len() > 128
        || !marker
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(FastSearchError::new(
            ErrorKind::InvalidIdentifier,
            "run marker must be 1..128 ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

fn failure(kind: ErrorKind, context: &str, error: std::io::Error) -> FastSearchError {
    FastSearchError::new(kind, format!("{context}: {error}"))
}
