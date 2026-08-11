use std::fmt;

use super::{Capability, RecordKind};

/// Категория ожидаемой ошибки на границе core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    InvalidIdentifier,
    InvalidLocator,
    InvalidContent,
    InvalidQuery,
    UnsupportedSource { kind: RecordKind },
    CapabilityUnavailable { capability: Capability },
    NotFound,
    StateFailure,
    SourceFailure,
    ProjectionFailure,
    DuplicateStableId,
}

/// Структурированная ошибка FastSearch без привязки к транспортному формату.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FastSearchError {
    kind: ErrorKind,
    message: String,
}

impl FastSearchError {
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn unsupported_source(kind: RecordKind) -> Self {
        Self::new(
            ErrorKind::UnsupportedSource { kind },
            "source kind is not supported by this capability",
        )
    }

    #[must_use]
    pub const fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for FastSearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for FastSearchError {}
