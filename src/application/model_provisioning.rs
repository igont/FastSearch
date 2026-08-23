//! Production preparation for the four-role model set.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use reqwest::blocking::Client;
use reqwest::{StatusCode, header::RANGE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};

use crate::{
    adapters::{qwen_reranker::QwenReranker, vector::observe_embedding_model},
    domain::{ErrorKind, FastSearchError, ProductionModelRole},
};

use super::{
    ModelRuntimeIdentity, ModelSetReadySnapshot, ProductionModelDescriptor, RoleReadinessMarker,
    model_readiness::{
        publish_model_set_snapshot_locked, read_model_set_snapshot_with_artifacts,
        with_model_set_lock,
    },
    production_model_descriptor,
    workspace::product_home,
};

#[cfg(test)]
use super::{model_readiness::read_model_set_snapshot, publish_model_set_snapshot};

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
    let (snapshot, downloaded_bytes) = with_model_set_lock(&model_root, || {
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

        let snapshot = publish_model_set_snapshot_locked(&model_root, markers)?;
        Ok((snapshot, downloaded_bytes))
    })?;
    Ok(ModelSetCommandReport::from_snapshot(
        &snapshot,
        downloaded_bytes,
        started.elapsed().as_millis(),
    ))
}

pub fn production_model_set_status() -> Result<ModelSetCommandReport, FastSearchError> {
    let started = Instant::now();
    let model_root = model_root()?;
    let snapshot = read_model_set_snapshot_with_artifacts(&model_root, |role| {
        verify_cached_artifacts(&model_root, role)
    })
    .map_err(model_set_not_ready)?;
    Ok(ModelSetCommandReport::from_snapshot(
        &snapshot,
        0,
        started.elapsed().as_millis(),
    ))
}

fn verify_cached_artifacts(
    model_root: &Path,
    role: ProductionModelRole,
) -> Result<(), FastSearchError> {
    let descriptor = production_model_descriptor(role);
    let snapshot_root = model_root
        .join("artifacts")
        .join(role.slug())
        .join("hub")
        .join(format!(
            "models--{}",
            descriptor.repository.replace('/', "--")
        ))
        .join("snapshots")
        .join(descriptor.revision);
    for artifact in descriptor.required_files {
        if !exact_file(
            &snapshot_root.join(artifact.path),
            artifact.bytes,
            artifact.sha256,
        )? {
            return Err(readiness_error(format!(
                "{} artifact {} is not exact",
                role.slug(),
                artifact.path
            )));
        }
    }
    Ok(())
}

fn model_set_not_ready(error: FastSearchError) -> FastSearchError {
    FastSearchError::new(
        ErrorKind::InvalidContent,
        format!("MODEL_SET_NOT_READY: {}", error.message()),
    )
}

struct ArtifactProvider {
    client: Client,
    endpoint: String,
    retry_delays: [Duration; 2],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PartialArtifactIdentity {
    schema: u8,
    repository: String,
    revision: String,
    artifact_path: String,
    expected_bytes: u64,
    expected_sha256: String,
}

impl PartialArtifactIdentity {
    fn new(
        descriptor: &ProductionModelDescriptor,
        artifact_path: &str,
        expected_bytes: u64,
        expected_sha256: &str,
    ) -> Self {
        Self {
            schema: 1,
            repository: descriptor.repository.to_owned(),
            revision: descriptor.revision.to_owned(),
            artifact_path: artifact_path.to_owned(),
            expected_bytes,
            expected_sha256: expected_sha256.to_owned(),
        }
    }

    fn sha256(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("partial identity is serializable"))
        )
    }
}

#[derive(Debug)]
enum ResponseBodyError {
    Oversize,
    Other(String),
}

