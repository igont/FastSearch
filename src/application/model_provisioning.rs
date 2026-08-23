//! Production preparation for the four-role model set.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    adapters::{qwen_reranker::QwenReranker, vector::observe_embedding_model},
    domain::{ErrorKind, FastSearchError, ProductionModelRole},
};

use super::{
    ModelRuntimeIdentity, ModelSetReadySnapshot, ProductionModelDescriptor, RoleReadinessMarker,
    production_model_descriptor, publish_model_set_snapshot, read_model_set_snapshot,
    workspace::product_home,
};

const OFFICIAL_MANIFEST: &[u8] =
    include_bytes!("../../evidence/dt4/fixtures/ts-dt4-01/model-manifest.json");
const ORACLE: &[u8] = include_bytes!("../../evidence/dt4/fixtures/ts-dt4-02/oracle.json");
const ORACLE_CASES: &[u8] =
    include_bytes!("../../evidence/dt4/fixtures/ts-dt4-02/oracle-cases.json");

#[derive(Clone, Debug, Serialize)]
pub struct ModelSetCommandRole {
    role: String,
    repository: String,
    revision: String,
    marker_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelSetCommandReport {
    schema_version: u8,
    status: &'static str,
    kind: &'static str,
    ready: bool,
    generation: String,
    roles: Vec<ModelSetCommandRole>,
    downloaded_bytes: u64,
    duration_ms: u128,
}

impl ModelSetCommandReport {
    fn from_snapshot(
        snapshot: &ModelSetReadySnapshot,
        downloaded_bytes: u64,
        duration_ms: u128,
    ) -> Self {
        Self {
            schema_version: 1,
            status: "ok",
            kind: "model_set",
            ready: true,
            generation: snapshot.generation().to_owned(),
            roles: snapshot
                .roles()
                .iter()
                .map(|role| ModelSetCommandRole {
                    role: role.role().slug().to_owned(),
                    repository: role.repository().to_owned(),
                    revision: role.revision().to_owned(),
                    marker_sha256: role.marker_sha256().to_owned(),
                })
                .collect(),
            downloaded_bytes,
            duration_ms,
        }
    }
}

#[derive(Clone, Copy)]
struct SupplementalArtifact {
    role: ProductionModelRole,
    path: &'static str,
    bytes: u64,
    sha256: &'static str,
}

const SUPPLEMENTAL_ARTIFACTS: &[SupplementalArtifact] = &[
    SupplementalArtifact {
        role: ProductionModelRole::ArcticEmbedLV2,
        path: "tokenizer_config.json",
        bytes: 1_339,
        sha256: "cb058b4c5c0c08738eb028c2ae82ed55cd84ce8999ece76b13472af80f0f77f1",
    },
    SupplementalArtifact {
        role: ProductionModelRole::ArcticEmbedLV2,
        path: "special_tokens_map.json",
        bytes: 964,
        sha256: "8c785abebea9ae3257b61681b4e6fd8365ceafde980c21970d001e834cf10835",
    },
    SupplementalArtifact {
        role: ProductionModelRole::MultilingualE5Large,
        path: "tokenizer_config.json",
        bytes: 1_147,
        sha256: "f90024142df07163e5e6c5b9a6ad7c8c68b22a9112af11e3db4559a9ff90f737",
    },
    SupplementalArtifact {
        role: ProductionModelRole::MultilingualE5Large,
        path: "special_tokens_map.json",
        bytes: 964,
        sha256: "8c785abebea9ae3257b61681b4e6fd8365ceafde980c21970d001e834cf10835",
    },
];

#[derive(Deserialize)]
struct OracleDocument {
    id: String,
    text: String,
}

#[derive(Deserialize)]
struct OracleEmbeddingCases {
    query: String,
    documents: Vec<OracleDocument>,
}

#[derive(Deserialize)]
struct OracleRerankerPair {
    id: String,
    query: String,
    document: String,
}

#[derive(Deserialize)]
struct OracleRerankerCases {
    pairs: Vec<OracleRerankerPair>,
}

#[derive(Deserialize)]
struct OracleCases {
    embedding: OracleEmbeddingCases,
    reranker: OracleRerankerCases,
}

#[derive(Deserialize)]
struct OracleEmbeddingObservation {
    dimension: usize,
    query_norm: f32,
    query_components: Vec<f32>,
    scores: std::collections::BTreeMap<String, f32>,
    order: Vec<String>,
}

#[derive(Deserialize)]
struct OracleRerankerObservation {
    scores: std::collections::BTreeMap<String, f32>,
    order: Vec<String>,
}

#[derive(Deserialize)]
struct OracleTolerance {
    embedding_component_absolute: f32,
    embedding_norm_absolute: f32,
    reranker_probability_absolute: f32,
}

#[derive(Deserialize)]
struct OracleObservations {
    #[serde(rename = "arctic-embed-l-v2")]
    arctic: OracleEmbeddingObservation,
    #[serde(rename = "multilingual-e5-large")]
    e5: OracleEmbeddingObservation,
    #[serde(rename = "nomic-embed-text-v2-moe")]
    nomic: OracleEmbeddingObservation,
    #[serde(rename = "qwen3-reranker-0.6b")]
    qwen: OracleRerankerObservation,
}

#[derive(Deserialize)]
struct Oracle {
    tolerance: OracleTolerance,
    observations: OracleObservations,
}

pub fn prepare_production_model_set() -> Result<ModelSetCommandReport, FastSearchError> {
    let started = Instant::now();
    let model_root = model_root()?;
    let provider = ArtifactProvider::new()?;
    let oracle: Oracle = serde_json::from_slice(ORACLE).map_err(readiness_error)?;
    let cases: OracleCases = serde_json::from_slice(ORACLE_CASES).map_err(readiness_error)?;
    let mut markers = Vec::with_capacity(ProductionModelRole::ALL.len());
    let mut downloaded_bytes = 0;

    for role in ProductionModelRole::ALL {
        let descriptor = production_model_descriptor(role);
        let cache_root = model_root.join("artifacts").join(role.slug()).join("hub");
        let snapshot_root = provider.ensure(descriptor, &cache_root, &mut downloaded_bytes)?;
        verify_role(role, &cache_root, &snapshot_root, &oracle, &cases)?;
        let runtime = ModelRuntimeIdentity::qualified(role, runtime_environment_sha256(role))
            .map_err(readiness_error)?;
        markers.push(
            RoleReadinessMarker::new(
                role,
                descriptor.repository,
                descriptor.revision,
                descriptor.compute_contract_sha256(),
                descriptor.manifest_sha256(),
                runtime,
            )
            .map_err(readiness_error)?,
        );
    }

    let snapshot = publish_model_set_snapshot(&model_root, markers)?;
    Ok(ModelSetCommandReport::from_snapshot(
        &snapshot,
        downloaded_bytes,
        started.elapsed().as_millis(),
    ))
}

pub fn production_model_set_status() -> Result<ModelSetCommandReport, FastSearchError> {
    let started = Instant::now();
    let snapshot = read_model_set_snapshot(&model_root()?)?;
    Ok(ModelSetCommandReport::from_snapshot(
        &snapshot,
        0,
        started.elapsed().as_millis(),
    ))
}

struct ArtifactProvider {
    client: Client,
    endpoint: String,
}

impl ArtifactProvider {
    fn new() -> Result<Self, FastSearchError> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(30 * 60))
                .redirect(reqwest::redirect::Policy::limited(10))
                .user_agent("FastSearch/0.1 model-set-provider")
                .build()
                .map_err(readiness_error)?,
            endpoint: std::env::var("HF_ENDPOINT")
                .unwrap_or_else(|_| "https://huggingface.co".to_owned()),
        })
    }

    fn ensure(
        &self,
        descriptor: &ProductionModelDescriptor,
        cache_root: &Path,
        downloaded_bytes: &mut u64,
    ) -> Result<PathBuf, FastSearchError> {
        let repository_root = cache_root.join(format!(
            "models--{}",
            descriptor.repository.replace('/', "--")
        ));
        let snapshot = repository_root.join("snapshots").join(descriptor.revision);
        for artifact in descriptor.required_files {
            self.ensure_file(
                descriptor,
                artifact.path,
                artifact.bytes,
                artifact.sha256,
                &snapshot.join(artifact.path),
                downloaded_bytes,
            )?;
        }
        for artifact in SUPPLEMENTAL_ARTIFACTS
            .iter()
            .filter(|artifact| artifact.role == descriptor.role)
        {
            self.ensure_file(
                descriptor,
                artifact.path,
                artifact.bytes,
                artifact.sha256,
                &snapshot.join(artifact.path),
                downloaded_bytes,
            )?;
        }
        let refs = repository_root.join("refs");
        fs::create_dir_all(&refs).map_err(readiness_error)?;
        fs::write(refs.join("main"), descriptor.revision).map_err(readiness_error)?;
        Ok(snapshot)
    }

    fn ensure_file(
        &self,
        descriptor: &ProductionModelDescriptor,
        artifact_path: &str,
        expected_bytes: u64,
        expected_sha256: &str,
        target: &Path,
        downloaded_bytes: &mut u64,
    ) -> Result<(), FastSearchError> {
        if exact_file(target, expected_bytes, expected_sha256)? {
            return Ok(());
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(readiness_error)?;
        }
        let partial = target.with_extension(format!(
            "{}download",
            target
                .extension()
                .and_then(|value| value.to_str())
                .map_or("", |_| ".")
        ));
        if partial.exists() {
            fs::remove_file(&partial).map_err(readiness_error)?;
        }
        let url = format!(
            "{}/{}/resolve/{}/{}",
            self.endpoint.trim_end_matches('/'),
            descriptor.repository,
            descriptor.revision,
            artifact_path
        );
        let mut last_error = String::new();
        for attempt in 1..=3 {
            match self.client.get(&url).send() {
                Ok(mut response) if response.status().is_success() => {
                    let mut output = File::create(&partial).map_err(readiness_error)?;
                    match response.copy_to(&mut output) {
                        Ok(bytes) => {
                            output.flush().map_err(readiness_error)?;
                            *downloaded_bytes = downloaded_bytes.saturating_add(bytes);
                            if exact_file(&partial, expected_bytes, expected_sha256)? {
                                fs::rename(&partial, target).map_err(readiness_error)?;
                                return Ok(());
                            }
                            last_error = format!("digest or length mismatch for {artifact_path}");
                        }
                        Err(error) => last_error = error.to_string(),
                    }
                }
                Ok(response) => last_error = format!("HTTP {}", response.status()),
                Err(error) => last_error = error.to_string(),
            }
            if attempt < 3 {
                std::thread::sleep(Duration::from_secs(attempt));
            }
        }
        Err(readiness_error(format!(
            "failed to obtain {}@{} {artifact_path}: {last_error}",
            descriptor.repository, descriptor.revision
        )))
    }
}

