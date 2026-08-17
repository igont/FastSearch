//! Durable, rebuildable storage for one model/revision vector projection.
//!
//! Canonical records remain owned by the shared SQLite state.  A partition
//! stores only stable IDs, content hashes and normalized vectors, then joins
//! them back to the current canonical snapshot during admission.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use fs2::FileExt;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{CanonicalRecord, EmbeddingModelId, ErrorKind, FastSearchError};

use super::ProjectedRecord;

pub(super) const VECTOR_RUNTIME_CONTRACT: &str = "fastsearch-vector-partition-v1-fastembed-5.17.4";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct PartitionManifest {
    pub schema_version: u32,
    pub model_slug: String,
    pub model_identity: String,
    pub artifact_manifest: String,
    pub runtime_contract: String,
    pub dimension: usize,
    pub state_generation: u64,
    pub corpus_fingerprint: String,
    pub record_count: usize,
    #[serde(default)]
    pub build_duration_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StoredPartitionMetrics {
    pub size_bytes: u64,
    pub build_duration_ms: u64,
}

pub(super) struct LoadedPartition {
    pub manifest: PartitionManifest,
    pub records: BTreeMap<String, ProjectedRecord>,
}

pub(super) struct ExpectedPartition<'a> {
    pub model_id: EmbeddingModelId,
    pub model_identity: &'a str,
    pub state_generation: u64,
    pub records: &'a [CanonicalRecord],
}

const CHECKPOINT_SCHEMA_VERSION: u32 = 1;
const CHECKPOINT_MANIFEST: &str = "checkpoint.toml";
const CHECKPOINT_VECTORS: &str = "checkpoint-vectors.bin";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct CheckpointManifest {
    schema_version: u32,
    model_slug: String,
    model_identity: String,
    artifact_manifest: String,
    runtime_contract: String,
    dimension: usize,
    state_generation: u64,
    corpus_fingerprint: String,
    record_order_fingerprint: String,
    record_count: usize,
    completed_records: usize,
}

pub(super) fn load_checkpoint(
    root: &Path,
    expected: &PartitionManifest,
    records: &[CanonicalRecord],
) -> Result<Vec<Vec<f32>>, FastSearchError> {
    let manifest_path = root.join(CHECKPOINT_MANIFEST);
    if !manifest_path.is_file() {
        return Ok(Vec::new());
    }
    let lock = open_lock(root)?;
    FileExt::lock_exclusive(&lock).map_err(partition_failure)?;
    let loaded = try_load_checkpoint(root, expected, records);
    match loaded {
        Ok(Some(vectors)) => Ok(vectors),
        Ok(None) | Err(_) => {
            remove_checkpoint_files(root)?;
            Ok(Vec::new())
        }
    }
}

