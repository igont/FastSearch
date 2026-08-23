//! Contract for the four-role production model set.
//!
//! Artifact transport and role probes are implemented by later DT4 leaves. This
//! module owns stable identities and the single aggregate commit point they use.

use std::{
    collections::BTreeSet,
    fmt,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{ErrorKind, FastSearchError, ModelRoleKind, ProductionModelRole};

use super::workspace::atomic_write;

pub const MODEL_SET_SCHEMA: u8 = 1;
pub const MODEL_SET_LOCK_FILE: &str = "model-set.install.lock";
pub const MODEL_SET_READY_FILE: &str = "model-set.ready.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelArtifactDescriptor {
    pub path: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductionModelDescriptor {
    pub role: ProductionModelRole,
    pub kind: ModelRoleKind,
    pub repository: &'static str,
    pub revision: &'static str,
    pub compute_contract: &'static str,
    pub required_files: &'static [ModelArtifactDescriptor],
}

const ARCTIC_FILES: &[ModelArtifactDescriptor] = &[
    ModelArtifactDescriptor {
        path: "config.json",
        bytes: 818,
        sha256: "706f5d1eb9ddf64b5d3067c562b893b80ad1022db7f68c20eb07ff24d33b6c15",
    },
    ModelArtifactDescriptor {
        path: "tokenizer.json",
        bytes: 17_083_074,
        sha256: "39feb9863a378165ab9c5c689047203d789422966c0c58721c5309fd039a8edc",
    },
    ModelArtifactDescriptor {
        path: "onnx/model.onnx",
        bytes: 702_280,
        sha256: "f74aa79745ccfb1e75daa7e8e6552a78402d4de193eb8ca67a931358d3e0a25e",
    },
    ModelArtifactDescriptor {
        path: "onnx/model.onnx_data",
        bytes: 2_266_886_160,
        sha256: "fe7d75ff258fbda10a6bea63c5422df5579d625355b1aca69ba6923c0ba604a9",
    },
];
const E5_LARGE_FILES: &[ModelArtifactDescriptor] = &[
    ModelArtifactDescriptor {
        path: "config.json",
        bytes: 716,
        sha256: "1de8c3be1f344c0eefa4962480a006f8639f416dfafaa95a770e3cf4bceae6a4",
    },
    ModelArtifactDescriptor {
        path: "tokenizer.json",
        bytes: 17_082_756,
        sha256: "f59925fcb90c92b894cb93e51bb9b4a6105c5c249fe54ce1c704420ac39b81af",
    },
    ModelArtifactDescriptor {
        path: "model.onnx",
        bytes: 545_851,
        sha256: "1c09780c907c8a91a77a6ab1fd231f79e090d2907ca431223703dfebeed3d36c",
    },
    ModelArtifactDescriptor {
        path: "model.onnx_data",
        bytes: 2_235_363_328,
        sha256: "0cf1883fee81c63819a44e2ba0efa51d4043d9759685a4ebebbde97e0623d15c",
    },
];
const NOMIC_FILES: &[ModelArtifactDescriptor] = &[
    ModelArtifactDescriptor {
        path: "config.json",
        bytes: 2_482,
        sha256: "4f076b4798fc2ba916f1900e0d10177714ee1aa94e0ef809102e723078d3efd3",
    },
    ModelArtifactDescriptor {
        path: "tokenizer.json",
        bytes: 17_082_734,
        sha256: "3a56def25aa40facc030ea8b0b87f3688e4b3c39eb8b45d5702b3a1300fe2a20",
    },
    ModelArtifactDescriptor {
        path: "model.safetensors",
        bytes: 1_901_187_232,
        sha256: "097012b27af76d80af74fed4bc2ccc9091245286f776adf03ad1758a24ade9a0",
    },
];
const QWEN_RERANKER_FILES: &[ModelArtifactDescriptor] = &[
    ModelArtifactDescriptor {
        path: "config.json",
        bytes: 727,
        sha256: "d479c427a9ca5295218063d4f9aca4f297ab4ac27487cca7af42c84643d51ef0",
    },
    ModelArtifactDescriptor {
        path: "tokenizer.json",
        bytes: 11_422_654,
        sha256: "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
    },
    ModelArtifactDescriptor {
        path: "model.safetensors",
        bytes: 1_191_588_280,
        sha256: "27cd75a405b9c1b46b59abfd88aaa209e6fed2a1972cde9b70e7659537c5e65b",
    },
];

pub const PRODUCTION_MODEL_CATALOG: [ProductionModelDescriptor; 4] = [
    ProductionModelDescriptor {
        role: ProductionModelRole::ArcticEmbedLV2,
        kind: ModelRoleKind::Embedding,
        repository: "Snowflake/snowflake-arctic-embed-l-v2.0",
        revision: "ac6544c8a46e00af67e330e85a9028c66b8cfd9a",
        compute_contract: "query-prefix=query: ;document-prefix=;truncate=right:8192;pool=cls;dimension=1024;normalize=l2;similarity=dot",
        required_files: ARCTIC_FILES,
    },
    ProductionModelDescriptor {
        role: ProductionModelRole::MultilingualE5Large,
        kind: ModelRoleKind::Embedding,
        repository: "Qdrant/multilingual-e5-large-onnx",
        revision: "66076b8dc6e367337e3e90e6fb309fb0f3addaf6",
        compute_contract: "query-prefix=query: ;document-prefix=passage: ;truncate=right:512;pool=attention-mean;dimension=1024;normalize=l2;similarity=dot",
        required_files: E5_LARGE_FILES,
    },
    ProductionModelDescriptor {
        role: ProductionModelRole::NomicEmbedTextV2Moe,
        kind: ModelRoleKind::Embedding,
        repository: "nomic-ai/nomic-embed-text-v2-moe",
        revision: "1066b6599d099fbb93dfcb64f9c37a7c9e503e85",
        compute_contract: "query-prefix=search_query: ;document-prefix=search_document: ;truncate=right:512;pool=attention-mean;dimension=768;matryoshka=full;normalize=l2;similarity=dot",
        required_files: NOMIC_FILES,
    },
    ProductionModelDescriptor {
        role: ProductionModelRole::Qwen3Reranker06B,
        kind: ModelRoleKind::Reranker,
        repository: "Qwen/Qwen3-Reranker-0.6B",
        revision: "e61197ed45024b0ed8a2d74b80b4d909f1255473",
        compute_contract: "engine=candle-qwen3-causal-lm;max-tokens=8192;padding=left;truncate-body=right;yes-token=9693;no-token=2152;score=softmax-yes",
        required_files: QWEN_RERANKER_FILES,
    },
];

#[must_use]
pub fn production_model_descriptor(
    role: ProductionModelRole,
) -> &'static ProductionModelDescriptor {
    PRODUCTION_MODEL_CATALOG
        .iter()
        .find(|descriptor| descriptor.role == role)
        .expect("the production model catalog covers every role")
}

