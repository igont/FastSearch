use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::domain::{
    CanonicalRecord, ContentHash, ErrorKind, FastSearchError, IndexFreshness, LifecycleStatus,
    RecordKind, SourceLocator, SourceSelector, SourceSnapshot, StableId,
};
use crate::ports::{StateChange, StateChangeSet, StateStore};

/// Внутренний SQLite-владелец durable canonical records.
pub struct SqliteStateStore {
    connection: Connection,
    mandatory_rebuild: bool,
}

impl SqliteStateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FastSearchError> {
        let connection = Connection::open(path).map_err(state_failure)?;
        let had_legacy_records = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'state_records')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(state_failure)? != 0;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(state_failure)?;
        connection
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS state_generation (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    value INTEGER NOT NULL CHECK (value >= 0)
                );
                INSERT OR IGNORE INTO state_generation (singleton, value) VALUES (1, 0);

                CREATE TABLE IF NOT EXISTS state_records (
                    id TEXT PRIMARY KEY,
                    kind INTEGER NOT NULL,
                    locator_path TEXT NOT NULL,
                    selector_kind INTEGER NOT NULL,
                    title TEXT NOT NULL,
                    searchable_content TEXT NOT NULL,
                    content_hash TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS state_selector_components (
                    record_id TEXT NOT NULL REFERENCES state_records(id) ON DELETE CASCADE,
                    position INTEGER NOT NULL CHECK (position >= 0),
                    value TEXT NOT NULL,
                    PRIMARY KEY (record_id, position)
                );
                CREATE TABLE IF NOT EXISTS state_metadata (
                    record_id TEXT NOT NULL REFERENCES state_records(id) ON DELETE CASCADE,
                    key TEXT NOT NULL,
                    value TEXT NOT NULL,
                    PRIMARY KEY (record_id, key)
                );
                CREATE TABLE IF NOT EXISTS state_relations (
                    record_id TEXT NOT NULL REFERENCES state_records(id) ON DELETE CASCADE,
                    position INTEGER NOT NULL CHECK (position >= 0),
                    related_id TEXT NOT NULL,
                    PRIMARY KEY (record_id, position)
                );
                CREATE TABLE IF NOT EXISTS state_source_snapshots (
                    source_key TEXT PRIMARY KEY,
                    file_hash TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS state_source_memberships (
                    source_key TEXT NOT NULL REFERENCES state_source_snapshots(source_key) ON DELETE CASCADE,
                    record_id TEXT NOT NULL REFERENCES state_records(id) ON DELETE CASCADE,
                    PRIMARY KEY (source_key, record_id),
                    UNIQUE (record_id)
                );
                CREATE TABLE IF NOT EXISTS state_identity_version (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    version TEXT NOT NULL
                );
                ",
            )
            .map_err(state_failure)?;
        let version = connection
            .query_row(
                "SELECT version FROM state_identity_version WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(state_failure)?;
        let mandatory_rebuild = had_legacy_records && version.is_none();
        if !mandatory_rebuild && version.is_none() {
            connection.execute(
                "INSERT INTO state_identity_version (singleton, version) VALUES (1, 'named-root-v1')",
                [],
            ).map_err(state_failure)?;
        }
        Ok(Self {
            connection,
            mandatory_rebuild,
        })
    }

    /// Записывает batch атомарно; повторяющиеся stable IDs отклоняются до открытия транзакции.
    pub fn put_all<I>(&mut self, records: I) -> Result<(), FastSearchError>
    where
        I: IntoIterator<Item = CanonicalRecord>,
    {
        let records = records.into_iter().collect::<Vec<_>>();
        let mut ids = BTreeSet::new();
        if records
            .iter()
            .any(|record| !ids.insert(record.id().as_str()))
        {
            return Err(FastSearchError::new(
                ErrorKind::DuplicateStableId,
                "input contains duplicate stable IDs",
            ));
        }

        let transaction = self.connection.transaction().map_err(state_failure)?;
        for record in &records {
            write_record(&transaction, record)?;
        }
        if !records.is_empty() {
            increment_generation(&transaction)?;
        }
        transaction.commit().map_err(state_failure)
    }

    fn generation(&self) -> Result<u64, FastSearchError> {
        let value: i64 = self
            .connection
            .query_row(
                "SELECT value FROM state_generation WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(state_failure)?;
        u64::try_from(value).map_err(|_| state_failure("stored generation is outside u64"))
    }

    fn existing_source_records(
        &self,
        source_key: &str,
    ) -> Result<BTreeMap<StableId, ContentHash>, FastSearchError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT r.id, r.content_hash
                 FROM state_source_memberships AS m
                 JOIN state_records AS r ON r.id = m.record_id
                 WHERE m.source_key = ?1
                 ORDER BY r.id",
            )
            .map_err(state_failure)?;
        statement
            .query_map([source_key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(state_failure)?
            .map(|row| {
                let (id, hash) = row.map_err(state_failure)?;
                Ok((
                    StableId::parse(id).map_err(|error| state_failure(error.message()))?,
                    ContentHash::parse(hash).map_err(|error| state_failure(error.message()))?,
                ))
            })
            .collect()
    }

    fn existing_records(&self) -> Result<BTreeMap<StableId, ContentHash>, FastSearchError> {
        let mut statement = self
            .connection
            .prepare("SELECT id, content_hash FROM state_records ORDER BY id")
            .map_err(state_failure)?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(state_failure)?
            .map(|row| {
                let (id, hash) = row.map_err(state_failure)?;
                Ok((
                    StableId::parse(id).map_err(|error| state_failure(error.message()))?,
                    ContentHash::parse(hash).map_err(|error| state_failure(error.message()))?,
                ))
            })
            .collect()
    }

    fn existing_memberships(&self) -> Result<BTreeSet<(String, StableId)>, FastSearchError> {
        let mut statement = self
            .connection
            .prepare("SELECT source_key, record_id FROM state_source_memberships ORDER BY source_key, record_id")
            .map_err(state_failure)?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(state_failure)?
            .map(|row| {
                let (source_key, record_id) = row.map_err(state_failure)?;
                StableId::parse(record_id)
                    .map(|record_id| (source_key, record_id))
                    .map_err(|error| state_failure(error.message()))
            })
            .collect()
    }

    fn reject_cross_source_ids(
        &self,
        source_key: &str,
        records: &[CanonicalRecord],
    ) -> Result<(), FastSearchError> {
        let mut statement = self
            .connection
            .prepare("SELECT source_key FROM state_source_memberships WHERE record_id = ?1")
            .map_err(state_failure)?;
        for record in records {
            let owner = statement
                .query_row([record.id().as_str()], |row| row.get::<_, String>(0))
                .optional()
                .map_err(state_failure)?;
            if owner.as_deref().is_some_and(|owner| owner != source_key) {
                return Err(FastSearchError::new(
                    ErrorKind::DuplicateStableId,
                    "snapshot record is already owned by another source locator",
                ));
            }
        }
        Ok(())
    }
}

impl StateStore for SqliteStateStore {
    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
        if self.mandatory_rebuild {
            return Ok(None);
        }
        let primary = self
            .connection
            .query_row(
                "SELECT kind, locator_path, selector_kind, title, searchable_content, content_hash
                 FROM state_records WHERE id = ?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(state_failure)?;
        let Some((kind, path, selector_kind, title, content, hash)) = primary else {
            return Ok(None);
        };

        let selector_values =
            ordered_strings(&self.connection, "state_selector_components", "value", id)?;
        let relations = ordered_strings(&self.connection, "state_relations", "related_id", id)?
            .into_iter()
            .map(StableId::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| state_failure(error.message()))?;
        let metadata = {
            let mut statement = self
                .connection
                .prepare("SELECT key, value FROM state_metadata WHERE record_id = ?1 ORDER BY key")
                .map_err(state_failure)?;
            statement
                .query_map([id.as_str()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(state_failure)?
                .collect::<Result<_, _>>()
                .map_err(state_failure)?
        };

        let selector = read_selector(selector_kind, selector_values)?;
        // Конструкторы локатора сохраняют domain validation при реконструкции SQLite данных.
        let locator = match selector {
            SourceSelector::MarkdownHeading { heading_path } => {
                SourceLocator::markdown(path, heading_path)
            }
            SourceSelector::RegistryRow { row } => SourceLocator::registry_row(path, row),
            SourceSelector::CodeSymbol { symbol } => SourceLocator::code_symbol(path, symbol),
            SourceSelector::WholeFile => SourceLocator::whole_file(path),
        }
        .map_err(|error| state_failure(error.message()))?;
        CanonicalRecord::new(
            id.clone(),
            read_kind(kind)?,
            locator,
            title,
            content,
            metadata,
            relations,
            ContentHash::parse(hash).map_err(|error| state_failure(error.message()))?,
        )
        .map(Some)
        .map_err(|error| state_failure(error.message()))
    }

    fn put(&mut self, record: CanonicalRecord) -> Result<(), FastSearchError> {
        self.put_all([record])
    }

    fn remove(&mut self, id: &StableId) -> Result<bool, FastSearchError> {
        let transaction = self.connection.transaction().map_err(state_failure)?;
        let removed = transaction
            .execute("DELETE FROM state_records WHERE id = ?1", [id.as_str()])
            .map_err(state_failure)?
            != 0;
        if removed {
            increment_generation(&transaction)?;
        }
        transaction.commit().map_err(state_failure)?;
        Ok(removed)
    }

    fn apply_snapshot(
        &mut self,
        snapshot: SourceSnapshot,
    ) -> Result<StateChangeSet, FastSearchError> {
        let records = snapshot.records();
        let mut ids = BTreeSet::new();
        if records
            .iter()
            .any(|record| !ids.insert(record.id().as_str()))
        {
            return Err(FastSearchError::new(
                ErrorKind::DuplicateStableId,
                "snapshot contains duplicate stable IDs",
            ));
        }

        let source_key = source_key(snapshot.locator());
        self.reject_cross_source_ids(&source_key, records)?;
        let previous = self.existing_source_records(&source_key)?;
        let current_ids = records
            .iter()
            .map(|record| record.id().clone())
            .collect::<BTreeSet<_>>();
        let mut changes = records
            .iter()
            .map(|record| match previous.get(record.id()) {
                None => StateChange::Added,
                Some(hash) if hash == record.content_hash() => StateChange::Unchanged,
                Some(_) => StateChange::Changed,
            })
            .collect::<Vec<_>>();
        changes.extend(
            previous
                .keys()
                .filter(|id| !current_ids.contains(*id))
                .map(|_| StateChange::Deleted),
        );
        let logical_change = changes
            .iter()
            .any(|change| *change != StateChange::Unchanged);

        let transaction = self.connection.transaction().map_err(state_failure)?;
        transaction
            .execute(
                "INSERT INTO state_source_snapshots (source_key, file_hash) VALUES (?1, ?2)
                 ON CONFLICT(source_key) DO UPDATE SET file_hash = excluded.file_hash",
                params![source_key, snapshot.file_hash().as_str()],
            )
            .map_err(state_failure)?;
        for id in previous.keys() {
            transaction
                .execute("DELETE FROM state_records WHERE id = ?1", [id.as_str()])
                .map_err(state_failure)?;
        }
        for record in records {
            write_record(&transaction, record)?;
            transaction
                .execute(
                    "INSERT INTO state_source_memberships (source_key, record_id) VALUES (?1, ?2)",
                    params![source_key, record.id().as_str()],
                )
                .map_err(state_failure)?;
        }
        if logical_change {
            increment_generation(&transaction)?;
        }
        transaction.commit().map_err(state_failure)?;
        Ok(StateChangeSet::new(changes, self.generation()?))
    }

    fn reconcile_snapshots(
        &mut self,
        snapshots: &[SourceSnapshot],
    ) -> Result<StateChangeSet, FastSearchError> {
        let mut source_keys = BTreeSet::new();
        let mut incoming = BTreeMap::new();
        let mut incoming_memberships = BTreeSet::new();
        for snapshot in snapshots {
            let source_key = source_key(snapshot.locator());
            if !source_keys.insert(source_key.clone()) {
                return Err(FastSearchError::new(
                    ErrorKind::StateFailure,
                    "complete scan contains duplicate source locators",
                ));
            }
            for record in snapshot.records() {
                if incoming.insert(record.id().clone(), record).is_some() {
                    return Err(FastSearchError::new(
                        ErrorKind::DuplicateStableId,
                        "complete scan contains duplicate stable IDs",
                    ));
                }
                incoming_memberships.insert((source_key.clone(), record.id().clone()));
            }
        }

        let existing = self.existing_records()?;
        let existing_memberships = self.existing_memberships()?;
        let mut changes = Vec::with_capacity(incoming.len() + existing.len());
        for snapshot in snapshots {
            for record in snapshot.records() {
                changes.push(match existing.get(record.id()) {
                    None => StateChange::Added,
                    Some(hash) if hash == record.content_hash() => StateChange::Unchanged,
                    Some(_) => StateChange::Changed,
                });
            }
        }
        changes.extend(
            existing
                .keys()
                .filter(|id| !incoming.contains_key(*id))
                .map(|_| StateChange::Deleted),
        );
        let logical_change = changes
            .iter()
            .any(|change| *change != StateChange::Unchanged)
            || existing_memberships != incoming_memberships;

        let transaction = self.connection.transaction().map_err(state_failure)?;
        transaction
            .execute("DELETE FROM state_source_snapshots", [])
            .map_err(state_failure)?;
        transaction
            .execute("DELETE FROM state_records", [])
            .map_err(state_failure)?;
        for snapshot in snapshots {
            let source_key = source_key(snapshot.locator());
            transaction
                .execute(
                    "INSERT INTO state_source_snapshots (source_key, file_hash) VALUES (?1, ?2)",
                    params![source_key, snapshot.file_hash().as_str()],
                )
                .map_err(state_failure)?;
            for record in snapshot.records() {
                write_record(&transaction, record)?;
                transaction
                    .execute(
                        "INSERT INTO state_source_memberships (source_key, record_id) VALUES (?1, ?2)",
                        params![source_key, record.id().as_str()],
                    )
                    .map_err(state_failure)?;
            }
        }
        if logical_change {
            increment_generation(&transaction)?;
        }
        transaction.commit().map_err(state_failure)?;
        if self.mandatory_rebuild {
            self.connection.execute(
                "INSERT INTO state_identity_version (singleton, version) VALUES (1, 'named-root-v1')\n                 ON CONFLICT(singleton) DO UPDATE SET version = excluded.version",
                [],
            ).map_err(state_failure)?;
            self.mandatory_rebuild = false;
        }
        Ok(StateChangeSet::new(changes, self.generation()?))
    }

    fn lifecycle_status(&self) -> LifecycleStatus {
        if self.mandatory_rebuild {
            return LifecycleStatus::new(
                IndexFreshness::Stale,
                0,
                None,
                "legacy DT2 state requires named-root-v1 rebuild",
            );
        }
        match self.generation() {
            Ok(generation) => LifecycleStatus::new(
                IndexFreshness::Stale,
                generation,
                None,
                "SQLite state has no lexical projection",
            ),
            Err(error) => LifecycleStatus::new(IndexFreshness::Degraded, 0, None, error.message()),
        }
    }
}

fn write_record(
    transaction: &Transaction<'_>,
    record: &CanonicalRecord,
) -> Result<(), FastSearchError> {
    transaction.execute(
        "INSERT INTO state_records (id, kind, locator_path, selector_kind, title, searchable_content, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, locator_path = excluded.locator_path,
           selector_kind = excluded.selector_kind, title = excluded.title,
           searchable_content = excluded.searchable_content, content_hash = excluded.content_hash",
        params![record.id().as_str(), kind_value(record.kind()), record.locator().path(), selector_kind(record.locator().selector()), record.title(), record.searchable_content(), record.content_hash().as_str()],
    ).map_err(state_failure)?;
    transaction
        .execute(
            "DELETE FROM state_selector_components WHERE record_id = ?1",
            [record.id().as_str()],
        )
        .map_err(state_failure)?;
    transaction
        .execute(
            "DELETE FROM state_metadata WHERE record_id = ?1",
            [record.id().as_str()],
        )
        .map_err(state_failure)?;
    transaction
        .execute(
            "DELETE FROM state_relations WHERE record_id = ?1",
            [record.id().as_str()],
        )
        .map_err(state_failure)?;

    let selector_components = match record.locator().selector() {
        SourceSelector::MarkdownHeading { heading_path } => heading_path.clone(),
        SourceSelector::RegistryRow { row } => vec![row.get().to_string()],
        SourceSelector::CodeSymbol { symbol } => vec![symbol.clone()],
        SourceSelector::WholeFile => Vec::new(),
    };
    for (position, value) in selector_components.iter().enumerate() {
        let position = storage_position(position)?;
        transaction.execute("INSERT INTO state_selector_components (record_id, position, value) VALUES (?1, ?2, ?3)", params![record.id().as_str(), position, value]).map_err(state_failure)?;
    }
    for (key, value) in record.metadata() {
        transaction
            .execute(
                "INSERT INTO state_metadata (record_id, key, value) VALUES (?1, ?2, ?3)",
                params![record.id().as_str(), key, value],
            )
            .map_err(state_failure)?;
    }
    for (position, relation) in record.relations().iter().enumerate() {
        let position = storage_position(position)?;
        transaction
            .execute(
                "INSERT INTO state_relations (record_id, position, related_id) VALUES (?1, ?2, ?3)",
                params![record.id().as_str(), position, relation.as_str()],
            )
            .map_err(state_failure)?;
    }
    Ok(())
}

fn ordered_strings(
    connection: &Connection,
    table: &str,
    column: &str,
    id: &StableId,
) -> Result<Vec<String>, FastSearchError> {
    let query = format!("SELECT {column} FROM {table} WHERE record_id = ?1 ORDER BY position");
    let mut statement = connection.prepare(&query).map_err(state_failure)?;
    statement
        .query_map([id.as_str()], |row| row.get(0))
        .map_err(state_failure)?
        .collect::<Result<_, _>>()
        .map_err(state_failure)
}

fn increment_generation(transaction: &Transaction<'_>) -> Result<(), FastSearchError> {
    transaction
        .execute(
            "UPDATE state_generation SET value = value + 1 WHERE singleton = 1",
            [],
        )
        .map_err(state_failure)?;
    Ok(())
}

fn kind_value(kind: RecordKind) -> i64 {
    match kind {
        RecordKind::MarkdownSection => 1,
        RecordKind::RegistryRow => 2,
        RecordKind::CodeMap => 3,
        RecordKind::CodeSymbol => 4,
    }
}
fn read_kind(value: i64) -> Result<RecordKind, FastSearchError> {
    match value {
        1 => Ok(RecordKind::MarkdownSection),
        2 => Ok(RecordKind::RegistryRow),
        3 => Ok(RecordKind::CodeMap),
        4 => Ok(RecordKind::CodeSymbol),
        _ => Err(state_failure("stored record kind is invalid")),
    }
}
fn selector_kind(selector: &SourceSelector) -> i64 {
    match selector {
        SourceSelector::MarkdownHeading { .. } => 1,
        SourceSelector::RegistryRow { .. } => 2,
        SourceSelector::CodeSymbol { .. } => 3,
        SourceSelector::WholeFile => 4,
    }
}
fn read_selector(kind: i64, values: Vec<String>) -> Result<SourceSelector, FastSearchError> {
    match kind {
        1 if !values.is_empty() => Ok(SourceSelector::MarkdownHeading {
            heading_path: values,
        }),
        2 if values.len() == 1 => values[0]
            .parse()
            .ok()
            .and_then(std::num::NonZeroUsize::new)
            .map(|row| SourceSelector::RegistryRow { row })
            .ok_or_else(|| state_failure("stored registry row is invalid")),
        3 if values.len() == 1 => values
            .into_iter()
            .next()
            .map(|symbol| SourceSelector::CodeSymbol { symbol })
            .ok_or_else(|| state_failure("stored code symbol is missing")),
        4 if values.is_empty() => Ok(SourceSelector::WholeFile),
        _ => Err(state_failure("stored selector components are invalid")),
    }
}
fn state_failure(error: impl std::fmt::Display) -> FastSearchError {
    FastSearchError::new(ErrorKind::StateFailure, error.to_string())
}

fn storage_position(position: usize) -> Result<i64, FastSearchError> {
    i64::try_from(position)
        .map_err(|_| state_failure("record component position exceeds SQLite range"))
}

fn source_key(locator: &SourceLocator) -> String {
    fn append_component(key: &mut String, value: &str) {
        key.push_str(&value.len().to_string());
        key.push(':');
        key.push_str(value);
    }

    let mut key = String::new();
    append_component(&mut key, locator.path());
    match locator.selector() {
        SourceSelector::MarkdownHeading { heading_path } => {
            key.push('M');
            for heading in heading_path {
                append_component(&mut key, heading);
            }
        }
        SourceSelector::RegistryRow { row } => {
            key.push('R');
            append_component(&mut key, &row.get().to_string());
        }
        SourceSelector::CodeSymbol { symbol } => {
            key.push('C');
            append_component(&mut key, symbol);
        }
        SourceSelector::WholeFile => key.push('F'),
    }
    key
}
