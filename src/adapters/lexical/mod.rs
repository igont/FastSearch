//! Disposable Tantivy projection of authoritative canonical records.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use tantivy::{
    Index, TantivyDocument, Term,
    collector::TopDocs,
    query::{Query, QueryParser, TermQuery},
    schema::{IndexRecordOption, STORED, STRING, Schema, TEXT, Value},
};

use crate::{
    domain::{
        CanonicalRecord, ContentHash, ErrorKind, FastSearchError, IndexFreshness, RecordKind,
        RetrievalChannel, SearchHit, SearchQuery, SearchResponse, SourceLocator, SourceSelector,
        StableId,
    },
    ports::LexicalRetrieval,
};

const MARKER: &str = "projection.marker";
const PAYLOAD_FIELD: &str = "payload";
const STABLE_ID_FIELD: &str = "stable_id";
const EXACT_IDENTIFIER_FIELD: &str = "exact_identifier";
const RUSSIAN_TEXT_FIELD: &str = "russian_text";

/// A persistent, disposable lexical projection. Canonical records remain its input authority.
#[derive(Debug)]
pub struct TantivyLexical {
    root: PathBuf,
    operation: Mutex<()>,
}

impl TantivyLexical {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, FastSearchError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(projection_error)?;
        Ok(Self {
            root,
            operation: Mutex::new(()),
        })
    }

    fn marker_path(&self) -> PathBuf {
        self.root.join(MARKER)
    }

    fn status(&self) -> LifecycleMarker {
        read_marker(&self.marker_path()).unwrap_or_else(|_| LifecycleMarker::stale())
    }

    fn mark_degraded(&self, state_generation: u64, detail: &str) {
        let mut marker = self.status();
        marker.freshness = IndexFreshness::Degraded;
        marker.state_generation = state_generation;
        marker.detail = detail.to_owned();
        let _ = write_marker(&self.marker_path(), &marker);
    }

    fn write_projection(
        &self,
        records: &[CanonicalRecord],
        state_generation: u64,
    ) -> Result<LifecycleMarker, FastSearchError> {
        validate_unique_ids(records)?;
        let generation = state_generation;
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(projection_error)?
            .as_nanos();
        let directory = self.root.join(format!("generation-{generation}-{suffix}"));
        fs::create_dir_all(&directory).map_err(projection_error)?;

        let result = (|| -> Result<(), FastSearchError> {
            let (schema, fields) = projection_schema();
            let index = Index::create_in_dir(&directory, schema).map_err(projection_error)?;
            let mut writer = index.writer(15_000_000).map_err(projection_error)?;
            for record in records {
                let mut document = TantivyDocument::default();
                document.add_text(fields.stable_id, record.id().as_str());
                for identifier in technical_identifiers(record) {
                    document.add_text(fields.exact_identifier, identifier);
                }
                document.add_text(
                    fields.russian_text,
                    format!("{}\n{}", record.title(), record.searchable_content()),
                );
                document.add_text(fields.payload, encode_record(record));
                writer.add_document(document).map_err(projection_error)?;
            }
            writer.commit().map_err(projection_error)?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&directory);
            return Err(error);
        }

        let marker = LifecycleMarker {
            freshness: IndexFreshness::Current,
            state_generation,
            projection_generation: Some(generation),
            directory: Some(
                directory
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
            detail: "lexical projection is current".to_owned(),
        };
        write_marker(&self.marker_path(), &marker)?;
        Ok(marker)
    }

    fn query(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        let marker = self.status();
        let Some(directory) = marker.directory.as_ref() else {
            return Ok(SearchResponse::with_freshness(Vec::new(), marker.freshness));
        };
        let (schema, fields) = projection_schema();
        let index = Index::open_in_dir(self.root.join(directory)).map_err(projection_error)?;
        if index.schema() != schema {
            return Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "lexical projection schema is incompatible",
            ));
        }
        let reader = index.reader().map_err(projection_error)?;
        let searcher = reader.searcher();
        let quoted = query.text().trim();
        let (parsed, channel): (Box<dyn Query>, RetrievalChannel) = if is_quoted(quoted) {
            let parser = QueryParser::for_index(&index, vec![fields.russian_text]);
            (
                parser.parse_query(quoted).map_err(projection_error)?,
                RetrievalChannel::Lexical,
            )
        } else {
            (
                Box::new(TermQuery::new(
                    Term::from_field_text(fields.exact_identifier, &quoted.to_ascii_lowercase()),
                    IndexRecordOption::Basic,
                )),
                RetrievalChannel::Exact,
            )
        };
        let documents = searcher
            .search(parsed.as_ref(), &TopDocs::with_limit(100).order_by_score())
            .map_err(projection_error)?;
        let mut hits = documents
            .into_iter()
            .map(|(score, address)| {
                let document: TantivyDocument = searcher.doc(address).map_err(projection_error)?;
                let payload = document
                    .get_first(fields.payload)
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        FastSearchError::new(
                            ErrorKind::ProjectionFailure,
                            "projection document has no payload",
                        )
                    })?;
                Ok(SearchHit::new(
                    decode_record(payload)?,
                    channel,
                    f64::from(score),
                ))
            })
            .collect::<Result<Vec<_>, FastSearchError>>()?;
        hits.sort_by(|left, right| {
            left.record()
                .id()
                .as_str()
                .cmp(right.record().id().as_str())
        });
        Ok(SearchResponse::with_freshness(hits, marker.freshness))
    }
}