impl std::fmt::Display for ResponseBodyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Oversize => {
                formatter.write_str("response body exceeds the admitted artifact length")
            }
            Self::Other(error) => formatter.write_str(error),
        }
    }
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
            retry_delays: [Duration::from_secs(1), Duration::from_secs(2)],
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
        let identity = PartialArtifactIdentity::new(
            descriptor,
            artifact_path,
            expected_bytes,
            expected_sha256,
        );
        let (partial, partial_identity) = partial_state_paths(target, &identity)?;
        let url = format!(
            "{}/{}/resolve/{}/{}",
            self.endpoint.trim_end_matches('/'),
            descriptor.repository,
            descriptor.revision,
            artifact_path
        );
        let mut last_error = String::new();
        for attempt in 1..=3 {
            let mut offset =
                partial_offset(&partial, &partial_identity, &identity, expected_bytes)?;
            if offset == expected_bytes {
                if exact_file(&partial, expected_bytes, expected_sha256)? {
                    publish_verified_partial(&partial, &partial_identity, target)?;
                    return Ok(());
                }
                remove_partial_state(&partial, &partial_identity)?;
                offset = 0;
            }
            let mut request = self.client.get(&url);
            if offset > 0 {
                request = request.header(RANGE, format!("bytes={offset}-"));
            }
            match request.send() {
                Ok(mut response) => {
                    if let Err(error) = validate_response_range(&response, offset, expected_bytes) {
                        last_error = error;
                    } else {
                        match append_response(
                            &mut response,
                            &partial,
                            &partial_identity,
                            &identity,
                            offset,
                            expected_bytes,
                            downloaded_bytes,
                        ) {
                            Ok(()) if exact_file(&partial, expected_bytes, expected_sha256)? => {
                                publish_verified_partial(&partial, &partial_identity, target)?;
                                return Ok(());
                            }
                            Ok(()) => {
                                last_error = format!("SHA-256 mismatch for {artifact_path}");
                                remove_partial_state(&partial, &partial_identity)?;
                            }
                            Err(ResponseBodyError::Oversize) => {
                                last_error = ResponseBodyError::Oversize.to_string();
                                remove_partial_state(&partial, &partial_identity)?;
                            }
                            Err(error) => last_error = error.to_string(),
                        }
                    }
                }
                Err(error) => last_error = error.to_string(),
            }
            if attempt < 3 {
                std::thread::sleep(self.retry_delays[attempt - 1]);
            }
        }
        Err(readiness_error(format!(
            "failed to obtain {}@{} {artifact_path}: {last_error}",
            descriptor.repository, descriptor.revision
        )))
    }
}

fn publish_verified_partial(
    partial: &Path,
    partial_identity: &Path,
    target: &Path,
) -> Result<(), FastSearchError> {
    publish_verified_partial_with(partial, target, atomic_replace_existing)?;
    if partial_identity.exists() {
        let _ = fs::remove_file(partial_identity);
    }
    Ok(())
}

fn publish_verified_partial_with(
    partial: &Path,
    target: &Path,
    replace_existing: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), FastSearchError> {
    if target.exists() {
        replace_existing(target, partial).map_err(readiness_error)
    } else {
        fs::rename(partial, target).map_err(readiness_error)
    }
}

#[cfg(windows)]
fn atomic_replace_existing(target: &Path, replacement: &Path) -> std::io::Result<()> {
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement = replacement
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both buffers are live, NUL-terminated UTF-16 paths. ReplaceFileW
    // is the single Windows replacement operation; failure leaves the old
    // target name in place instead of exposing a delete/rename gap.
    let replaced = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace_existing(target: &Path, replacement: &Path) -> std::io::Result<()> {
    fs::rename(replacement, target)
}

fn partial_state_paths(
    target: &Path,
    identity: &PartialArtifactIdentity,
) -> Result<(PathBuf, PathBuf), FastSearchError> {
    let parent = target
        .parent()
        .ok_or_else(|| readiness_error("model artifact target has no parent"))?;
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| readiness_error("model artifact target has no UTF-8 file name"))?;
    let partial = parent.join(format!(
        ".{file_name}.fastsearch-{}.download",
        identity.sha256()
    ));
    let partial_identity = partial.with_extension("download.identity.json");
    Ok((partial, partial_identity))
}

fn partial_offset(
    partial: &Path,
    partial_identity: &Path,
    expected_identity: &PartialArtifactIdentity,
    expected_bytes: u64,
) -> Result<u64, FastSearchError> {
    let Ok(metadata) = fs::metadata(partial) else {
        if partial_identity.exists() {
            fs::remove_file(partial_identity).map_err(readiness_error)?;
        }
        return Ok(0);
    };
    let identity_matches = fs::read(partial_identity)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PartialArtifactIdentity>(&bytes).ok())
        .is_some_and(|identity| identity == *expected_identity);
    if !identity_matches {
        remove_partial_state(partial, partial_identity)?;
        return Ok(0);
    }
    if metadata.len() <= expected_bytes {
        return Ok(metadata.len());
    }
    remove_partial_state(partial, partial_identity)?;
    Ok(0)
}

