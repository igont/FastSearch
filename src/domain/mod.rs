//! Канонические типы, не зависящие от конкретных adapters.

mod error;
mod record;
mod search;
mod status;

pub use error::{ErrorKind, FastSearchError};
pub use record::{
    CanonicalRecord, ContentHash, RecordKind, SourceLocator, SourceSelector, StableId,
};
pub use search::{
    RelatedQuery, RetrievalChannel, SearchHit, SearchMode, SearchQuery, SearchResponse,
};
pub use status::{BackendKind, Capability, CapabilityState, CapabilityStatus};
