//! Application coordination and production compositions for FastSearch.

mod cli;
mod comparison;
mod compatibility;
mod console;
pub mod fusion;
mod model_cache;
mod production;
mod workspace;

pub use cli::{CliError, OutputFormat, execute_cli, execute_cli_formatted};
pub use comparison::{
    ComparisonCoordinator, ComparisonModelResult, ComparisonReadiness, ComparisonRun,
    ComparisonUpdateOutcome,
};
pub use compatibility::RealRuntime;
pub use console::{help_text, run_interactive, version_text};
pub use model_cache::{
    E5ModelAvailability, EmbeddingModelAvailability, EmbeddingModelCacheStatus,
    EmbeddingModelDescriptor, MODEL_CATALOG, ModelProvisionProgress, ModelRuntimeCapabilities,
    embedding_model_cache_status, ensure_e5_model, ensure_embedding_model,
    ensure_embedding_model_with_progress, model_descriptor, model_runtime_capabilities,
};
pub use production::{ModelPartitionMetrics, ProductionConfig, ProductionRuntime};
pub use workspace::{
    CatalogEntry, DiscoveryReport, SourceRoot, WorkspaceCatalog, WorkspaceProfile, WorkspaceStore,
};
