use super::{CanonicalRecord, ErrorKind, FastSearchError, IndexFreshness, StableId};

/// Режим представления результата; ranking реализуется adapters позднее.
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

/// Один результат retrieval без навязывания ranking algorithm.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    record: CanonicalRecord,
    channel: RetrievalChannel,
    score: f64,
}

impl SearchHit {
    #[must_use]
    pub const fn new(record: CanonicalRecord, channel: RetrievalChannel, score: f64) -> Self {
        Self {
            record,
            channel,
            score,
        }
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