impl LexicalRetrieval for TantivyLexical {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        let _guard = self
            .operation
            .lock()
            .expect("lexical operation mutex is not poisoned");
        self.query(query)
    }

    fn lifecycle_status(&self) -> crate::domain::LifecycleStatus {
        let _guard = self
            .operation
            .lock()
            .expect("lexical operation mutex is not poisoned");
        self.status().to_status()
    }

    fn apply_projection(
        &self,
        records: &[CanonicalRecord],
        state_generation: u64,
    ) -> Result<crate::domain::LifecycleStatus, FastSearchError> {
        let _guard = self
            .operation
            .lock()
            .expect("lexical operation mutex is not poisoned");
        match self.write_projection(records, state_generation) {
            Ok(marker) => Ok(marker.to_status()),
            Err(error) => {
                self.mark_degraded(state_generation, error.message());
                Err(error)
            }
        }
    }

    fn rebuild(
        &self,
        records: &[CanonicalRecord],
        state_generation: u64,
    ) -> Result<crate::domain::LifecycleStatus, FastSearchError> {
        self.apply_projection(records, state_generation)
    }
}

#[derive(Clone, Debug)]
struct LifecycleMarker {
    freshness: IndexFreshness,
    state_generation: u64,
    projection_generation: Option<u64>,
    directory: Option<String>,
    detail: String,
}

impl LifecycleMarker {
    fn stale() -> Self {
        Self {
            freshness: IndexFreshness::Stale,
            state_generation: 0,
            projection_generation: None,
            directory: None,
            detail: "lexical projection is absent".to_owned(),
        }
    }

    fn to_status(&self) -> crate::domain::LifecycleStatus {
        crate::domain::LifecycleStatus::new(
            self.freshness,
            self.state_generation,
            self.projection_generation,
            &self.detail,
        )
    }
}

fn projection_schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let stable_id = builder.add_text_field(STABLE_ID_FIELD, STRING | STORED);
    let exact_identifier = builder.add_text_field(EXACT_IDENTIFIER_FIELD, STRING);
    let russian_text = builder.add_text_field(RUSSIAN_TEXT_FIELD, TEXT);
    let payload = builder.add_text_field(PAYLOAD_FIELD, STORED);
    (
        builder.build(),
        Fields {
            stable_id,
            exact_identifier,
            russian_text,
            payload,
        },
    )
}

struct Fields {
    stable_id: tantivy::schema::Field,
    exact_identifier: tantivy::schema::Field,
    russian_text: tantivy::schema::Field,
    payload: tantivy::schema::Field,
}