pub(super) fn append_checkpoint(
    root: &Path,
    expected: &PartitionManifest,
    records: &[CanonicalRecord],
    completed_before: usize,
    chunk: &[Vec<f32>],
) -> Result<(), FastSearchError> {
    if chunk.is_empty() {
        return Ok(());
    }
    if completed_before > records.len()
        || chunk.len() > records.len().saturating_sub(completed_before)
        || chunk.iter().any(|vector| {
            vector.len() != expected.dimension || vector.iter().any(|value| !value.is_finite())
        })
    {
        return Err(partition_failure("checkpoint chunk is inconsistent"));
    }
    fs::create_dir_all(root).map_err(partition_failure)?;
    let lock = open_lock(root)?;
    FileExt::lock_exclusive(&lock).map_err(partition_failure)?;

    if completed_before == 0 {
        remove_checkpoint_files(root)?;
    } else {
        let stored = read_checkpoint_manifest(root)?
            .ok_or_else(|| partition_failure("checkpoint manifest is missing"))?;
        if !checkpoint_matches(&stored, expected, records)
            || stored.completed_records != completed_before
        {
            return Err(partition_failure(
                "checkpoint changed while vectorization was running",
            ));
        }
    }

    let vector_path = root.join(CHECKPOINT_VECTORS);
    let expected_bytes = checkpoint_vector_bytes(completed_before, expected.dimension)?;
    let mut vectors = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&vector_path)
        .map_err(partition_failure)?;
    if vectors.metadata().map_err(partition_failure)?.len() < expected_bytes {
        return Err(partition_failure("checkpoint vector file is truncated"));
    }
    vectors.set_len(expected_bytes).map_err(partition_failure)?;
    vectors.seek(SeekFrom::End(0)).map_err(partition_failure)?;
    for vector in chunk {
        for value in vector {
            vectors
                .write_all(&value.to_le_bytes())
                .map_err(partition_failure)?;
        }
    }
    vectors.sync_all().map_err(partition_failure)?;

    let checkpoint = CheckpointManifest {
        schema_version: CHECKPOINT_SCHEMA_VERSION,
        model_slug: expected.model_slug.clone(),
        model_identity: expected.model_identity.clone(),
        artifact_manifest: expected.artifact_manifest.clone(),
        runtime_contract: expected.runtime_contract.clone(),
        dimension: expected.dimension,
        state_generation: expected.state_generation,
        corpus_fingerprint: expected.corpus_fingerprint.clone(),
        record_order_fingerprint: record_order_fingerprint(records),
        record_count: expected.record_count,
        completed_records: completed_before + chunk.len(),
    };
    let encoded = toml::to_string_pretty(&checkpoint).map_err(partition_failure)?;
    let manifest_path = root.join(CHECKPOINT_MANIFEST);
    let temporary = temporary_path(&manifest_path);
    remove_if_exists(&temporary)?;
    let mut manifest_file = File::create(&temporary).map_err(partition_failure)?;
    manifest_file
        .write_all(encoded.as_bytes())
        .and_then(|()| manifest_file.sync_all())
        .map_err(partition_failure)?;
    replace_file(&temporary, &manifest_path)
}

pub(super) fn clear_checkpoint(root: &Path) -> Result<(), FastSearchError> {
    let lock = open_lock(root)?;
    FileExt::lock_exclusive(&lock).map_err(partition_failure)?;
    remove_checkpoint_files(root)
}

fn try_load_checkpoint(
    root: &Path,
    expected: &PartitionManifest,
    records: &[CanonicalRecord],
) -> Result<Option<Vec<Vec<f32>>>, FastSearchError> {
    let Some(manifest) = read_checkpoint_manifest(root)? else {
        return Ok(None);
    };
    if !checkpoint_matches(&manifest, expected, records) {
        return Ok(None);
    }
    let expected_bytes = checkpoint_vector_bytes(manifest.completed_records, manifest.dimension)?;
    let vector_path = root.join(CHECKPOINT_VECTORS);
    if fs::metadata(&vector_path).map_err(partition_failure)?.len() < expected_bytes {
        return Ok(None);
    }
    let mut reader = BufReader::new(File::open(vector_path).map_err(partition_failure)?);
    let mut vectors = Vec::with_capacity(manifest.completed_records);
    for _ in 0..manifest.completed_records {
        let mut vector = Vec::with_capacity(manifest.dimension);
        for _ in 0..manifest.dimension {
            let mut bytes = [0_u8; 4];
            reader.read_exact(&mut bytes).map_err(partition_failure)?;
            let value = f32::from_le_bytes(bytes);
            if !value.is_finite() {
                return Ok(None);
            }
            vector.push(value);
        }
        vectors.push(vector);
    }
    Ok(Some(vectors))
}

fn read_checkpoint_manifest(root: &Path) -> Result<Option<CheckpointManifest>, FastSearchError> {
    let path = root.join(CHECKPOINT_MANIFEST);
    if !path.is_file() {
        return Ok(None);
    }
    toml::from_str(&fs::read_to_string(path).map_err(partition_failure)?)
        .map(Some)
        .map_err(partition_failure)
}

