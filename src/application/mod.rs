//! Application coordination and production compositions for FastSearch.

pub(crate) mod chunking;
mod cli;
mod comparison;
mod compatibility;
mod console;
pub mod fusion;
mod inspection;
mod model_cache;
mod production;
mod public_search;
mod retrieval_projection;
mod workspace;

pub use cli::{CliError, OutputFormat, execute_cli, execute_cli_formatted};
pub use comparison::{
    ComparisonCoordinator, ComparisonModelResult, ComparisonReadiness, ComparisonRun,
    ComparisonUpdateOutcome,
};
pub use compatibility::RealRuntime;
pub use console::{help_text, run_interactive, version_text};
pub use inspection::InspectionReport;
pub use model_cache::{
    E5ModelAvailability, EmbeddingModelAvailability, EmbeddingModelCacheStatus,
    EmbeddingModelDescriptor, MODEL_CATALOG, ModelProvisionProgress, ModelRuntimeCapabilities,
    embedding_model_cache_status, ensure_e5_model, ensure_embedding_model,
    ensure_embedding_model_with_progress, model_descriptor, model_runtime_capabilities,
};
pub use production::{ModelPartitionMetrics, ProductionConfig, ProductionRuntime};
pub use public_search::{
    DocumentStatus, MAX_PUBLIC_QUERY_CHARS, MAX_PUBLIC_RESULTS, MAX_SERIALIZED_RESPONSE_BYTES,
    ProjectScope, PublicSearchError, PublicSearchErrorCode, PublicSearchRequest,
    PublicSearchResponse, PublicSearchResult,
};
pub use workspace::{
    CatalogEntry, DiscoveryReport, SourceRoot, WorkspaceCatalog, WorkspaceProfile, WorkspaceStore,
};