fn remove_partial_state(partial: &Path, partial_identity: &Path) -> Result<(), FastSearchError> {
    if partial.exists() {
        fs::remove_file(partial).map_err(readiness_error)?;
    }
    if partial_identity.exists() {
        fs::remove_file(partial_identity).map_err(readiness_error)?;
    }
    Ok(())
}

fn ensure_partial_identity(
    partial_identity: &Path,
    identity: &PartialArtifactIdentity,
) -> Result<(), ResponseBodyError> {
    if partial_identity.exists() {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(identity)
        .map_err(|error| ResponseBodyError::Other(error.to_string()))?;
    let mut file = File::create(partial_identity)
        .map_err(|error| ResponseBodyError::Other(error.to_string()))?;
    file.write_all(&bytes)
        .map_err(|error| ResponseBodyError::Other(error.to_string()))?;
    file.flush()
        .map_err(|error| ResponseBodyError::Other(error.to_string()))?;
    file.sync_all()
        .map_err(|error| ResponseBodyError::Other(error.to_string()))
}

fn validate_response_range(
    response: &reqwest::blocking::Response,
    offset: u64,
    expected_bytes: u64,
) -> Result<(), String> {
    if expected_bytes == 0 {
        return Err("zero-length model artifacts are not admitted".to_owned());
    }
    if offset == 0 {
        return (response.status() == StatusCode::OK)
            .then_some(())
            .ok_or_else(|| format!("expected HTTP 200, got {}", response.status()));
    }
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(format!(
            "expected HTTP 206 for Range bytes={offset}-, got {}",
            response.status()
        ));
    }
    let expected = format!("bytes {offset}-{}/{expected_bytes}", expected_bytes - 1);
    let actual = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok());
    if actual != Some(expected.as_str()) {
        return Err(format!(
            "unexpected Content-Range: expected {expected}, got {}",
            actual.unwrap_or("<missing>")
        ));
    }
    Ok(())
}

