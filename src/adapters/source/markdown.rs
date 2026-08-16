use std::collections::BTreeMap;

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
    let frontmatter = parse_frontmatter(&document)?;
    let mut sections = Vec::new();
    let mut headings = Vec::new();
    let mut current: Option<MarkdownSection> = None;

    for line in frontmatter.markdown.lines() {
        if let Some((level, heading)) = heading(line)? {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            headings.truncate(level.saturating_sub(1));
            headings.push(heading.clone());
            current = Some(MarkdownSection {
                headings: headings.clone(),
                title: heading,
                body: Vec::new(),
            });
        } else if let Some(section) = &mut current {
            section.body.push(line.to_owned());
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }

    let records = sections
        .into_iter()
        .map(|section| {
            canonical_record(
                locator,
                &frontmatter.metadata,
                &frontmatter.relations,
                section,
                root_id,
            )
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
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

#[derive(Debug)]
struct MarkdownSection {
    headings: Vec<String>,
    title: String,
    body: Vec<String>,
}

#[derive(Debug)]
struct MarkdownFrontmatter {
    metadata: BTreeMap<String, String>,
    relations: Vec<StableId>,
    markdown: String,
}

fn canonical_record(
    path: &str,
    metadata: &BTreeMap<String, String>,
    relations: &[StableId],
    section: MarkdownSection,
    root_id: Option<&LogicalRootId>,
) -> Result<Option<CanonicalRecord>, FastSearchError> {
    let content = section.body.join("\n").trim().to_owned();
    if content.is_empty() {
        return Ok(None);
    }
    let heading_path = section.headings.join("/");
    let locator = SourceLocator::markdown(path, section.headings.iter().cloned())
        .map_err(|error| source_contract_failure(error.message()))?;
    let id = match root_id {
        Some(root_id) => RootedSourceLocator::new(root_id.clone(), locator.clone())?.stable_id(),
        None => StableId::parse(format!("markdown:{path}#{heading_path}"))
            .map_err(|error| source_contract_failure(error.message()))?,
    };
    let content_hash = ContentHash::parse(record_hash(
        path,
        &section.headings,
        &section.title,
        &content,
        metadata,
        relations,
    ))
    .map_err(|error| source_contract_failure(error.message()))?;
    CanonicalRecord::new(
        id,
        RecordKind::MarkdownSection,
        locator,
        section.title,
        content,
        metadata.clone(),
        relations.to_vec(),
        content_hash,
    )
    .map(Some)
    .map_err(|error| source_contract_failure(error.message()))
}

fn parse_frontmatter(document: &str) -> Result<MarkdownFrontmatter, FastSearchError> {
    let Some(after_open) = document.strip_prefix("---\n") else {
        return Ok(MarkdownFrontmatter {
            metadata: BTreeMap::new(),
            relations: Vec::new(),
            markdown: document.to_owned(),
        });
    };
    let (frontmatter, markdown) = after_open
        .strip_prefix("---\n")
        .map(|markdown| ("", markdown))
        .or_else(|| (after_open == "---").then_some(("", "")))
        .or_else(|| after_open.split_once("\n---\n"))
        .or_else(|| {
            after_open
                .strip_suffix("\n---")
                .map(|frontmatter| (frontmatter, ""))
        })
        .ok_or_else(|| {
            FastSearchError::new(
                ErrorKind::SourceFailure,
                "unterminated Markdown frontmatter",
            )
        })?;
    let mut metadata = BTreeMap::new();
    let mut relations = Vec::new();
    let mut relations_seen = false;
    for line in frontmatter.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once(':').ok_or_else(|| {
            FastSearchError::new(
                ErrorKind::SourceFailure,
                "malformed Markdown frontmatter entry",
            )
        })?;
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() || !is_supported_frontmatter_value(value) {
            return Err(FastSearchError::new(
                ErrorKind::SourceFailure,
                "frontmatter requires non-empty UTF-8 scalar key: value entries",
            ));
        }
        if key == "relations" {
            if relations_seen {
                return Err(FastSearchError::new(
                    ErrorKind::SourceFailure,
                    "duplicate frontmatter key: relations",
                ));
            }
            relations_seen = true;
            relations = value
                .split(',')
                .map(str::trim)
                .map(|relation| {
                    StableId::parse(relation.to_owned()).map_err(|_| {
                        FastSearchError::new(
                            ErrorKind::SourceFailure,
                            "frontmatter relations must be comma-separated non-empty StableIds",
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
        } else if metadata.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(FastSearchError::new(
                ErrorKind::SourceFailure,
                format!("duplicate frontmatter key: {key}"),
            ));
        }
    }
    Ok(MarkdownFrontmatter {
        metadata,
        relations,
        markdown: markdown.to_owned(),
    })
}

fn is_supported_frontmatter_value(value: &str) -> bool {
    match value.as_bytes().first() {
        Some(b'|') | Some(b'>') => false,
        Some(b'[') => value.ends_with(']'),
        Some(b'{') => value.ends_with('}'),
        Some(_) => true,
        None => false,
    }
}

fn heading(line: &str) -> Result<Option<(usize, String)>, FastSearchError> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    if level == 0 {
        return Ok(None);
    }
    let Some(rest) = line.get(level..) else {
        return Ok(None);
    };
    if level > 6 || !rest.starts_with(char::is_whitespace) {
        return Ok(None);
    }
    let heading = rest.trim().trim_end_matches('#').trim();
    if heading.is_empty() {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "Markdown heading must not be blank",
        ));
    }
    Ok(Some((level, heading.to_owned())))
}

fn record_hash(
    path: &str,
    headings: &[String],
    title: &str,
    content: &str,
    metadata: &BTreeMap<String, String>,
    relations: &[StableId],
) -> String {
    let mut fields = vec![
        "markdown".to_owned(),
        path.to_owned(),
        headings.len().to_string(),
    ];
    fields.extend(headings.iter().cloned());
    fields.extend([
        title.to_owned(),
        content.to_owned(),
        metadata.len().to_string(),
    ]);
    for (key, value) in metadata {
        fields.extend([key.clone(), value.clone()]);
    }
    fields.push(relations.len().to_string());
    fields.extend(
        relations
            .iter()
            .map(|relation| relation.as_str().to_owned()),
    );
    versioned_hash("record", fields.iter().map(String::as_str))
}
