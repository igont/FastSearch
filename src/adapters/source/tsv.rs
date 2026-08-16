use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;

use crate::domain::{
    CanonicalRecord, ContentHash, ErrorKind, FastSearchError, FileHash, LogicalRootId, RecordKind,
    RootedSourceLocator, SourceLocator, SourceSnapshot, StableId,
};

use super::{normalize_document, source_contract_failure, versioned_hash};

#[cfg(test)]
pub(super) fn parse(locator: &str, bytes: &[u8]) -> Result<SourceSnapshot, FastSearchError> {
    parse_with_root(locator, bytes, None)
}

pub(super) fn parse_with_root(
    locator: &str,
    bytes: &[u8],
    root_id: Option<&LogicalRootId>,
) -> Result<SourceSnapshot, FastSearchError> {
    let document = std::str::from_utf8(bytes)
        .map_err(|_| FastSearchError::new(ErrorKind::SourceFailure, "source file is not UTF-8"))?;
    let document = normalize_document(document);
    let mut lines = document.lines().enumerate();
    let Some((_, header)) = lines.next() else {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "TSV source requires a header row",
        ));
    };
    let headers = parse_header(header)?;
    let records = lines
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| parse_record(locator, index + 1, &headers, line, root_id))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshot_locator = SourceLocator::whole_file(locator)
        .map_err(|error| source_contract_failure(error.message()))?;
    let file_hash = FileHash::parse(versioned_hash("file", [document.as_str()]))
        .map_err(|error| source_contract_failure(error.message()))?;
    Ok(match root_id {
        Some(root_id) => {
            SourceSnapshot::for_root(root_id.clone(), snapshot_locator, file_hash, records)
        }
        None => SourceSnapshot::new(snapshot_locator, file_hash, records),
    })
}

fn parse_header(header: &str) -> Result<Vec<String>, FastSearchError> {
    let headers = header
        .split('\t')
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if headers.len() < 2 || headers.iter().any(|header| header.is_empty()) {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "TSV header requires nonblank title and metadata columns",
        ));
    }
    let unique = headers.iter().skip(1).collect::<BTreeSet<_>>();
    if unique.len() != headers.len() - 1 || headers.iter().skip(1).any(|header| header == "format")
    {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "TSV metadata headers must be unique and must not override format",
        ));
    }
    Ok(headers)
}

fn parse_record(
    path: &str,
    row: usize,
    headers: &[String],
    line: &str,
    root_id: Option<&LogicalRootId>,
) -> Result<CanonicalRecord, FastSearchError> {
    let cells = line.split('\t').map(str::trim).collect::<Vec<_>>();
    if cells.len() != headers.len() {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "TSV data row does not match header arity",
        ));
    }
    let title = cells[0];
    if title.is_empty() {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "TSV data row title must not be blank",
        ));
    }
    let row = NonZeroUsize::new(row).ok_or_else(|| {
        FastSearchError::new(ErrorKind::SourceFailure, "TSV row number must be non-zero")
    })?;
    let content = cells.join("\t");
    let metadata = std::iter::once(("format".to_owned(), "tsv".to_owned()))
        .chain(
            headers
                .iter()
                .skip(1)
                .zip(cells.iter().skip(1))
                .map(|(header, cell)| (header.clone(), (*cell).to_owned())),
        )
        .collect::<BTreeMap<_, _>>();
    let locator = SourceLocator::registry_row(path, row)
        .map_err(|error| source_contract_failure(error.message()))?;
    let id = match root_id {
        Some(root_id) => RootedSourceLocator::new(root_id.clone(), locator.clone())?.stable_id(),
        None => StableId::parse(format!("registry:{path}#row={row}"))
            .map_err(|error| source_contract_failure(error.message()))?,
    };
    let content_hash = ContentHash::parse(record_hash(path, row, title, &content, &metadata))
        .map_err(|error| source_contract_failure(error.message()))?;
    CanonicalRecord::new(
        id,
        RecordKind::RegistryRow,
        locator,
        title,
        content,
        metadata,
        Vec::new(),
        content_hash,
    )
    .map_err(|error| source_contract_failure(error.message()))
}

fn record_hash(
    path: &str,
    row: NonZeroUsize,
    title: &str,
    content: &str,
    metadata: &BTreeMap<String, String>,
) -> String {
    let mut fields = vec![
        "registry".to_owned(),
        path.to_owned(),
        row.to_string(),
        title.to_owned(),
        content.to_owned(),
        metadata.len().to_string(),
    ];
    for (key, value) in metadata {
        fields.extend([key.clone(), value.clone()]);
    }
    fields.push("0".to_owned());
    versioned_hash("record", fields.iter().map(String::as_str))
}