fn append_response(
    response: &mut reqwest::blocking::Response,
    partial: &Path,
    partial_identity: &Path,
    identity: &PartialArtifactIdentity,
    offset: u64,
    expected_bytes: u64,
    downloaded_bytes: &mut u64,
) -> Result<(), ResponseBodyError> {
    ensure_partial_identity(partial_identity, identity)?;
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .append(offset > 0)
        .truncate(offset == 0)
        .open(partial)
        .map_err(|error| ResponseBodyError::Other(error.to_string()))?;
    let expected_body = expected_bytes - offset;
    let mut received = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = match response.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                output
                    .flush()
                    .and_then(|()| output.sync_all())
                    .map_err(|sync_error| ResponseBodyError::Other(sync_error.to_string()))?;
                return Err(ResponseBodyError::Other(error.to_string()));
            }
        };
        if read == 0 {
            break;
        }
        let read =
            u64::try_from(read).map_err(|error| ResponseBodyError::Other(error.to_string()))?;
        if received.saturating_add(read) > expected_body {
            return Err(ResponseBodyError::Oversize);
        }
        output
            .write_all(&buffer[..read as usize])
            .map_err(|error| ResponseBodyError::Other(error.to_string()))?;
        received += read;
        *downloaded_bytes = downloaded_bytes.saturating_add(read);
    }
    output
        .flush()
        .map_err(|error| ResponseBodyError::Other(error.to_string()))?;
    output
        .sync_all()
        .map_err(|error| ResponseBodyError::Other(error.to_string()))?;
    if received != expected_body {
        return Err(ResponseBodyError::Other(format!(
            "incomplete response body: expected {expected_body} bytes, received {received}"
        )));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Write as _},
        net::TcpListener,
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::SystemTime,
    };

    use super::*;

    const BODY: &[u8] = b"0123456789abcdef";
    const BODY_SHA256: &str = "9f9f5111f7b27a781f1f1ddde5ebc2dd2b796bfc7365c9c28b548e564176929f";
    const TEST_FILES: &[super::super::ModelArtifactDescriptor] = &[];
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "fastsearch-provider-{label}-{}-{unique}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Reply {
        status: &'static str,
        headers: Vec<String>,
        body: Vec<u8>,
    }

    impl Reply {
        fn ok(body: &[u8], declared_length: usize) -> Self {
            Self {
                status: "200 OK",
                headers: vec![format!("Content-Length: {declared_length}")],
                body: body.to_vec(),
            }
        }

        fn partial(body: &[u8], content_range: &str) -> Self {
            Self {
                status: "206 Partial Content",
                headers: vec![
                    format!("Content-Length: {}", body.len()),
                    format!("Content-Range: {content_range}"),
                ],
                body: body.to_vec(),
            }
        }

        fn failure() -> Self {
            Self {
                status: "503 Service Unavailable",
                headers: vec!["Content-Length: 0".to_owned()],
                body: Vec::new(),
            }
        }
    }

    fn serve(replies: Vec<Reply>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut requests = Vec::with_capacity(replies.len());
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8(request).unwrap());
                let mut response = format!("HTTP/1.1 {}\r\n", reply.status);
                for header in reply.headers {
                    response.push_str(&header);
                    response.push_str("\r\n");
                }
                response.push_str("Connection: close\r\n\r\n");
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(&reply.body).unwrap();
                stream.flush().unwrap();
            }
            requests
        });
        (endpoint, handle)
    }

    fn provider(endpoint: String) -> ArtifactProvider {
        ArtifactProvider {
            client: Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            endpoint,
            retry_delays: [Duration::ZERO, Duration::ZERO],
        }
    }

    fn descriptor() -> ProductionModelDescriptor {
        ProductionModelDescriptor {
            role: ProductionModelRole::Qwen3Reranker06B,
            kind: crate::domain::ModelRoleKind::Reranker,
            repository: "fixture/repository",
            revision: "fixture-revision",
            compute_contract: "fixture",
            required_files: TEST_FILES,
        }
    }

    fn marker(role: ProductionModelRole) -> RoleReadinessMarker {
        let descriptor = production_model_descriptor(role);
        RoleReadinessMarker::new(
            role,
            descriptor.repository,
            descriptor.revision,
            descriptor.compute_contract_sha256(),
            descriptor.manifest_sha256(),
            ModelRuntimeIdentity::qualified(role, "11".repeat(32)).unwrap(),
        )
        .unwrap()
    }

    fn partial_fixture_paths(target: &Path, artifact_path: &str) -> (PathBuf, PathBuf) {
        let identity = PartialArtifactIdentity::new(
            &descriptor(),
            artifact_path,
            BODY.len() as u64,
            BODY_SHA256,
        );
        partial_state_paths(target, &identity).unwrap()
    }

    fn seed_partial(target: &Path, artifact_path: &str, bytes: &[u8]) -> (PathBuf, PathBuf) {
        let identity = PartialArtifactIdentity::new(
            &descriptor(),
            artifact_path,
            BODY.len() as u64,
            BODY_SHA256,
        );
        let paths = partial_state_paths(target, &identity).unwrap();
        fs::write(&paths.0, bytes).unwrap();
        fs::write(&paths.1, serde_json::to_vec_pretty(&identity).unwrap()).unwrap();
        paths
    }

    #[test]
    fn interrupted_transfer_resumes_from_the_durable_partial_offset() {
        let root = TempTree::new("resume");
        let target = root.0.join("artifact.bin");
        let (endpoint, server) = serve(vec![
            Reply::ok(&BODY[..6], BODY.len()),
            Reply::partial(&BODY[6..], "bytes 6-15/16"),
        ]);
        let mut downloaded = 0;

        provider(endpoint)
            .ensure_file(
                &descriptor(),
                "artifact.bin",
                BODY.len() as u64,
                BODY_SHA256,
                &target,
                &mut downloaded,
            )
            .unwrap();

        let requests = server.join().unwrap();
        assert!(!requests[0].contains("Range:"));
        assert!(requests[1].contains("range: bytes=6-") || requests[1].contains("Range: bytes=6-"));
        assert_eq!(fs::read(&target).unwrap(), BODY);
        assert_eq!(downloaded, BODY.len() as u64);
        let partial = partial_fixture_paths(&target, "artifact.bin");
        assert!(!partial.0.exists());
        assert!(!partial.1.exists());
    }

    #[test]
    fn completed_verified_partial_is_published_without_another_request() {
        let root = TempTree::new("completed-partial");
        let target = root.0.join("artifact.bin");
        fs::write(&target, b"fedcba9876543210").unwrap();
        seed_partial(&target, "artifact.bin", BODY);
        let mut downloaded = 0;

        provider("http://127.0.0.1:1".to_owned())
            .ensure_file(
                &descriptor(),
                "artifact.bin",
                BODY.len() as u64,
                BODY_SHA256,
                &target,
                &mut downloaded,
            )
            .unwrap();

        assert_eq!(fs::read(target).unwrap(), BODY);
        assert_eq!(downloaded, 0);
    }

    #[test]
    fn publication_failure_preserves_the_previous_generation_and_verified_partial() {
        let root = TempTree::new("publication-failure");
        let target = root.0.join("artifact.bin");
        fs::write(&target, b"previous-accepted").unwrap();
        let partial = root.0.join("verified.download");
        fs::write(&partial, BODY).unwrap();
        let previous = publish_model_set_snapshot(
            &root.0,
            ProductionModelRole::ALL.into_iter().map(marker).collect(),
        )
        .unwrap();
        let error = publish_verified_partial_with(&partial, &target, |old, replacement| {
            assert_eq!(fs::read(old).unwrap(), b"previous-accepted");
            assert_eq!(fs::read(replacement).unwrap(), BODY);
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected replacement-boundary failure",
            ))
        })
        .unwrap_err();

        assert!(error.to_string().contains("replacement-boundary failure"));
        assert_eq!(fs::read(&target).unwrap(), b"previous-accepted");
        assert_eq!(fs::read(&partial).unwrap(), BODY);
        assert_eq!(read_model_set_snapshot(&root.0).unwrap(), previous);
    }

    #[test]
    fn wrong_content_range_exhausts_bounded_retries_without_publication() {
        let root = TempTree::new("wrong-range");
        let target = root.0.join("artifact.bin");
        let (partial, _) = seed_partial(&target, "artifact.bin", &BODY[..4]);
        let replies = (0..3)
            .map(|_| Reply::partial(&BODY[4..], "bytes 0-11/16"))
            .collect();
        let (endpoint, server) = serve(replies);
        let previous = publish_model_set_snapshot(
            &root.0,
            ProductionModelRole::ALL.into_iter().map(marker).collect(),
        )
        .unwrap();
        let mut downloaded = 0;

        let error = provider(endpoint)
            .ensure_file(
                &descriptor(),
                "artifact.bin",
                BODY.len() as u64,
                BODY_SHA256,
                &target,
                &mut downloaded,
            )
            .unwrap_err();

        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(error.to_string().contains("unexpected Content-Range"));
        assert!(!target.exists());
        assert_eq!(fs::read(&partial).unwrap(), &BODY[..4]);
        assert_eq!(read_model_set_snapshot(&root.0).unwrap(), previous);
        assert_eq!(downloaded, 0);
    }

    #[test]
    fn cross_artifact_partial_identity_never_drives_a_range_request() {
        let root = TempTree::new("cross-artifact");
        let onnx_target = root.0.join("model.onnx");
        let data_target = root.0.join("model.onnx_data");
        let onnx_identity = PartialArtifactIdentity::new(
            &descriptor(),
            "model.onnx",
            BODY.len() as u64,
            BODY_SHA256,
        );
        let data_identity = PartialArtifactIdentity::new(
            &descriptor(),
            "model.onnx_data",
            BODY.len() as u64,
            BODY_SHA256,
        );
        let onnx_paths = partial_state_paths(&onnx_target, &onnx_identity).unwrap();
        let data_paths = partial_state_paths(&data_target, &data_identity).unwrap();
        assert_ne!(onnx_paths.0, data_paths.0);
        assert!(
            onnx_paths
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("model.onnx.fastsearch-")
        );
        assert!(
            data_paths
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("model.onnx_data.fastsearch-")
        );

        // Simulate stale bytes placed under the intended path but carrying the
        // other official ONNX artifact identity.
        fs::write(&data_paths.0, &BODY[..4]).unwrap();
        fs::write(
            &data_paths.1,
            serde_json::to_vec_pretty(&onnx_identity).unwrap(),
        )
        .unwrap();
        let (endpoint, server) = serve(vec![Reply::ok(BODY, BODY.len())]);
        let mut downloaded = 0;

        provider(endpoint)
            .ensure_file(
                &descriptor(),
                "model.onnx_data",
                BODY.len() as u64,
                BODY_SHA256,
                &data_target,
                &mut downloaded,
            )
            .unwrap();

        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].contains("Range:"));
        assert_eq!(fs::read(data_target).unwrap(), BODY);
        assert_eq!(downloaded, BODY.len() as u64);
    }

    #[test]
    fn oversize_http_bodies_are_discarded_and_never_publish() {
        let root = TempTree::new("oversize");
        let target = root.0.join("artifact.bin");
        let previous = publish_model_set_snapshot(
            &root.0,
            ProductionModelRole::ALL.into_iter().map(marker).collect(),
        )
        .unwrap();
        let mut oversize = BODY.to_vec();
        oversize.push(b'x');
        let (endpoint, server) = serve(
            (0..3)
                .map(|_| Reply::ok(&oversize, oversize.len()))
                .collect(),
        );
        let mut downloaded = 0;

        let error = provider(endpoint)
            .ensure_file(
                &descriptor(),
                "artifact.bin",
                BODY.len() as u64,
                BODY_SHA256,
                &target,
                &mut downloaded,
            )
            .unwrap_err();

        assert_eq!(server.join().unwrap().len(), 3);
        assert!(
            error
                .to_string()
                .contains("exceeds the admitted artifact length")
        );
        let partial = partial_fixture_paths(&target, "artifact.bin");
        assert!(!target.exists());
        assert!(!partial.0.exists());
        assert!(!partial.1.exists());
        assert_eq!(read_model_set_snapshot(&root.0).unwrap(), previous);
        assert_eq!(downloaded, 0);
    }

    #[test]
    fn wrong_sha_and_http_failures_do_not_publish_a_target() {
        let root = TempTree::new("negative-matrix");
        let target = root.0.join("artifact.bin");
        let wrong = b"fedcba9876543210";
        let (endpoint, server) = serve((0..3).map(|_| Reply::ok(wrong, wrong.len())).collect());
        let mut downloaded = 0;
        let error = provider(endpoint)
            .ensure_file(
                &descriptor(),
                "artifact.bin",
                BODY.len() as u64,
                BODY_SHA256,
                &target,
                &mut downloaded,
            )
            .unwrap_err();
        server.join().unwrap();
        assert!(error.to_string().contains("SHA-256 mismatch"));
        assert!(!target.exists());
        let partial = partial_fixture_paths(&target, "artifact.bin");
        assert!(!partial.0.exists());
        assert!(!partial.1.exists());

        let (endpoint, server) = serve((0..3).map(|_| Reply::failure()).collect());
        let error = provider(endpoint)
            .ensure_file(
                &descriptor(),
                "artifact.bin",
                BODY.len() as u64,
                BODY_SHA256,
                &target,
                &mut downloaded,
            )
            .unwrap_err();
        server.join().unwrap();
        assert!(error.to_string().contains("got 503"));
        assert!(!target.exists());
    }

    #[test]
    fn short_response_is_retained_only_as_a_non_published_partial() {
        let root = TempTree::new("short-response");
        let target = root.0.join("artifact.bin");
        let (endpoint, server) = serve(vec![
            Reply::ok(&BODY[..15], 15),
            Reply::failure(),
            Reply::failure(),
        ]);
        let mut downloaded = 0;

        provider(endpoint)
            .ensure_file(
                &descriptor(),
                "artifact.bin",
                BODY.len() as u64,
                BODY_SHA256,
                &target,
                &mut downloaded,
            )
            .unwrap_err();

        let requests = server.join().unwrap();
        assert!(
            requests[1].contains("range: bytes=15-") || requests[1].contains("Range: bytes=15-")
        );
        assert!(!target.exists());
        assert_eq!(
            fs::read(partial_fixture_paths(&target, "artifact.bin").0).unwrap(),
            &BODY[..15]
        );
        assert_eq!(downloaded, 15);
    }

    #[test]
    fn exact_file_rejects_wrong_size_and_digest() {
        let root = TempTree::new("exact-file");
        let path = root.0.join("artifact.bin");
        fs::write(&path, &BODY[..15]).unwrap();
        assert!(!exact_file(&path, BODY.len() as u64, BODY_SHA256).unwrap());
        fs::write(&path, b"fedcba9876543210").unwrap();
        assert!(!exact_file(&path, BODY.len() as u64, BODY_SHA256).unwrap());
    }
}
