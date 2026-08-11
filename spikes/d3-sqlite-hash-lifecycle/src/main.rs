use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use fastsearch::domain::{ContentHash, StableId};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug)]
struct RecordInput {
    id: StableId,
    hash: ContentHash,
    payload: String,
}

#[derive(Debug, Eq, PartialEq)]
enum Transition {
    Added,
    Unchanged,
    Changed,
    Deleted,
}

fn fixture_records() -> Result<Vec<RecordInput>, Box<dyn Error>> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/lifecycle.tsv");
    let content = fs::read_to_string(fixture)?;
    let mut lines = content.lines();
    if lines.next() != Some("stable_id\tcontent_hash\tpayload") {
        return Err("unexpected lifecycle fixture header".into());
    }
    let records = lines
        .map(|line| {
            let mut fields = line.split('\t');
            let id = StableId::parse(fields.next().ok_or("missing stable_id")?)?;
            let hash = ContentHash::parse(fields.next().ok_or("missing content_hash")?)?;
            let payload = fields.next().ok_or("missing payload")?.to_owned();
            if fields.next().is_some() {
                return Err("unexpected lifecycle fixture field".into());
            }
            Ok(RecordInput { id, hash, payload })
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    if records.len() != 3 || records.windows(2).any(|pair| pair[0].id != pair[1].id) {
        return Err("fixture must contain three payloads for one stable identity".into());
    }
    Ok(records)
}

fn create_lifecycle_table(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE records (
            stable_id TEXT PRIMARY KEY NOT NULL,
            content_hash TEXT NOT NULL,
            payload TEXT NOT NULL
        );",
    )
}

fn stored_hash(connection: &Connection, id: &StableId) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "SELECT content_hash FROM records WHERE stable_id = ?1",
            params![id.as_str()],
            |row| row.get(0),
        )
        .optional()
}

fn apply(connection: &Connection, record: &RecordInput) -> rusqlite::Result<Transition> {
    match stored_hash(connection, &record.id)? {
        None => {
            let inserted = connection.execute(
                "INSERT INTO records (stable_id, content_hash, payload) VALUES (?1, ?2, ?3)",
                params![record.id.as_str(), record.hash.as_str(), record.payload],
            )?;
            assert_eq!(inserted, 1, "add must insert exactly one row");
            Ok(Transition::Added)
        }
        Some(existing_hash) if existing_hash == record.hash.as_str() => Ok(Transition::Unchanged),
        Some(_) => {
            let updated = connection.execute(
                "UPDATE records SET content_hash = ?2, payload = ?3 WHERE stable_id = ?1",
                params![record.id.as_str(), record.hash.as_str(), record.payload],
            )?;
            assert_eq!(updated, 1, "changed payload must update exactly one row");
            Ok(Transition::Changed)
        }
    }
}

fn delete(connection: &Connection, id: &StableId) -> rusqlite::Result<Transition> {
    let deleted = connection.execute(
        "DELETE FROM records WHERE stable_id = ?1",
        params![id.as_str()],
    )?;
    assert_eq!(deleted, 1, "delete must remove exactly one identity");
    Ok(Transition::Deleted)
}

fn assert_stored_hash(connection: &Connection, record: &RecordInput) -> rusqlite::Result<()> {
    assert_eq!(
        stored_hash(connection, &record.id)?,
        Some(record.hash.as_str().to_owned()),
        "stable identity must preserve the supplied content hash at the SQLite boundary"
    );
    Ok(())
}

fn run_red(database: &Path) -> Result<(), Box<dyn Error>> {
    let records = fixture_records()?;
    let connection = Connection::open(database)?;
    connection.execute_batch(
        "CREATE TABLE records (
            stable_id TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            payload TEXT NOT NULL
        );",
    )?;
    for record in records.iter().take(2) {
        connection.execute(
            "INSERT INTO records (stable_id, content_hash, payload) VALUES (?1, ?2, ?3)",
            params![record.id.as_str(), record.hash.as_str(), record.payload],
        )?;
    }
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM records", [], |row| row.get(0))?;
    assert_eq!(
        count, 1,
        "causal RED: lifecycle storage without stable identity uniqueness permits duplicate state"
    );
    Ok(())
}

fn run_green(database: &Path) -> Result<(), Box<dyn Error>> {
    let records = fixture_records()?;
    let connection = Connection::open(database)?;
    create_lifecycle_table(&connection)?;
    let mut transitions = Vec::new();
    transitions.push(apply(&connection, &records[0])?);
    assert_stored_hash(&connection, &records[0])?;
    transitions.push(apply(&connection, &records[0])?);
    assert_stored_hash(&connection, &records[0])?;
    transitions.push(apply(&connection, &records[1])?);
    assert_stored_hash(&connection, &records[1])?;
    transitions.push(apply(&connection, &records[2])?);
    assert_stored_hash(&connection, &records[2])?;
    transitions.push(delete(&connection, &records[0].id)?);
    assert_eq!(
        transitions,
        vec![
            Transition::Added,
            Transition::Unchanged,
            Transition::Changed,
            Transition::Changed,
            Transition::Deleted,
        ]
    );
    assert_eq!(stored_hash(&connection, &records[0].id)?, None);
    println!("D3 PASS: add=1; unchanged=1; changed=2; delete=1; stable_id_and_hash=preserved");
    Ok(())
}

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "green".to_owned());
    let database = env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("d3-lifecycle.sqlite"));
    let result = match mode.as_str() {
        "red" => run_red(&database),
        "green" => run_green(&database),
        _ => Err("mode must be red or green".into()),
    };
    if let Err(error) = result {
        eprintln!("D3 {mode} observed: {error}");
        std::process::exit(101);
    }
}
