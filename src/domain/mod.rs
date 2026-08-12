//! Канонические типы, не зависящие от конкретных adapters.

mod error;
mod record;
mod search;
mod status;

pub use error::{ErrorKind, FastSearchError};
pub use record::{
    CanonicalRecord, ContentHash, FileHash, LogicalRootId, RecordKind, RootedSourceLocator,
    SourceAdmission, SourceLocator, SourceSelector, SourceSnapshot, StableId,
};
pub use search::{
    ModelIdentity, ProjectionProvenance, RelatedQuery, RetrievalChannel, SearchHit, SearchMode,
    SearchQuery, SearchResponse,
};
pub use status::{
    BackendKind, Capability, CapabilityState, CapabilityStatus, IndexFreshness, LifecycleStatus,
};
