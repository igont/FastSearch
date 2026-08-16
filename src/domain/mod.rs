//! Канонические типы, не зависящие от конкретных adapters.

mod embedding_model;
mod error;
mod execution;
mod record;
mod search;
mod status;

pub use embedding_model::EmbeddingModelId;
pub use error::{ErrorKind, FastSearchError};
pub use execution::{DeviceCapabilityStatus, ExecutionDevice};
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