fn checkpoint_matches(
    actual: &CheckpointManifest,
    expected: &PartitionManifest,
    records: &[CanonicalRecord],
) -> bool {
    actual.schema_version == CHECKPOINT_SCHEMA_VERSION
        && actual.model_slug == expected.model_slug
        && actual.model_identity == expected.model_identity
        && actual.artifact_manifest == expected.artifact_manifest
        && actual.runtime_contract == expected.runtime_contract
        && actual.dimension == expected.dimension
        && actual.state_generation == expected.state_generation
        && actual.corpus_fingerprint == expected.corpus_fingerprint
        && actual.record_order_fingerprint == record_order_fingerprint(records)
        && actual.record_count == expected.record_count
        && actual.completed_records <= actual.record_count
}

fn checkpoint_vector_bytes(records: usize, dimension: usize) -> Result<u64, FastSearchError> {
    records
        .checked_mul(dimension)
        .and_then(|value| value.checked_mul(4))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| partition_failure("checkpoint vector file size overflow"))
}

fn record_order_fingerprint(records: &[CanonicalRecord]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"fastsearch-corpus-order-v1\0");
    for record in records {
        digest.update(record.id().as_str().len().to_le_bytes());
        digest.update(record.id().as_str().as_bytes());
        digest.update(record.content_hash().as_str().len().to_le_bytes());
        digest.update(record.content_hash().as_str().as_bytes());
    }
    format!("{:X}", digest.finalize())
}

fn remove_checkpoint_files(root: &Path) -> Result<(), FastSearchError> {
    remove_if_exists(&root.join(CHECKPOINT_MANIFEST))?;
    remove_if_exists(&root.join(CHECKPOINT_VECTORS))
}

pub(super) fn save(
    root: &Path,
    manifest: &PartitionManifest,
    records: &BTreeMap<String, ProjectedRecord>,
    build_started: Instant,
) -> Result<(), FastSearchError> {
    fs::create_dir_all(root).map_err(partition_failure)?;
    let lock = open_lock(root)?;
    FileExt::lock_exclusive(&lock).map_err(partition_failure)?;

    let vectors_target = root.join("vectors.bin");
    let records_target = root.join("records.sqlite");
    let manifest_target = root.join("manifest.toml");
    let vectors_temporary = temporary_path(&vectors_target);
    let records_temporary = temporary_path(&records_target);
    let manifest_temporary = temporary_path(&manifest_target);
    remove_if_exists(&vectors_temporary)?;
    remove_if_exists(&records_temporary)?;
    remove_if_exists(&manifest_temporary)?;

    let mut vectors = BufWriter::new(File::create(&vectors_temporary).map_err(partition_failure)?);
    let connection = Connection::open(&records_temporary).map_err(partition_failure)?;
    connection
        .execute_batch(
            "
            PRAGMA journal_mode = OFF;
            PRAGMA synchronous = FULL;
            CREATE TABLE projection_records (
                position INTEGER PRIMARY KEY CHECK (position >= 0),
                record_id TEXT NOT NULL UNIQUE,
                content_hash TEXT NOT NULL,
                offset_bytes INTEGER NOT NULL CHECK (offset_bytes >= 0),
                dimension INTEGER NOT NULL CHECK (dimension > 0)
            );
            ",
        )
        .map_err(partition_failure)?;

    let transaction = connection
        .unchecked_transaction()
        .map_err(partition_failure)?;
    let mut offset = 0_u64;
    for (position, (id, projected)) in records.iter().enumerate() {
        if projected.vector.len() != manifest.dimension
            || projected.vector.iter().any(|value| !value.is_finite())
        {
            return Err(partition_failure(
                "vector partition contains an invalid vector",
            ));
        }
        for value in &projected.vector {
            vectors
                .write_all(&value.to_le_bytes())
                .map_err(partition_failure)?;
        }
        transaction
            .execute(
                "INSERT INTO projection_records
                 (position, record_id, content_hash, offset_bytes, dimension)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    i64::try_from(position).map_err(partition_failure)?,
                    id,
                    projected.content_hash,
                    i64::try_from(offset).map_err(partition_failure)?,
                    i64::try_from(manifest.dimension).map_err(partition_failure)?,
                ],
            )
            .map_err(partition_failure)?;
        offset = offset
            .checked_add(
                u64::try_from(manifest.dimension)
                    .map_err(partition_failure)?
                    .checked_mul(4)
                    .ok_or_else(|| partition_failure("vector byte size overflow"))?,
            )
            .ok_or_else(|| partition_failure("vector file offset overflow"))?;
    }
    transaction.commit().map_err(partition_failure)?;
    connection
        .close()
        .map_err(|(_, error)| partition_failure(error))?;
    vectors.flush().map_err(partition_failure)?;
    vectors.get_ref().sync_all().map_err(partition_failure)?;

    let mut committed_manifest = manifest.clone();
    committed_manifest.build_duration_ms =
        Some(u64::try_from(build_started.elapsed().as_millis()).unwrap_or(u64::MAX));
    let encoded = toml::to_string_pretty(&committed_manifest).map_err(partition_failure)?;
    let mut manifest_file = File::create(&manifest_temporary).map_err(partition_failure)?;
    manifest_file
        .write_all(encoded.as_bytes())
        .and_then(|()| manifest_file.sync_all())
        .map_err(partition_failure)?;

    // The manifest is the commit marker and is always published last.
    remove_if_exists(&manifest_target)?;
    replace_file(&vectors_temporary, &vectors_target)?;
    replace_file(&records_temporary, &records_target)?;
    replace_file(&manifest_temporary, &manifest_target)?;
    Ok(())
}

