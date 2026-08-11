use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::domain::{
    CanonicalRecord, ContentHash, ErrorKind, FastSearchError, IndexFreshness, LifecycleStatus,
    RecordKind, SourceLocator, SourceSelector, StableId,
};
use crate::ports::StateStore;

/// Внутренний SQLite-владелец durable canonical records.
pub struct SqliteStateStore {
    connection: Connection,
}

impl SqliteStateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FastSearchError> {
        let connection = Connection::open(path).map_err(state_failure)?;
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
                ",
            )
            .map_err(state_failure)?;
        Ok(Self { connection })
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
}

impl StateStore for SqliteStateStore {
    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
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

    fn lifecycle_status(&self) -> LifecycleStatus {
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
