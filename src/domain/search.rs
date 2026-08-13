use super::{CanonicalRecord, ErrorKind, FastSearchError, IndexFreshness, StableId};

/// Режим ranking общей retrieval/fusion composition.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SearchMode {
    #[default]
    Balanced,
    Current,
    Design,
}

/// Проверенный запрос к retrieval boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchQuery {
    text: String,
    mode: SearchMode,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>, mode: SearchMode) -> Result<Self, FastSearchError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidQuery,
                "search query must not be blank",
            ));
        }

        Ok(Self { text, mode })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
    #[must_use]
    pub const fn mode(&self) -> SearchMode {
        self.mode
    }
}

/// Канал, по которому result был найден.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetrievalChannel {
    Exact,
    Lexical,
    Vector,
    CodeMap,
    Symbol,
}

/// Полная идентичность модели, привязанная к проверенному набору артефактов.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelIdentity {
    model: String,
    upstream_revision: String,
    artifact_manifest_sha256: String,
}

impl ModelIdentity {
    pub fn new(
        model: impl Into<String>,
        upstream_revision: impl Into<String>,
        artifact_manifest_sha256: impl Into<String>,
    ) -> Result<Self, FastSearchError> {
        let model = model.into();
        let upstream_revision = upstream_revision.into();
        let artifact_manifest_sha256 = artifact_manifest_sha256.into();
        if model.trim().is_empty() || upstream_revision.trim().is_empty() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidIdentifier,
                "model identity requires model name and upstream revision",
            ));
        }
        if artifact_manifest_sha256.len() != 64
            || !artifact_manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "model identity requires a full SHA-256 artifact-manifest digest",
            ));
        }
        Ok(Self {
            model,
            upstream_revision,
            artifact_manifest_sha256: artifact_manifest_sha256.to_ascii_lowercase(),
        })
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub fn upstream_revision(&self) -> &str {
        &self.upstream_revision
    }

    #[must_use]
    pub fn artifact_manifest_sha256(&self) -> &str {
        &self.artifact_manifest_sha256
    }
}

/// Происхождение rebuildable projection относительно durable state authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionProvenance {
    model_identity: ModelIdentity,
    authoritative_state_generation: u64,
    derived_projection_generation: u64,
}

impl ProjectionProvenance {
    #[must_use]
    pub const fn new(
        model_identity: ModelIdentity,
        authoritative_state_generation: u64,
        derived_projection_generation: u64,
    ) -> Self {
        Self {
            model_identity,
            authoritative_state_generation,
            derived_projection_generation,
        }
    }

    #[must_use]
    pub const fn model_identity(&self) -> &ModelIdentity {
        &self.model_identity
    }

    #[must_use]
    pub const fn authoritative_state_generation(&self) -> u64 {
        self.authoritative_state_generation
    }

    #[must_use]
    pub const fn derived_projection_generation(&self) -> u64 {
        self.derived_projection_generation
    }
}

/// Один результат retrieval без навязывания ranking algorithm.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    record: CanonicalRecord,
    channel: RetrievalChannel,
    score: f64,
    projection_provenance: Option<ProjectionProvenance>,
}

impl SearchHit {
    #[must_use]
    pub const fn new(record: CanonicalRecord, channel: RetrievalChannel, score: f64) -> Self {
        Self {
            record,
            channel,
            score,
            projection_provenance: None,
        }
    }

    #[must_use]
    pub fn with_projection_provenance(mut self, provenance: ProjectionProvenance) -> Self {
        self.projection_provenance = Some(provenance);
        self
    }
    #[must_use]
    pub const fn record(&self) -> &CanonicalRecord {
        &self.record
    }
    #[must_use]
    pub const fn channel(&self) -> RetrievalChannel {
        self.channel
    }
    #[must_use]
    pub const fn score(&self) -> f64 {
        self.score
    }

    #[must_use]
    pub const fn projection_provenance(&self) -> Option<&ProjectionProvenance> {
        self.projection_provenance.as_ref()
    }
}

/// Ответ search boundary; exact lookup и ranking могут возвращать несколько hits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchResponse {
    hits: Vec<SearchHit>,
    freshness: IndexFreshness,
}

impl SearchResponse {
    #[must_use]
    pub fn new(hits: Vec<SearchHit>) -> Self {
        Self {
            hits,
            freshness: IndexFreshness::NotConfigured,
        }
    }
    #[must_use]
    pub fn with_freshness(hits: Vec<SearchHit>, freshness: IndexFreshness) -> Self {
        Self { hits, freshness }
    }
    #[must_use]
    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }
    pub fn projection_provenances(&self) -> impl Iterator<Item = &ProjectionProvenance> {
        self.hits
            .iter()
            .filter_map(SearchHit::projection_provenance)
    }
    #[must_use]
    pub const fn freshness(&self) -> IndexFreshness {
        self.freshness
    }

    /// Deterministically fuses independently produced hits without hiding provenance.
    #[must_use]
    pub fn fuse(
        mut hits: Vec<SearchHit>,
        capability_statuses: Vec<super::CapabilityStatus>,
    ) -> Self {
        hits.sort_by(|left, right| {
            right
                .score()
                .total_cmp(&left.score())
                .then_with(|| channel_order(left.channel()).cmp(&channel_order(right.channel())))
                .then_with(|| left.record().id().cmp(right.record().id()))
        });
        let freshness = if capability_statuses
            .iter()
            .any(|status| matches!(status.state(), super::CapabilityState::Degraded { .. }))
        {
            IndexFreshness::Degraded
        } else if capability_statuses.iter().any(|status| {
            matches!(
                status.state(),
                super::CapabilityState::Stale { .. } | super::CapabilityState::Unavailable { .. }
            )
        }) {
            IndexFreshness::Stale
        } else {
            IndexFreshness::Current
        };
        Self { hits, freshness }
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

/// Идентификатор, для которого нужно вернуть ближайшие явные связи.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelatedQuery {
    id: StableId,
}

impl RelatedQuery {
    #[must_use]
    pub const fn new(id: StableId) -> Self {
        Self { id }
    }
    #[must_use]
    pub const fn id(&self) -> &StableId {
        &self.id
    }
}