fn verify_role(
    role: ProductionModelRole,
    cache_root: &Path,
    snapshot_root: &Path,
    oracle: &Oracle,
    cases: &OracleCases,
) -> Result<(), FastSearchError> {
    if let Some(model) = role.embedding_model() {
        let expected = match role {
            ProductionModelRole::ArcticEmbedLV2 => &oracle.observations.arctic,
            ProductionModelRole::MultilingualE5Large => &oracle.observations.e5,
            ProductionModelRole::NomicEmbedTextV2Moe => &oracle.observations.nomic,
            ProductionModelRole::Qwen3Reranker06B => unreachable!(),
        };
        let documents = cases
            .embedding
            .documents
            .iter()
            .map(|document| document.text.clone())
            .collect::<Vec<_>>();
        let actual =
            observe_embedding_model(model, cache_root, &cases.embedding.query, &documents)?;
        if actual.dimension != expected.dimension
            || (actual.norm - expected.query_norm).abs() > oracle.tolerance.embedding_norm_absolute
            || actual.components.len() != expected.query_components.len()
            || actual
                .components
                .iter()
                .zip(&expected.query_components)
                .any(|(left, right)| {
                    (left - right).abs() > oracle.tolerance.embedding_component_absolute
                })
        {
            return Err(readiness_error(format!(
                "{} embedding observation differs from the independent oracle",
                role.slug()
            )));
        }
        let actual_scores = cases
            .embedding
            .documents
            .iter()
            .zip(&actual.scores)
            .map(|(document, score)| (document.id.clone(), *score))
            .collect::<std::collections::BTreeMap<_, _>>();
        verify_order(&actual_scores, &expected.order, role)?;
        for (id, expected_score) in &expected.scores {
            let actual_score = actual_scores
                .get(id)
                .ok_or_else(|| readiness_error("missing embedding score"))?;
            if (actual_score - expected_score).abs() > oracle.tolerance.embedding_component_absolute
            {
                return Err(readiness_error(format!(
                    "{} score {id} differs from the independent oracle",
                    role.slug()
                )));
            }
        }
        return Ok(());
    }

    let manifest_path = snapshot_root.join("model-manifest.json");
    fs::write(&manifest_path, OFFICIAL_MANIFEST).map_err(readiness_error)?;
    let mut reranker =
        QwenReranker::open(snapshot_root, &manifest_path).map_err(readiness_error)?;
    let mut actual_scores = std::collections::BTreeMap::new();
    for pair in &cases.reranker.pairs {
        actual_scores.insert(
            pair.id.clone(),
            reranker
                .score(&pair.query, &pair.document)
                .map_err(readiness_error)?,
        );
    }
    verify_order(&actual_scores, &oracle.observations.qwen.order, role)?;
    for (id, expected) in &oracle.observations.qwen.scores {
        let actual = actual_scores
            .get(id)
            .ok_or_else(|| readiness_error("missing reranker score"))?;
        if (actual - expected).abs() > oracle.tolerance.reranker_probability_absolute {
            return Err(readiness_error(format!(
                "{} score {id} differs from the independent oracle",
                role.slug()
            )));
        }
    }
    Ok(())
}