pub(super) fn load(
    root: &Path,
    expected: ExpectedPartition<'_>,
) -> Result<Option<LoadedPartition>, FastSearchError> {
    let manifest_path = root.join("manifest.toml");
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let lock = open_lock(root)?;
    FileExt::lock_shared(&lock).map_err(partition_failure)?;
    let manifest_text = fs::read_to_string(&manifest_path).map_err(partition_failure)?;
    let manifest: PartitionManifest = toml::from_str(&manifest_text).map_err(partition_failure)?;
    let fingerprint = corpus_fingerprint(expected.records);
    if manifest.schema_version != 2
        || manifest.model_slug != expected.model_id.slug()
        || manifest.model_identity != expected.model_identity
        || manifest.runtime_contract != VECTOR_RUNTIME_CONTRACT
        || manifest.dimension != expected.model_id.dimension()
        || manifest.state_generation != expected.state_generation
        || manifest.corpus_fingerprint != fingerprint
        || manifest.record_count != expected.records.len()
        || manifest.build_duration_ms.is_none()
    {
        return Ok(None);
    }

    let canonical = expected
        .records
        .iter()
        .map(|record| (record.id().as_str(), record))
        .collect::<BTreeMap<_, _>>();
    let connection = Connection::open_with_flags(
        root.join("records.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(partition_failure)?;
    let mut statement = connection
        .prepare(
            "SELECT position, record_id, content_hash, offset_bytes, dimension
             FROM projection_records ORDER BY position",
        )
        .map_err(partition_failure)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(partition_failure)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(partition_failure)?;
    drop(statement);
    drop(connection);
    if rows.len() != manifest.record_count {
        return Err(partition_failure("partition record count is inconsistent"));
    }

    let expected_bytes = manifest
        .record_count
        .checked_mul(manifest.dimension)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| partition_failure("partition vector file size overflow"))?;
    let vectors_path = root.join("vectors.bin");
    if fs::metadata(&vectors_path)
        .map_err(partition_failure)?
        .len()
        != u64::try_from(expected_bytes).map_err(partition_failure)?
    {
        return Err(partition_failure(
            "partition vector file size is inconsistent",
        ));
    }
    let mut vectors = BufReader::new(File::open(vectors_path).map_err(partition_failure)?);
    let mut projected = BTreeMap::new();
    for (expected_position, (position, id, hash, offset, dimension)) in rows.into_iter().enumerate()
    {
        if position != i64::try_from(expected_position).map_err(partition_failure)?
            || offset
                != i64::try_from(expected_position * manifest.dimension * 4)
                    .map_err(partition_failure)?
            || dimension != i64::try_from(manifest.dimension).map_err(partition_failure)?
        {
            return Err(partition_failure(
                "partition record offsets are inconsistent",
            ));
        }
        let record = canonical
            .get(id.as_str())
            .ok_or_else(|| partition_failure("partition references an unknown record"))?;
        if record.content_hash().as_str() != hash {
            return Ok(None);
        }
        let mut vector = Vec::with_capacity(manifest.dimension);
        for _ in 0..manifest.dimension {
            let mut bytes = [0_u8; 4];
            vectors.read_exact(&mut bytes).map_err(partition_failure)?;
            let value = f32::from_le_bytes(bytes);
            if !value.is_finite() {
                return Err(partition_failure("partition contains a non-finite vector"));
            }
            vector.push(value);
        }
        projected.insert(
            id,
            ProjectedRecord {
                record: (*record).clone(),
                content_hash: hash,
                vector,
            },
        );
    }
    Ok(Some(LoadedPartition {
        manifest,
        records: projected,
    }))
}

pub(super) fn stored_metrics(
    root: &Path,
) -> Result<Option<StoredPartitionMetrics>, FastSearchError> {
    let manifest_path = root.join("manifest.toml");
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let lock = open_lock(root)?;
    FileExt::lock_shared(&lock).map_err(partition_failure)?;
    let manifest: PartitionManifest =
        toml::from_str(&fs::read_to_string(&manifest_path).map_err(partition_failure)?)
            .map_err(partition_failure)?;
    let Some(build_duration_ms) = manifest.build_duration_ms else {
        return Ok(None);
    };
    let mut size_bytes = 0_u64;
    for name in ["manifest.toml", "records.sqlite", "vectors.bin"] {
        size_bytes = size_bytes
            .checked_add(
                fs::metadata(root.join(name))
                    .map_err(partition_failure)?
                    .len(),
            )
            .ok_or_else(|| partition_failure("partition byte size overflow"))?;
    }
    Ok(Some(StoredPartitionMetrics {
        size_bytes,
        build_duration_ms,
    }))
}

pub(super) fn corpus_fingerprint(records: &[CanonicalRecord]) -> String {
    let mut entries = records
        .iter()
        .map(|record| (record.id().as_str(), record.content_hash().as_str()))
        .collect::<Vec<_>>();
    entries.sort_unstable();
    let mut digest = Sha256::new();
    digest.update(b"fastsearch-corpus-v1\0");
    for (id, hash) in entries {
        digest.update(id.len().to_le_bytes());
        digest.update(id.as_bytes());
        digest.update(hash.len().to_le_bytes());
        digest.update(hash.as_bytes());
    }
    format!("{:X}", digest.finalize())
}

fn open_lock(root: &Path) -> Result<File, FastSearchError> {
    let parent = root
        .parent()
        .ok_or_else(|| partition_failure("partition root has no parent"))?;
    fs::create_dir_all(parent).map_err(partition_failure)?;
    let revision = root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| partition_failure("partition revision is not valid UTF-8"))?;
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(parent.join(format!(".{revision}.lock")))
        .map_err(partition_failure)
}

