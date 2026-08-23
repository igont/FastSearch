//! Application coordination and production compositions for FastSearch.

pub(crate) mod chunking;
mod cli;
mod comparison;
mod compatibility;
mod console;
pub mod fusion;
mod inspection;
mod model_cache;
mod model_provisioning;
mod model_readiness;
mod production;
mod public_search;
mod retrieval_projection;
mod thin_search;
#[cfg(test)]
mod unified_search;
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
pub use model_provisioning::{
    ModelSetCommandReport, ModelSetCommandRole, prepare_production_model_set,
    production_model_set_status,
};
pub use model_readiness::{
    MODEL_SET_LOCK_FILE, MODEL_SET_READY_FILE, MODEL_SET_ROLES_DIRECTORY, MODEL_SET_SCHEMA,
    ModelArtifactDescriptor, ModelContractError, ModelRuntimeIdentity, ModelRuntimeMechanism,
    ModelRuntimeTarget, ModelSetReadySnapshot, ModelSetRoleSnapshot, PRODUCTION_MODEL_CATALOG,
    ProductionModelDescriptor, ROLE_MANIFEST_FILE, ROLE_READY_FILE, RoleArtifactManifest,
    RoleReadinessMarker, model_role_paths, model_set_identity_sha256, model_set_paths,
    production_model_descriptor,
};
pub use production::{ModelPartitionMetrics, ProductionConfig, ProductionRuntime};
pub use public_search::{
    DocumentStatus, MAX_PUBLIC_QUERY_CHARS, MAX_PUBLIC_RESULTS, MAX_SERIALIZED_RESPONSE_BYTES,
    ProjectScope, PublicSearchError, PublicSearchErrorCode, PublicSearchRequest,
    PublicSearchResponse, PublicSearchResult,
};
pub use thin_search::{SEARCH_DEADLINE, ThinSearchAudit, ThinSearchCoordinator};
pub use workspace::{
    CatalogEntry, DiscoveryReport, SourceRoot, WorkspaceCatalog, WorkspaceProfile, WorkspaceStore,
};