impl ProductionModelDescriptor {
    #[must_use]
    pub fn compute_contract_sha256(&self) -> String {
        sha256(self.compute_contract.as_bytes())
    }

    #[must_use]
    pub fn manifest_sha256(&self) -> String {
        let mut canonical = String::new();
        for artifact in self.required_files {
            canonical.push_str(artifact.path);
            canonical.push('\0');
            canonical.push_str(&artifact.bytes.to_string());
            canonical.push('\0');
            canonical.push_str(artifact.sha256);
            canonical.push('\n');
        }
        sha256(canonical.as_bytes())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelContractError {
    WrongRoleKind,
    DuplicateRole(ProductionModelRole),
    IdentityMismatch(ProductionModelRole),
    InvalidDigest,
    MissingRole(ProductionModelRole),
    GenerationMismatch,
    MarkerMismatch(ProductionModelRole),
    InvalidSchema(u8),
    InvalidJson,
}

impl fmt::Display for ModelContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ModelContractError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleReadinessMarker {
    schema: u8,
    role: ProductionModelRole,
    repository: String,
    revision: String,
    compute_contract_sha256: String,
    manifest_sha256: String,
    runtime: String,
}

impl RoleReadinessMarker {
    pub fn new(
        role: ProductionModelRole,
        repository: impl Into<String>,
        revision: impl Into<String>,
        compute_contract_sha256: impl Into<String>,
        manifest_sha256: impl Into<String>,
        runtime: impl Into<String>,
    ) -> Result<Self, ModelContractError> {
        let marker = Self {
            schema: MODEL_SET_SCHEMA,
            role,
            repository: repository.into(),
            revision: revision.into(),
            compute_contract_sha256: compute_contract_sha256.into(),
            manifest_sha256: manifest_sha256.into(),
            runtime: runtime.into(),
        };
        marker.validate()?;
        Ok(marker)
    }

    fn validate(&self) -> Result<(), ModelContractError> {
        if self.schema != MODEL_SET_SCHEMA {
            return Err(ModelContractError::InvalidSchema(self.schema));
        }
        let expected = production_model_descriptor(self.role);
        if expected.kind != self.role.kind() {
            return Err(ModelContractError::WrongRoleKind);
        }
        if self.repository != expected.repository
            || self.revision != expected.revision
            || self.compute_contract_sha256 != expected.compute_contract_sha256()
            || self.manifest_sha256 != expected.manifest_sha256()
        {
            return Err(ModelContractError::IdentityMismatch(self.role));
        }
        if !is_sha256(&self.compute_contract_sha256) || !is_sha256(&self.manifest_sha256) {
            return Err(ModelContractError::InvalidDigest);
        }
        Ok(())
    }

    #[must_use]
    pub const fn role(&self) -> ProductionModelRole {
        self.role
    }

    #[must_use]
    pub fn marker_sha256(&self) -> String {
        sha256(
            serde_json::to_vec(self)
                .expect("role marker is serializable")
                .as_slice(),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelSetRoleSnapshot {
    role: ProductionModelRole,
    repository: String,
    revision: String,
    compute_contract_sha256: String,
    marker_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelSetReadySnapshot {
    schema: u8,
    generation: String,
    roles: Vec<ModelSetRoleSnapshot>,
}

impl ModelSetReadySnapshot {
    pub fn new(markers: Vec<RoleReadinessMarker>) -> Result<Self, ModelContractError> {
        let mut by_role = markers
            .into_iter()
            .map(|marker| (marker.role(), marker))
            .collect::<Vec<_>>();
        by_role.sort_by_key(|(role, _)| role_order(*role));
        let unique = by_role
            .iter()
            .map(|(role, _)| *role)
            .collect::<BTreeSet<_>>();
        if unique.len() != by_role.len() {
            let duplicate = by_role
                .windows(2)
                .find(|pair| pair[0].0 == pair[1].0)
                .map(|pair| pair[0].0)
                .expect("a duplicate role exists");
            return Err(ModelContractError::DuplicateRole(duplicate));
        }
        for role in ProductionModelRole::ALL {
            if !unique.contains(&role) {
                return Err(ModelContractError::MissingRole(role));
            }
        }
        let roles = by_role
            .into_iter()
            .map(|(_, marker)| {
                marker.validate()?;
                let marker_sha256 = marker.marker_sha256();
                Ok(ModelSetRoleSnapshot {
                    role: marker.role,
                    repository: marker.repository,
                    revision: marker.revision,
                    compute_contract_sha256: marker.compute_contract_sha256,
                    marker_sha256,
                })
            })
            .collect::<Result<Vec<_>, ModelContractError>>()?;
        let generation = generation(&roles);
        Ok(Self {
            schema: MODEL_SET_SCHEMA,
            generation,
            roles,
        })
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ModelContractError> {
        let snapshot: Self =
            serde_json::from_slice(bytes).map_err(|_| ModelContractError::InvalidJson)?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn validate_current_markers(
        &self,
        marker_hashes: &[(ProductionModelRole, String)],
    ) -> Result<(), ModelContractError> {
        self.validate()?;
        let current_roles = marker_hashes
            .iter()
            .map(|(role, _)| *role)
            .collect::<BTreeSet<_>>();
        if current_roles.len() != marker_hashes.len() {
            let mut roles = marker_hashes
                .iter()
                .map(|(role, _)| *role)
                .collect::<Vec<_>>();
            roles.sort_by_key(|role| role_order(*role));
            let duplicate = roles
                .windows(2)
                .find(|pair| pair[0] == pair[1])
                .map(|pair| pair[0])
                .expect("a duplicate current role exists");
            return Err(ModelContractError::DuplicateRole(duplicate));
        }
        for role in ProductionModelRole::ALL {
            if !current_roles.contains(&role) {
                return Err(ModelContractError::MissingRole(role));
            }
        }
        for expected in &self.roles {
            let actual = marker_hashes
                .iter()
                .find(|(role, _)| *role == expected.role)
                .map(|(_, hash)| hash.as_str())
                .ok_or(ModelContractError::MissingRole(expected.role))?;
            if actual != expected.marker_sha256 {
                return Err(ModelContractError::MarkerMismatch(expected.role));
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), ModelContractError> {
        if self.schema != MODEL_SET_SCHEMA {
            return Err(ModelContractError::InvalidSchema(self.schema));
        }
        if self.roles.len() != ProductionModelRole::ALL.len() {
            return Err(ModelContractError::MissingRole(
                ProductionModelRole::ALL
                    .iter()
                    .copied()
                    .find(|role| !self.roles.iter().any(|item| item.role == *role))
                    .unwrap_or(ProductionModelRole::ArcticEmbedLV2),
            ));
        }
        for (index, role) in ProductionModelRole::ALL.iter().enumerate() {
            let actual = &self.roles[index];
            let descriptor = production_model_descriptor(*role);
            if actual.role != *role
                || actual.repository != descriptor.repository
                || actual.revision != descriptor.revision
                || actual.compute_contract_sha256 != descriptor.compute_contract_sha256()
            {
                return Err(ModelContractError::IdentityMismatch(actual.role));
            }
            if !is_sha256(&actual.marker_sha256) {
                return Err(ModelContractError::InvalidDigest);
            }
        }
        if generation(&self.roles) != self.generation {
            return Err(ModelContractError::GenerationMismatch);
        }
        Ok(())
    }

    #[must_use]
    pub fn generation(&self) -> &str {
        &self.generation
    }

    #[must_use]
    pub fn roles(&self) -> &[ModelSetRoleSnapshot] {
        &self.roles
    }

    #[must_use]
    pub fn to_json(&self) -> Vec<u8> {
        let mut bytes =
            serde_json::to_vec_pretty(self).expect("model set snapshot is serializable");
        bytes.push(b'\n');
        bytes
    }
}

/// Atomically publishes one generation while holding the shared model-set lock.
pub fn publish_model_set_snapshot(
    model_root: &Path,
    markers: Vec<RoleReadinessMarker>,
) -> Result<ModelSetReadySnapshot, FastSearchError> {
    with_model_set_lock(model_root, || {
        let snapshot = ModelSetReadySnapshot::new(markers).map_err(contract_error)?;
        atomic_write(&model_root.join(MODEL_SET_READY_FILE), &snapshot.to_json())?;
        Ok(snapshot)
    })
}

/// Reads and verifies one generation and its current role markers under the same lock.
pub fn read_model_set_snapshot(
    model_root: &Path,
    marker_hashes: &[(ProductionModelRole, String)],
) -> Result<ModelSetReadySnapshot, FastSearchError> {
    with_model_set_lock(model_root, || {
        let bytes = fs::read(model_root.join(MODEL_SET_READY_FILE)).map_err(state_error)?;
        let snapshot = ModelSetReadySnapshot::from_json(&bytes).map_err(contract_error)?;
        snapshot
            .validate_current_markers(marker_hashes)
            .map_err(contract_error)?;
        Ok(snapshot)
    })
}

fn with_model_set_lock<T>(
    model_root: &Path,
    operation: impl FnOnce() -> Result<T, FastSearchError>,
) -> Result<T, FastSearchError> {
    fs::create_dir_all(model_root).map_err(state_error)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(model_root.join(MODEL_SET_LOCK_FILE))
        .map_err(state_error)?;
    lock.lock_exclusive().map_err(state_error)?;
    operation()
}

fn role_order(role: ProductionModelRole) -> usize {
    ProductionModelRole::ALL
        .iter()
        .position(|candidate| *candidate == role)
        .expect("every production role has a canonical order")
}

fn generation(roles: &[ModelSetRoleSnapshot]) -> String {
    let mut canonical = Vec::new();
    for role in roles {
        for field in [
            role.role.slug(),
            role.repository.as_str(),
            role.revision.as_str(),
            role.compute_contract_sha256.as_str(),
            role.marker_sha256.as_str(),
        ] {
            canonical.extend_from_slice(&(field.len() as u64).to_be_bytes());
            canonical.extend_from_slice(field.as_bytes());
        }
    }
    sha256(&canonical)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn contract_error(error: ModelContractError) -> FastSearchError {
    FastSearchError::new(
        ErrorKind::InvalidContent,
        format!("model-set contract: {error}"),
    )
}

fn state_error(error: std::io::Error) -> FastSearchError {
    FastSearchError::new(
        ErrorKind::StateFailure,
        format!("model-set storage: {error}"),
    )
}

#[must_use]
pub fn model_set_paths(model_root: &Path) -> (PathBuf, PathBuf) {
    (
        model_root.join(MODEL_SET_LOCK_FILE),
        model_root.join(MODEL_SET_READY_FILE),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::SystemTime,
    };

    use super::*;

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self(std::env::temp_dir().join(format!(
                "fastsearch-model-set-contract-{}-{unique}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn markers() -> Vec<RoleReadinessMarker> {
        ProductionModelRole::ALL
            .into_iter()
            .map(|role| {
                let descriptor = production_model_descriptor(role);
                RoleReadinessMarker::new(
                    role,
                    descriptor.repository,
                    descriptor.revision,
                    descriptor.compute_contract_sha256(),
                    descriptor.manifest_sha256(),
                    "windows-x86_64/cpu",
                )
                .unwrap()
            })
            .collect()
    }

    #[test]
    fn production_catalog_has_exact_roles_and_revisions() {
        assert_eq!(
            PRODUCTION_MODEL_CATALOG.map(|descriptor| descriptor.role),
            ProductionModelRole::ALL
        );
        for descriptor in PRODUCTION_MODEL_CATALOG {
            assert_eq!(descriptor.kind, descriptor.role.kind());
            assert_eq!(descriptor.revision.len(), 40);
            assert!(!descriptor.required_files.is_empty());
            assert!(is_sha256(&descriptor.compute_contract_sha256()));
            assert!(is_sha256(&descriptor.manifest_sha256()));
        }
    }

    #[test]
    fn wrong_revision_and_qwen_embedding_identity_fail_closed() {
        let qwen = production_model_descriptor(ProductionModelRole::Qwen3Reranker06B);
        assert!(matches!(
            RoleReadinessMarker::new(
                ProductionModelRole::Qwen3Reranker06B,
                "Qwen/Qwen3-Embedding-0.6B",
                "97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3",
                qwen.compute_contract_sha256(),
                qwen.manifest_sha256(),
                "windows-x86_64/cpu",
            ),
            Err(ModelContractError::IdentityMismatch(_))
        ));
    }

    #[test]
    fn generation_is_stable_but_mixed_or_duplicate_markers_are_rejected() {
        let first = ModelSetReadySnapshot::new(markers()).unwrap();
        let mut reversed = markers();
        reversed.reverse();
        let second = ModelSetReadySnapshot::new(reversed).unwrap();
        assert_eq!(first.generation(), second.generation());

        let mut duplicate = markers();
        duplicate[1] = duplicate[0].clone();
        assert!(matches!(
            ModelSetReadySnapshot::new(duplicate),
            Err(ModelContractError::DuplicateRole(_))
        ));

        let mut hashes = markers()
            .into_iter()
            .map(|marker| (marker.role(), marker.marker_sha256()))
            .collect::<Vec<_>>();
        hashes[2].1 = "00".repeat(32);
        assert!(matches!(
            first.validate_current_markers(&hashes),
            Err(ModelContractError::MarkerMismatch(
                ProductionModelRole::NomicEmbedTextV2Moe
            ))
        ));

        let mut duplicate_hashes = markers()
            .into_iter()
            .map(|marker| (marker.role(), marker.marker_sha256()))
            .collect::<Vec<_>>();
        duplicate_hashes[1] = duplicate_hashes[0].clone();
        assert!(matches!(
            first.validate_current_markers(&duplicate_hashes),
            Err(ModelContractError::DuplicateRole(_))
        ));
    }

    #[test]
    fn serialized_snapshot_rejects_a_forged_generation() {
        let snapshot = ModelSetReadySnapshot::new(markers()).unwrap();
        let mut value = serde_json::to_value(snapshot).unwrap();
        value["generation"] = serde_json::Value::String("00".repeat(32));
        assert_eq!(
            ModelSetReadySnapshot::from_json(&serde_json::to_vec(&value).unwrap()),
            Err(ModelContractError::GenerationMismatch)
        );
    }

    #[test]
    fn aggregate_marker_is_published_and_read_through_one_lock() {
        let root = TempRoot::new();
        let markers = markers();
        let hashes = markers
            .iter()
            .map(|marker| (marker.role(), marker.marker_sha256()))
            .collect::<Vec<_>>();
        let published = publish_model_set_snapshot(&root.0, markers).unwrap();
        let observed = read_model_set_snapshot(&root.0, &hashes).unwrap();
        assert_eq!(observed, published);
        let (lock, ready) = model_set_paths(&root.0);
        assert!(lock.is_file());
        assert!(ready.is_file());
    }
}
