use std::env;
use std::fs;
use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::{QueryParser, TermQuery};
use tantivy::schema::{IndexRecordOption, STORED, STRING, Schema, TEXT, Value};
use tantivy::{Index, TantivyDocument, Term, doc};

const EXACT_RECORD_ID: &str = "markdown:synthetic/exact-document.md#ZX42";
const PHRASE_RECORD_ID: &str = "markdown:synthetic/russian-phrase.md#RUS-001";
const TECHNICAL_IDENTIFIER: &str = "ZX42";
const RUSSIAN_PHRASE: &str = "поиск документа";

fn fixture_body(name: &str) -> tantivy::Result<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    let content = fs::read_to_string(&path)
        .map_err(|error| tantivy::TantivyError::InvalidArgument(error.to_string()))?;
    let mut lines = content.lines();
    let heading = lines.next().unwrap_or_default();
    let separator = lines.next().unwrap_or_default();
    let body = lines.collect::<Vec<_>>().join("\n");
    if !heading.starts_with("# ") || !separator.is_empty() || body.trim().is_empty() {
        return Err(tantivy::TantivyError::InvalidArgument(format!(
            "fixture {name} must contain one H1 and one non-empty body"
        )));
    }
    Ok(body)
}

fn stable_ids(
    index: &Index,
    query: &dyn tantivy::query::Query,
    stable_id_field: tantivy::schema::Field,
) -> tantivy::Result<Vec<String>> {
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let top_docs = searcher.search(query, &TopDocs::with_limit(10).order_by_score())?;
    top_docs
        .into_iter()
        .map(|(_, address)| {
            let document: TantivyDocument = searcher.doc(address)?;
            document
                .get_first(stable_id_field)
                .and_then(|value| value.as_str())
                .map(str::to_owned)
                .ok_or_else(|| tantivy::TantivyError::InvalidArgument("missing stable_id".into()))
        })
        .collect()
}

fn run_red() -> tantivy::Result<()> {
    let exact_body = fixture_body("exact-document.md")?;
    let phrase_body = fixture_body("russian-phrase.md")?;
    let mut schema_builder = Schema::builder();
    let stable_id = schema_builder.add_text_field("stable_id", STRING | STORED);
    let combined = schema_builder.add_text_field("combined", TEXT);
    let schema = schema_builder.build();
    let index = Index::create_in_ram(schema);
    let mut writer = index.writer(15_000_000)?;
    writer.add_document(doc!(stable_id => EXACT_RECORD_ID, combined => format!("{TECHNICAL_IDENTIFIER} {exact_body}")))?;
    writer.add_document(doc!(stable_id => PHRASE_RECORD_ID, combined => phrase_body))?;
    writer.commit()?;

    let query = QueryParser::for_index(&index, vec![combined]).parse_query(TECHNICAL_IDENTIFIER)?;
    let hit_ids = stable_ids(&index, query.as_ref(), stable_id)?;
    assert_eq!(
        hit_ids,
        vec![EXACT_RECORD_ID.to_owned()],
        "causal RED: one combined TEXT field cannot prove exact technical identifier intent"
    );
    Ok(())
}

fn phrase_hits(
    index: &Index,
    text_field: tantivy::schema::Field,
    stable_id_field: tantivy::schema::Field,
    phrase: &str,
) -> tantivy::Result<Vec<String>> {
    if phrase.trim().is_empty() {
        return Ok(Vec::new());
    }
    let query =
        QueryParser::for_index(index, vec![text_field]).parse_query(&format!("\"{}\"", phrase))?;
    stable_ids(index, query.as_ref(), stable_id_field)
}

fn run_green() -> tantivy::Result<()> {
    let exact_body = fixture_body("exact-document.md")?;
    let phrase_body = fixture_body("russian-phrase.md")?;
    let mut schema_builder = Schema::builder();
    let stable_id = schema_builder.add_text_field("stable_id", STRING | STORED);
    let exact_identifier = schema_builder.add_text_field("exact_identifier", STRING);
    let russian_text = schema_builder.add_text_field("russian_text", TEXT);
    let schema = schema_builder.build();
    let index = Index::create_in_ram(schema);
    let mut writer = index.writer(15_000_000)?;
    writer.add_document(doc!(
        stable_id => EXACT_RECORD_ID,
        exact_identifier => TECHNICAL_IDENTIFIER,
        russian_text => exact_body
    ))?;
    writer.add_document(doc!(
        stable_id => PHRASE_RECORD_ID,
        exact_identifier => "RUS-001",
        russian_text => phrase_body
    ))?;
    writer.commit()?;

    let exact_query = TermQuery::new(
        Term::from_field_text(exact_identifier, TECHNICAL_IDENTIFIER),
        IndexRecordOption::Basic,
    );
    assert_eq!(
        stable_ids(&index, &exact_query, stable_id)?,
        vec![EXACT_RECORD_ID.to_owned()],
        "exact lookup must use only exact_identifier"
    );
    assert_eq!(
        phrase_hits(&index, russian_text, stable_id, RUSSIAN_PHRASE)?,
        vec![PHRASE_RECORD_ID.to_owned()],
        "Russian phrase query must use only russian_text"
    );
    assert!(phrase_hits(&index, russian_text, stable_id, "")?.is_empty());
    assert!(phrase_hits(&index, russian_text, stable_id, "несуществующая фраза")?.is_empty());

    println!(
        "D2 PASS: exact_id={EXACT_RECORD_ID}; russian_phrase={PHRASE_RECORD_ID}; no_hit=explicit"
    );
    Ok(())
}

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "green".to_owned());
    let result = match mode.as_str() {
        "red" => run_red(),
        "green" => run_green(),
        _ => Err(tantivy::TantivyError::InvalidArgument(
            "mode must be red or green".into(),
        )),
    };
    if let Err(error) = result {
        eprintln!("D2 {mode} observed: {error}");
        std::process::exit(101);
    }
}