fn validate_unique_ids(records: &[CanonicalRecord]) -> Result<(), FastSearchError> {
    let mut ids = HashSet::new();
    for record in records {
        if !ids.insert(record.id().as_str()) {
            return Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "projection input contains a duplicate stable identifier",
            ));
        }
    }
    Ok(())
}

fn technical_identifiers(record: &CanonicalRecord) -> Vec<String> {
    let mut values = vec![
        record.id().as_str(),
        record.title(),
        record.searchable_content(),
    ];
    values.extend(
        record
            .metadata()
            .iter()
            .flat_map(|(key, value)| [key.as_str(), value.as_str()]),
    );
    values.extend(record.relations().iter().map(StableId::as_str));
    values.into_iter().flat_map(ascii_tokens).collect()
}

fn ascii_tokens(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn is_quoted(query: &str) -> bool {
    query.len() >= 2 && query.starts_with('"') && query.ends_with('"')
}

fn projection_error(error: impl std::fmt::Display) -> FastSearchError {
    FastSearchError::new(ErrorKind::ProjectionFailure, error.to_string())
}

fn write_marker(path: &Path, marker: &LifecycleMarker) -> Result<(), FastSearchError> {
    let freshness = match marker.freshness {
        IndexFreshness::Current => "current",
        IndexFreshness::Stale => "stale",
        IndexFreshness::Degraded => "degraded",
        IndexFreshness::NotConfigured => "not-configured",
    };
    let content = encode_parts(&[
        freshness.to_owned(),
        marker.state_generation.to_string(),
        marker
            .projection_generation
            .map_or_else(String::new, |value| value.to_string()),
        marker.directory.clone().unwrap_or_default(),
        marker.detail.clone(),
    ]);
    fs::write(path, content).map_err(projection_error)
}

fn read_marker(path: &Path) -> Result<LifecycleMarker, FastSearchError> {
    let content = fs::read_to_string(path).map_err(projection_error)?;
    let fields = decode_parts(&content)?;
    if fields.len() != 5 {
        return Err(FastSearchError::new(
            ErrorKind::ProjectionFailure,
            "invalid lexical projection marker",
        ));
    }
    let freshness = match fields[0].as_str() {
        "current" => IndexFreshness::Current,
        "stale" => IndexFreshness::Stale,
        "degraded" => IndexFreshness::Degraded,
        _ => {
            return Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "invalid lexical projection freshness",
            ));
        }
    };
    Ok(LifecycleMarker {
        freshness,
        state_generation: fields[1].parse().map_err(projection_error)?,
        projection_generation: if fields[2].is_empty() {
            None
        } else {
            Some(fields[2].parse().map_err(projection_error)?)
        },
        directory: (!fields[3].is_empty()).then(|| fields[3].clone()),
        detail: fields[4].clone(),
    })
}

fn encode_record(record: &CanonicalRecord) -> String {
    let (selector_kind, selector_values) = match record.locator().selector() {
        SourceSelector::MarkdownHeading { heading_path } => ("markdown", heading_path.clone()),
        SourceSelector::RegistryRow { row } => ("registry", vec![row.to_string()]),
        SourceSelector::CodeSymbol { symbol } => ("symbol", vec![symbol.clone()]),
        SourceSelector::WholeFile => ("file", Vec::new()),
    };
    let mut parts = vec![
        record.id().as_str().to_owned(),
        record_kind(record.kind()).to_owned(),
        record.locator().path().to_owned(),
        selector_kind.to_owned(),
        selector_values.len().to_string(),
    ];
    parts.extend(selector_values);
    parts.extend([
        record.title().to_owned(),
        record.searchable_content().to_owned(),
        record.metadata().len().to_string(),
    ]);
    for (key, value) in record.metadata() {
        parts.extend([key.clone(), value.clone()]);
    }
    parts.push(record.relations().len().to_string());
    parts.extend(
        record
            .relations()
            .iter()
            .map(|relation| relation.as_str().to_owned()),
    );
    parts.push(record.content_hash().as_str().to_owned());
    encode_parts(&parts)
}