fn verify_order(
    scores: &std::collections::BTreeMap<String, f32>,
    expected: &[String],
    role: ProductionModelRole,
) -> Result<(), FastSearchError> {
    let mut actual = scores.iter().collect::<Vec<_>>();
    actual.sort_by(|left, right| right.1.total_cmp(left.1));
    if actual
        .iter()
        .map(|(id, _)| id.as_str())
        .ne(expected.iter().map(String::as_str))
    {
        return Err(readiness_error(format!(
            "{} ordering differs from the independent oracle",
            role.slug()
        )));
    }
    Ok(())
}

fn exact_file(
    path: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<bool, FastSearchError> {
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(false);
    };
    if metadata.len() != expected_bytes {
        return Ok(false);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(readiness_error)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(readiness_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()) == expected_sha256)
}

fn runtime_environment_sha256(role: ProductionModelRole) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "fastsearch={}\0target=windows-x86_64-cpu\0role={}\0contract={}",
            env!("CARGO_PKG_VERSION"),
            role.slug(),
            production_model_descriptor(role).compute_contract
        ))
    )
}

fn model_root() -> Result<PathBuf, FastSearchError> {
    Ok(product_home()?.join("models").join("production"))
}

fn readiness_error(error: impl std::fmt::Display) -> FastSearchError {
    FastSearchError::new(
        ErrorKind::InvalidContent,
        format!("model-set preparation: {error}"),
    )
}
