//! Local-only multilingual-E5 derived vector projection.
//!
//! The adapter receives canonical records from the SQLite authority.  It never
//! writes them back, and a model/content identity mismatch makes its projection
//! stale until the caller supplies the authoritative record set again.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use sha2::{Digest, Sha256};

use crate::{
    domain::{
        BackendKind, CanonicalRecord, Capability, CapabilityStatus, ErrorKind, FastSearchError,
        IndexFreshness, LifecycleStatus, ModelIdentity, ProjectionProvenance, RetrievalChannel,
        SearchHit, SearchQuery, SearchResponse,
    },
    ports::VectorRetrieval,
};

#[derive(Clone)]
struct ProjectedRecord {
    record: CanonicalRecord,
    content_hash: String,
    vector: Vec<f32>,
}

struct ProjectionState {
    model_root: PathBuf,
    model_identity: String,
    model_manifest: Option<String>,
    records: BTreeMap<String, ProjectedRecord>,
    state_generation: u64,
    projection_generation: Option<u64>,
    freshness: IndexFreshness,
    detail: String,
}

/// Rebuildable local vector projection using only explicitly supplied model files.
pub struct LocalE5Vector {
    state: Mutex<ProjectionState>,
}

/// Provenance of the currently usable derived projection.  Consumers must not
/// confuse it with the authoritative SQLite generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorProjectionProvenance {
    model_identity: String,
    state_generation: u64,
    projection_generation: Option<u64>,
}

impl VectorProjectionProvenance {
    #[must_use]
    pub fn model_identity(&self) -> &str {
        &self.model_identity
    }
    #[must_use]
    pub const fn state_generation(&self) -> u64 {
        self.state_generation
    }
    #[must_use]
    pub const fn projection_generation(&self) -> Option<u64> {
        self.projection_generation
    }
}

impl LocalE5Vector {
    #[must_use]
    pub fn open(model_root: impl Into<PathBuf>, model_identity: impl Into<String>) -> Self {
        Self {
            state: Mutex::new(ProjectionState {
                model_root: model_root.into(),
                model_identity: model_identity.into(),
                model_manifest: None,
                records: BTreeMap::new(),
                state_generation: 0,
                projection_generation: None,
                freshness: IndexFreshness::NotConfigured,
                detail: "vector projection is absent".to_owned(),
            }),
        }
    }

    /// Applies the complete authoritative record set. Removed IDs disappear from
    /// the derived projection because the map is replaced atomically after E5 succeeds.
    pub fn apply(
        &self,
        records: &[CanonicalRecord],
        state_generation: u64,
    ) -> Result<LifecycleStatus, FastSearchError> {
        let (root, identity) = {
            let state = self.lock()?;
            (state.model_root.clone(), state.model_identity.clone())
        };
        let manifest =
            model_manifest(&root).map_err(|error| self.provider_failed(state_generation, error))?;
        let unchanged = {
            let state = self.lock()?;
            state.freshness == IndexFreshness::Current
                && state.model_manifest.as_deref() == Some(&manifest)
                && state.records.len() == records.len()
                && records.iter().all(|record| {
                    state
                        .records
                        .get(record.id().as_str())
                        .is_some_and(|projected| {
                            projected.content_hash == record.content_hash().as_str()
                        })
                })
        };
        if unchanged {
            let mut state = self.lock()?;
            state.state_generation = state_generation;
            state.projection_generation = Some(state_generation);
            return Ok(status(&state));
        }
        let vectors =
            embed(&root, records).map_err(|error| self.provider_failed(state_generation, error))?;
        let mut next = BTreeMap::new();
        for (record, vector) in records.iter().cloned().zip(vectors) {
            if next.contains_key(record.id().as_str()) {
                return Err(FastSearchError::new(
                    ErrorKind::ProjectionFailure,
                    "vector projection input contains a duplicate stable identifier",
                ));
            }
            next.insert(
                record.id().as_str().to_owned(),
                ProjectedRecord {
                    content_hash: record.content_hash().as_str().to_owned(),
                    record,
                    vector,
                },
            );
        }
        let mut state = self.lock()?;
        // Reconfiguration during embedding is a causal stale result, never Current.
        if state.model_identity != identity
            || state.model_root != root
            || state
                .model_manifest
                .as_deref()
                .is_some_and(|current| current != manifest)
        {
            state.freshness = IndexFreshness::Stale;
            state.detail = "model identity changed while projection was building".to_owned();
            return Ok(status(&state));
        }
        state.records = next;
        state.model_manifest = Some(manifest);
        state.state_generation = state_generation;
        state.projection_generation = Some(state_generation);
        state.freshness = IndexFreshness::Current;
        state.detail = format!("local E5 projection is current for {identity}");
        Ok(status(&state))
    }

