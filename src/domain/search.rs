use super::{CanonicalRecord, ErrorKind, FastSearchError, StableId};

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
}

impl SearchResponse {
    #[must_use]
    pub fn new(hits: Vec<SearchHit>) -> Self {
        Self { hits }
    }
    #[must_use]
    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
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