fn decode_record(payload: &str) -> Result<CanonicalRecord, FastSearchError> {
    let values = decode_parts(payload)?;
    let mut cursor = 0;
    let mut next = || {
        let value = values.get(cursor).cloned().ok_or_else(|| {
            FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "invalid lexical record payload",
            )
        });
        cursor += 1;
        value
    };
    let id = StableId::parse(next()?)?;
    let kind = match next()?.as_str() {
        "markdown" => RecordKind::MarkdownSection,
        "registry" => RecordKind::RegistryRow,
        "map" => RecordKind::CodeMap,
        "symbol" => RecordKind::CodeSymbol,
        _ => {
            return Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "invalid record kind",
            ));
        }
    };
    let path = next()?;
    let selector = next()?;
    let count: usize = next()?.parse().map_err(projection_error)?;
    let selector_values = (0..count).map(|_| next()).collect::<Result<Vec<_>, _>>()?;
    let locator = match selector.as_str() {
        "markdown" => SourceLocator::markdown(path, selector_values)?,
        "registry" => SourceLocator::registry_row(
            path,
            selector_values
                .first()
                .ok_or_else(|| {
                    FastSearchError::new(
                        ErrorKind::ProjectionFailure,
                        "registry selector is missing row",
                    )
                })?
                .parse()
                .map_err(projection_error)?,
        )?,
        "symbol" => SourceLocator::code_symbol(
            path,
            selector_values.first().ok_or_else(|| {
                FastSearchError::new(
                    ErrorKind::ProjectionFailure,
                    "symbol selector is missing name",
                )
            })?,
        )?,
        "file" => SourceLocator::whole_file(path)?,
        _ => {
            return Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "invalid locator selector",
            ));
        }
    };
    let title = next()?;
    let content = next()?;
    let metadata_count: usize = next()?.parse().map_err(projection_error)?;
    let mut metadata = std::collections::BTreeMap::new();
    for _ in 0..metadata_count {
        metadata.insert(next()?, next()?);
    }
    let relation_count: usize = next()?.parse().map_err(projection_error)?;
    let relations = (0..relation_count)
        .map(|_| next().and_then(StableId::parse))
        .collect::<Result<Vec<_>, _>>()?;
    let content_hash = ContentHash::parse(next()?)?;
    if cursor != values.len() {
        return Err(FastSearchError::new(
            ErrorKind::ProjectionFailure,
            "lexical record payload has trailing values",
        ));
    }
    CanonicalRecord::new(
        id,
        kind,
        locator,
        title,
        content,
        metadata,
        relations,
        content_hash,
    )
}

fn record_kind(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::MarkdownSection => "markdown",
        RecordKind::RegistryRow => "registry",
        RecordKind::CodeMap => "map",
        RecordKind::CodeSymbol => "symbol",
    }
}

fn encode_parts(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| format!("{}:{part}", part.len()))
        .collect()
}

fn decode_parts(input: &str) -> Result<Vec<String>, FastSearchError> {
    let mut remainder = input;
    let mut parts = Vec::new();
    while !remainder.is_empty() {
        let Some(separator) = remainder.find(':') else {
            return Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "invalid length-prefixed lexical payload",
            ));
        };
        let length: usize = remainder[..separator].parse().map_err(projection_error)?;
        let start = separator + 1;
        let end = start.checked_add(length).ok_or_else(|| {
            FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "lexical payload length overflows",
            )
        })?;
        if remainder.len() < end {
            return Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "truncated lexical payload",
            ));
        }
        if !remainder.is_char_boundary(start) || !remainder.is_char_boundary(end) {
            return Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "invalid UTF-8 lexical payload boundary",
            ));
        }
        let value = remainder[start..end].to_owned();
        parts.push(value);
        remainder = &remainder[end..];
    }
    Ok(parts)
}