fn temporary_path(target: &Path) -> PathBuf {
    PathBuf::from(format!("{}.tmp", target.display()))
}

fn replace_file(source: &Path, target: &Path) -> Result<(), FastSearchError> {
    remove_if_exists(target)?;
    fs::rename(source, target).map_err(partition_failure)
}

fn remove_if_exists(path: &Path) -> Result<(), FastSearchError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(partition_failure(error)),
    }
}

fn partition_failure(error: impl std::fmt::Display) -> FastSearchError {
    FastSearchError::new(
        ErrorKind::ProjectionFailure,
        format!("persistent vector partition failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, time::SystemTime};

    use crate::domain::{CanonicalRecord, ContentHash, RecordKind, SourceLocator, StableId};

    use super::*;

    struct TempPartition(PathBuf);
    impl TempPartition {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "fastsearch-vector-partition-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for TempPartition {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn record(id: &str, hash: &str) -> CanonicalRecord {
        CanonicalRecord::new(
            StableId::parse(id).unwrap(),
            RecordKind::MarkdownSection,
            SourceLocator::whole_file("docs/one.md").unwrap(),
            "One",
            "persistent vector record",
            BTreeMap::new(),
            Vec::new(),
            ContentHash::parse(hash).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn partition_round_trip_reopens_and_rejects_changed_corpus() {
        let temp = TempPartition::new();
        let records = vec![record("record-1", "hash-1")];
        let model = EmbeddingModelId::MultilingualE5Small;
        let identity = "intfloat/multilingual-e5-small@revision";
        let manifest = PartitionManifest {
            schema_version: 2,
            model_slug: model.slug().to_owned(),
            model_identity: identity.to_owned(),
            artifact_manifest: "A".repeat(64),
            runtime_contract: VECTOR_RUNTIME_CONTRACT.to_owned(),
            dimension: model.dimension(),
            state_generation: 7,
            corpus_fingerprint: corpus_fingerprint(&records),
            record_count: 1,
            build_duration_ms: None,
        };
        let mut projected = BTreeMap::new();
        projected.insert(
            "record-1".to_owned(),
            ProjectedRecord {
                record: records[0].clone(),
                content_hash: "hash-1".to_owned(),
                vector: vec![0.5; model.dimension()],
            },
        );
        save(&temp.0, &manifest, &projected, Instant::now()).unwrap();

        let loaded = load(
            &temp.0,
            ExpectedPartition {
                model_id: model,
                model_identity: identity,
                state_generation: 7,
                records: &records,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(loaded.manifest.schema_version, manifest.schema_version);
        assert!(loaded.manifest.build_duration_ms.is_some());
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.records["record-1"].vector.len(), model.dimension());
        let metrics = stored_metrics(&temp.0).unwrap().unwrap();
        assert!(metrics.size_bytes > u64::try_from(model.dimension() * 4).unwrap());

        let changed = vec![record("record-1", "hash-2")];
        assert!(
            load(
                &temp.0,
                ExpectedPartition {
                    model_id: model,
                    model_identity: identity,
                    state_generation: 8,
                    records: &changed,
                },
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn checkpoint_resumes_only_the_exact_model_and_ordered_corpus() {
        let temp = TempPartition::new();
        let records = vec![record("record-1", "hash-1"), record("record-2", "hash-2")];
        let model = EmbeddingModelId::MultilingualE5Small;
        let manifest = PartitionManifest {
            schema_version: 2,
            model_slug: model.slug().to_owned(),
            model_identity: "model@revision".to_owned(),
            artifact_manifest: "B".repeat(64),
            runtime_contract: VECTOR_RUNTIME_CONTRACT.to_owned(),
            dimension: model.dimension(),
            state_generation: 9,
            corpus_fingerprint: corpus_fingerprint(&records),
            record_count: records.len(),
            build_duration_ms: None,
        };
        let first = vec![vec![0.25; model.dimension()]];
        append_checkpoint(&temp.0, &manifest, &records, 0, &first).unwrap();
        let resumed = load_checkpoint(&temp.0, &manifest, &records).unwrap();
        assert_eq!(resumed, first);

        OpenOptions::new()
            .append(true)
            .open(temp.0.join(CHECKPOINT_VECTORS))
            .unwrap()
            .write_all(&123_f32.to_le_bytes())
            .unwrap();
        let second = vec![vec![0.75; model.dimension()]];
        append_checkpoint(&temp.0, &manifest, &records, 1, &second).unwrap();
        let resumed = load_checkpoint(&temp.0, &manifest, &records).unwrap();
        assert_eq!(resumed.len(), 2);
        assert_eq!(resumed[0], first[0]);
        assert_eq!(resumed[1], second[0]);

        let reordered = vec![records[1].clone(), records[0].clone()];
        assert!(
            load_checkpoint(&temp.0, &manifest, &reordered)
                .unwrap()
                .is_empty()
        );
        append_checkpoint(&temp.0, &manifest, &records, 0, &first).unwrap();
        let mut changed = manifest.clone();
        changed.state_generation += 1;
        assert!(
            load_checkpoint(&temp.0, &changed, &records)
                .unwrap()
                .is_empty()
        );
        assert!(!temp.0.join(CHECKPOINT_MANIFEST).exists());
        assert!(!temp.0.join(CHECKPOINT_VECTORS).exists());
    }
}