    pub fn rebuild(
        &self,
        records: &[CanonicalRecord],
        state_generation: u64,
    ) -> Result<LifecycleStatus, FastSearchError> {
        self.apply(records, state_generation)
    }

    /// A complete model identity change invalidates all derived vectors.
    pub fn reconfigure(
        &self,
        model_root: impl Into<PathBuf>,
        model_identity: impl Into<String>,
    ) -> Result<(), FastSearchError> {
        let mut state = self.lock()?;
        state.model_root = model_root.into();
        state.model_identity = model_identity.into();
        state.model_manifest = None;
        state.records.clear();
        state.projection_generation = None;
        state.freshness = IndexFreshness::Stale;
        state.detail = "model identity changed; vector projection must rebuild".to_owned();
        Ok(())
    }

    #[must_use]
    pub fn lifecycle_status(&self) -> LifecycleStatus {
        match self.lock() {
            Ok(state) => status(&state),
            Err(_) => LifecycleStatus::new(
                IndexFreshness::Degraded,
                0,
                None,
                "vector state lock failed",
            ),
        }
    }

    #[must_use]
    pub fn capability_status(&self) -> CapabilityStatus {
        match self.lifecycle_status().freshness() {
            IndexFreshness::Current => {
                CapabilityStatus::available(Capability::VectorRetrieval, BackendKind::Real)
            }
            IndexFreshness::Degraded => CapabilityStatus::degraded(
                Capability::VectorRetrieval,
                self.lifecycle_status().detail(),
            ),
            _ => CapabilityStatus::unavailable(
                Capability::VectorRetrieval,
                self.lifecycle_status().detail(),
            ),
        }
    }

    #[must_use]
    pub fn provenance(&self) -> VectorProjectionProvenance {
        match self.lock() {
            Ok(state) => VectorProjectionProvenance {
                model_identity: state.model_identity.clone(),
                state_generation: state.state_generation,
                projection_generation: state.projection_generation,
            },
            Err(_) => VectorProjectionProvenance {
                model_identity: "unavailable".to_owned(),
                state_generation: 0,
                projection_generation: None,
            },
        }
    }

    fn provider_failed(&self, generation: u64, error: FastSearchError) -> FastSearchError {
        if let Ok(mut state) = self.lock() {
            state.state_generation = generation;
            state.projection_generation = None;
            state.freshness = IndexFreshness::Degraded;
            state.detail = error.message().to_owned();
        }
        error
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ProjectionState>, FastSearchError> {
        self.state.lock().map_err(|_| {
            FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "vector state lock is poisoned",
            )
        })
    }
}

impl VectorRetrieval for LocalE5Vector {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        let (
            root,
            entries,
            freshness,
            generation,
            projection_generation,
            declared_identity,
            manifest,
        ) = {
            let state = self.lock()?;
            (
                state.model_root.clone(),
                state.records.values().cloned().collect::<Vec<_>>(),
                state.freshness,
                state.state_generation,
                state.projection_generation,
                state.model_identity.clone(),
                state.model_manifest.clone(),
            )
        };
        if freshness != IndexFreshness::Current {
            return Ok(SearchResponse::with_freshness(Vec::new(), freshness));
        }
        let query_vector = embed_texts(&root, &[query.text().to_owned()])
            .map_err(|error| self.provider_failed(generation, error))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                FastSearchError::new(ErrorKind::ProjectionFailure, "E5 returned no query vector")
            })?;
        let provenance = projection_generation
            .zip(manifest)
            .map(|(projection, fingerprint)| {
                projection_provenance(&declared_identity, fingerprint, generation, projection)
            })
            .transpose()?;
        let mut hits = entries
            .into_iter()
            .map(|entry| {
                let hit = SearchHit::new(
                    entry.record,
                    RetrievalChannel::Vector,
                    f64::from(cosine(&query_vector, &entry.vector)),
                );
                match &provenance {
                    Some(value) => hit.with_projection_provenance(value.clone()),
                    None => hit,
                }
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .score()
                .total_cmp(&left.score())
                .then_with(|| left.record().id().cmp(right.record().id()))
        });
        Ok(SearchResponse::with_freshness(
            hits,
            IndexFreshness::Current,
        ))
    }
}

fn status(state: &ProjectionState) -> LifecycleStatus {
    LifecycleStatus::new(
        state.freshness,
        state.state_generation,
        state.projection_generation,
        &state.detail,
    )
}

fn projection_provenance(
    declared: &str,
    fingerprint: String,
    state_generation: u64,
    projection_generation: u64,
) -> Result<ProjectionProvenance, FastSearchError> {
    let (model, revision) = declared.rsplit_once('@').ok_or_else(|| {
        FastSearchError::new(
            ErrorKind::InvalidIdentifier,
            "local E5 model identity must be model@upstream-revision",
        )
    })?;
    let identity = ModelIdentity::new(model, revision, fingerprint)?;
    Ok(ProjectionProvenance::new(
        identity,
        state_generation,
        projection_generation,
    ))
}

fn embed(root: &Path, records: &[CanonicalRecord]) -> Result<Vec<Vec<f32>>, FastSearchError> {
    let texts = records
        .iter()
        .map(|record| format!("{}\n{}", record.title(), record.searchable_content()))
        .collect::<Vec<_>>();
    embed_texts(root, &texts)
}

fn embed_texts(root: &Path, texts: &[String]) -> Result<Vec<Vec<f32>>, FastSearchError> {
    let model = local_model(root)?;
    let mut runtime =
        TextEmbedding::try_new_from_user_defined(model, InitOptionsUserDefined::default())
            .map_err(provider_error)?;
    let vectors = runtime.embed(texts, Some(1)).map_err(provider_error)?;
    if vectors.len() != texts.len()
        || vectors
            .iter()
            .any(|vector| vector.is_empty() || vector.iter().any(|value| !value.is_finite()))
    {
        return Err(FastSearchError::new(
            ErrorKind::ProjectionFailure,
            "local E5 returned an invalid vector",
        ));
    }
    Ok(vectors.into_iter().map(normalize).collect())
}

fn local_model(root: &Path) -> Result<UserDefinedEmbeddingModel, FastSearchError> {
    let onnx = root.join("onnx");
    let required = [
        "model.onnx",
        "tokenizer.json",
        "config.json",
        "special_tokens_map.json",
        "tokenizer_config.json",
    ];
    if required.iter().any(|name| !onnx.join(name).is_file()) {
        return Err(FastSearchError::new(
            ErrorKind::CapabilityUnavailable {
                capability: Capability::VectorRetrieval,
            },
            "B2_NO_LOCAL_E5_PROVIDER",
        ));
    }
    let read = |name: &str| {
        fs::read(onnx.join(name)).map_err(|_| {
            FastSearchError::new(
                ErrorKind::CapabilityUnavailable {
                    capability: Capability::VectorRetrieval,
                },
                "B2_NO_LOCAL_E5_PROVIDER",
            )
        })
    };
    Ok(UserDefinedEmbeddingModel::new(
        read("model.onnx")?,
        TokenizerFiles {
            tokenizer_file: read("tokenizer.json")?,
            config_file: read("config.json")?,
            special_tokens_map_file: read("special_tokens_map.json")?,
            tokenizer_config_file: read("tokenizer_config.json")?,
        },
    )
    .with_pooling(Pooling::Mean))
}

/// B1 accepted complete E5 cache set: canonical locator/bytes/full-SHA256 root.
const B1_E5_MANIFEST_ROOT: &str =
    "63A0FA9AEC56D0A3F5080D82956111F4BBEE57BF0A3637371CF16E451B194D0E";

fn model_manifest(root: &Path) -> Result<String, FastSearchError> {
    fn collect(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), FastSearchError> {
        for entry in fs::read_dir(directory).map_err(provider_error)? {
            let entry = entry.map_err(provider_error)?;
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                collect(&path, files)?;
            } else if path.is_file() {
                files.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    collect(root, &mut files)?;
    files.sort();
    let mut lines = Vec::new();
    for path in files {
        let bytes = fs::read(&path).map_err(provider_error)?;
        let hash = format!("{:X}", Sha256::digest(&bytes));
        let locator = path
            .strip_prefix(root)
            .map_err(provider_error)?
            .to_string_lossy()
            .replace('/', "\\");
        lines.push(format!("{locator}|{}|{hash}", bytes.len()));
    }
    let root_hash = format!(
        "{:X}",
        Sha256::digest(format!("{}\n", lines.join("\n")).as_bytes())
    );
    if root_hash != B1_E5_MANIFEST_ROOT {
        return Err(FastSearchError::new(
            ErrorKind::CapabilityUnavailable {
                capability: Capability::VectorRetrieval,
            },
            "B2_LOCAL_E5_MANIFEST_MISMATCH",
        ));
    }
    Ok(root_hash)
}

fn provider_error(error: impl std::fmt::Display) -> FastSearchError {
    FastSearchError::new(
        ErrorKind::CapabilityUnavailable {
            capability: Capability::VectorRetrieval,
        },
        format!("B2_LOCAL_E5_PROVIDER_FAILED: {error}"),
    )
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm.is_finite() && norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}
