//! Filesystem boundary for read-only document sources.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;
use sha2::{Digest, Sha256};

use crate::domain::{
    CanonicalRecord, ContentHash, ErrorKind, FastSearchError, FileHash, RecordKind, SourceLocator,
    SourceSnapshot, StableId,
};
use crate::ports::SourcePort;

const EXCLUDED_DIRECTORIES: &[&str] = &[
    ".agents",
    ".cfknowledge",
    ".git",
    ".obsidian",
    "build",
    "generated",
    "service",
    "target",
    "vendor",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScannedSourceKind {
    Markdown,
    Tsv,
}

/// A verified filesystem source awaiting B2/B3 parsing.
#[derive(Debug)]
struct ScannedSource {
    path: PathBuf,
    locator: String,
    bytes: Vec<u8>,
    kind: ScannedSourceKind,
}

fn scan_sources(root: &Path) -> Result<Vec<ScannedSource>, FastSearchError> {
    let root = root
        .canonicalize()
        .map_err(|error| source_failure("canonicalize source root", error))?;
    if !root.is_dir() {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "source root must be a directory",
        ));
    }

    let mut sources = Vec::new();
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_global(false)
        .git_exclude(false)
        .git_ignore(true)
        .require_git(false)
        .follow_links(false);
    for entry in builder
        .filter_entry(|entry| {
            !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_dir())
                || !EXCLUDED_DIRECTORIES
                    .iter()
                    .any(|excluded| entry.file_name() == OsStr::new(excluded))
        })
        .build()
    {
        let entry = entry.map_err(|error| {
            FastSearchError::new(
                ErrorKind::SourceFailure,
                format!("walk source tree: {error}"),
            )
        })?;
        if entry.depth() == 0
            || !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        if let Some(kind) = source_kind(entry.path()) {
            sources.push(read_source(&root, entry.into_path(), kind)?);
        }
    }
    sources.sort_by(|left, right| left.locator.cmp(&right.locator));
    Ok(sources)
}

/// Reads canonical Markdown snapshots from the verified source root.
pub fn markdown_snapshots(root: &Path) -> Result<Vec<SourceSnapshot>, FastSearchError> {
    collect_snapshots(root, Some(ScannedSourceKind::Markdown))
}

/// Read-only filesystem implementation of the source boundary.
#[derive(Debug)]
pub struct FilesystemSource {
    root: PathBuf,
}

impl FilesystemSource {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn snapshots(&self) -> Result<Vec<SourceSnapshot>, FastSearchError> {
        let snapshots = collect_snapshots(&self.root, None)?;
        ensure_unique_snapshot_ids(&snapshots)?;
        Ok(snapshots)
    }
}

impl SourcePort for FilesystemSource {
    fn records(&self) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        let snapshots = self.snapshots()?;
        let mut records = Vec::new();
        for snapshot in snapshots {
            for record in snapshot.records() {
                records.push(record.clone());
            }
        }
        Ok(records)
    }

    fn snapshot(&self) -> Result<Vec<SourceSnapshot>, FastSearchError> {
        self.snapshots()
    }
}

fn ensure_unique_snapshot_ids(snapshots: &[SourceSnapshot]) -> Result<(), FastSearchError> {
    let mut ids = BTreeSet::new();
    for snapshot in snapshots {
        for record in snapshot.records() {
            if !ids.insert(record.id().as_str()) {
                return Err(FastSearchError::new(
                    ErrorKind::DuplicateStableId,
                    "source snapshots contain duplicate stable IDs",
                ));
            }
        }
    }
    Ok(())
}

fn collect_snapshots(
    root: &Path,
    kind: Option<ScannedSourceKind>,
) -> Result<Vec<SourceSnapshot>, FastSearchError> {
    scan_sources(root)?
        .iter()
        .filter(|source| kind.is_none_or(|expected| source.kind == expected))
        .map(|source| {
            let parsed = match source.kind {
                ScannedSourceKind::Markdown => parse_markdown_source(source),
                ScannedSourceKind::Tsv => parse_tsv_source(source),
            };
            parsed.map_err(|error| {
                FastSearchError::new(
                    error.kind().clone(),
                    format!("{}: {}", source.locator, error.message()),
                )
            })
        })
        .collect()
}

fn read_source(
    root: &Path,
    path: PathBuf,
    kind: ScannedSourceKind,
) -> Result<ScannedSource, FastSearchError> {
    let canonical_path = path
        .canonicalize()
        .map_err(|error| source_failure("canonicalize source file", error))?;
    let relative = canonical_path.strip_prefix(root).map_err(|_| {
        FastSearchError::new(
            ErrorKind::SourceFailure,
            "source file escapes configured root",
        )
    })?;
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "source locator is not a contained relative path",
        ));
    }
    let bytes =
        fs::read(&canonical_path).map_err(|error| source_failure("read source file", error))?;
    std::str::from_utf8(&bytes)
        .map_err(|_| FastSearchError::new(ErrorKind::SourceFailure, "source file is not UTF-8"))?;

    Ok(ScannedSource {
        locator: relative.to_string_lossy().replace('\\', "/"),
        path: canonical_path,
        bytes,
        kind,
    })
}

fn source_kind(path: &Path) -> Option<ScannedSourceKind> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("md") => Some(ScannedSourceKind::Markdown),
        Some("tsv") => Some(ScannedSourceKind::Tsv),
        _ => None,
    }
}

fn parse_markdown_source(source: &ScannedSource) -> Result<SourceSnapshot, FastSearchError> {
    if source.kind != ScannedSourceKind::Markdown {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "expected Markdown source",
        ));
    }
    if !source.path.is_absolute() {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "Markdown source path must remain canonical and absolute",
        ));
    }
    let document = std::str::from_utf8(&source.bytes)
        .map_err(|_| FastSearchError::new(ErrorKind::SourceFailure, "source file is not UTF-8"))?;
    let document = normalize_document(document);
    let frontmatter = parse_frontmatter(&document)?;
    let mut sections = Vec::new();
    let mut headings = Vec::new();
    let mut current: Option<MarkdownSection> = None;

    for line in frontmatter.markdown.lines() {
        if let Some((level, heading)) = markdown_heading(line)? {
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
            canonical_markdown_record(
                &source.locator,
                &frontmatter.metadata,
                &frontmatter.relations,
                section,
            )
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    let locator = SourceLocator::whole_file(&source.locator)
        .map_err(|error| source_contract_failure(error.message()))?;
    let file_hash = FileHash::parse(versioned_hash("file", [document.as_str()]))
        .map_err(|error| source_contract_failure(error.message()))?;
    Ok(SourceSnapshot::new(locator, file_hash, records))
}

fn parse_tsv_source(source: &ScannedSource) -> Result<SourceSnapshot, FastSearchError> {
    if source.kind != ScannedSourceKind::Tsv {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "expected TSV source",
        ));
    }
    if !source.path.is_absolute() {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "TSV source path must remain canonical and absolute",
        ));
    }
    let document = std::str::from_utf8(&source.bytes)
        .map_err(|_| FastSearchError::new(ErrorKind::SourceFailure, "source file is not UTF-8"))?;
    let document = normalize_document(document);
    let mut lines = document.lines().enumerate();
    let Some((_, header)) = lines.next() else {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "TSV source requires a header row",
        ));
    };
    let headers = parse_tsv_header(header)?;
    let records = lines
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| parse_tsv_record(&source.locator, index + 1, &headers, line))
        .collect::<Result<Vec<_>, _>>()?;
    let locator = SourceLocator::whole_file(&source.locator)
        .map_err(|error| source_contract_failure(error.message()))?;
    let file_hash = FileHash::parse(versioned_hash("file", [document.as_str()]))
        .map_err(|error| source_contract_failure(error.message()))?;
    Ok(SourceSnapshot::new(locator, file_hash, records))
}

fn parse_tsv_header(header: &str) -> Result<Vec<String>, FastSearchError> {
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

fn parse_tsv_record(
    path: &str,
    row: usize,
    headers: &[String],
    line: &str,
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
    let id = StableId::parse(format!("registry:{path}#row={row}"))
        .map_err(|error| source_contract_failure(error.message()))?;
    let locator = SourceLocator::registry_row(path, row)
        .map_err(|error| source_contract_failure(error.message()))?;
    let content_hash = ContentHash::parse(tsv_record_hash(path, row, title, &content, &metadata))
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

fn canonical_markdown_record(
    path: &str,
    metadata: &BTreeMap<String, String>,
    relations: &[StableId],
    section: MarkdownSection,
) -> Result<Option<CanonicalRecord>, FastSearchError> {
    let content = section.body.join("\n").trim().to_owned();
    if content.is_empty() {
        return Ok(None);
    }
    let heading_path = section.headings.join("/");
    let id = StableId::parse(format!("markdown:{path}#{heading_path}"))
        .map_err(|error| source_contract_failure(error.message()))?;
    let locator = SourceLocator::markdown(path, section.headings.iter().cloned())
        .map_err(|error| source_contract_failure(error.message()))?;
    let content_hash = ContentHash::parse(markdown_record_hash(
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

fn normalize_document(document: &str) -> String {
    document
        .strip_prefix('\u{feff}')
        .unwrap_or(document)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
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

fn markdown_heading(line: &str) -> Result<Option<(usize, String)>, FastSearchError> {
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

fn markdown_record_hash(
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

fn tsv_record_hash(
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

fn versioned_hash<'a>(scope: &str, fields: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"fastsearch:sha256:v1\0");
    update_hash_field(&mut hasher, scope);
    for field in fields {
        update_hash_field(&mut hasher, field);
    }
    format!("sha256:v1:{:x}", hasher.finalize())
}

fn update_hash_field(hasher: &mut Sha256, field: &str) {
    hasher.update((field.len() as u64).to_be_bytes());
    hasher.update(field.as_bytes());
}

fn source_contract_failure(message: &str) -> FastSearchError {
    FastSearchError::new(ErrorKind::SourceFailure, message)
}

fn source_failure(context: &str, error: std::io::Error) -> FastSearchError {
    FastSearchError::new(ErrorKind::SourceFailure, format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::domain::{ErrorKind, RecordKind, SourceSelector};
    use crate::ports::SourcePort;

    use super::{FilesystemSource, ScannedSourceKind, markdown_snapshots, scan_sources};

    static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn scanner_returns_only_allowed_contained_utf8_sources_in_locator_order() {
        let fixture = Fixture::new();
        fixture.write("docs/zeta.md", "zeta");
        fixture.write("docs/alpha.md", "alpha");
        fixture.write("registry.tsv", "id\ttitle");
        fixture.write("ignored.md", "ignored");
        fixture.write("ignored-dir/hidden.tsv", "hidden");
        fixture.write("target/build.md", "generated");
        fixture.write(".cfknowledge/derived.md", "derived state");
        fixture.write(".obsidian/config.md", "editor configuration");
        fixture.write("notes.txt", "unsupported");
        fixture.write(".gitignore", "ignored.md\nignored-dir/\n");

        let scanned = scan_sources(fixture.path()).expect("allowed fixture must scan");
        let locators = scanned
            .iter()
            .map(|source| source.locator.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            locators,
            ["docs/alpha.md", "docs/zeta.md", "registry.tsv"],
            "scanner must exclude ignored/build/unsupported files and sort lexically"
        );
        let canonical_root = fixture
            .path()
            .canonicalize()
            .expect("canonical fixture root");
        assert!(
            scanned
                .iter()
                .all(|source| source.path.starts_with(&canonical_root))
        );
        assert!(
            scanned
                .iter()
                .all(|source| std::str::from_utf8(&source.bytes).is_ok())
        );
        assert_eq!(
            scanned.iter().map(|source| source.kind).collect::<Vec<_>>(),
            [
                ScannedSourceKind::Markdown,
                ScannedSourceKind::Markdown,
                ScannedSourceKind::Tsv
            ]
        );
    }

    #[test]
    fn scanner_rejects_non_utf8_allowed_source_without_partial_result() {
        let fixture = Fixture::new();
        fixture.write("allowed.md", "valid");
        fs::write(fixture.path().join("invalid.tsv"), [0xff, 0xfe]).expect("invalid UTF-8 fixture");

        let error =
            scan_sources(fixture.path()).expect_err("invalid source must reject complete scan");

        assert_eq!(error.kind(), &ErrorKind::SourceFailure);
        assert!(error.message().contains("not UTF-8"));
    }

    #[test]
    fn scanner_supports_gitignore_globs_negation_and_nested_paths() {
        let fixture = Fixture::new();
        fixture.write("drop.md", "ignored by glob");
        fixture.write("keep.md", "restored by negation");
        fixture.write("nested/drop.tsv", "ignored nested path");
        fixture.write("registry.tsv", "id\ttitle\n2433\tkept");
        fixture.write(".gitignore", "*.md\n!keep.md\nnested/**\n");

        let scanned = scan_sources(fixture.path()).expect("standard gitignore rules must work");
        let locators = scanned
            .iter()
            .map(|source| source.locator.as_str())
            .collect::<Vec<_>>();

        assert_eq!(locators, ["keep.md", "registry.tsv"]);
    }

    #[test]
    fn scanner_does_not_follow_outside_root_junction_sentinel() {
        let fixture = Fixture::new();
        let outside = fixture.root.with_extension("outside");
        fs::create_dir(&outside).expect("outside sentinel directory");
        fs::write(outside.join("sentinel.md"), "outside sentinel").expect("outside sentinel");
        create_directory_junction(&outside, &fixture.path().join("outside-junction"));

        let scanned = scan_sources(fixture.path()).expect("junction must not compromise scan");

        assert!(
            scanned.is_empty(),
            "outside sentinel must not become source input"
        );
        fs::remove_dir(fixture.path().join("outside-junction")).expect("remove junction");
        fs::remove_dir_all(outside).expect("remove outside sentinel");
    }

    #[test]
    fn markdown_parser_returns_expected_section_records_with_frontmatter_metadata_and_heading_locators()
     {
        let fixture = Fixture::new();
        fixture.write(
            "docs/guide.md",
            "\u{feff}---\r\nalignment: CURRENT\r\nrelations: TDR-17, TDR-42\r\ntdr_refs: [TDR-17, TDR-42]\r\n# comment\r\nowner: Search team\r\n---\r\n\r\n# Руководство \r\nОбщий текст, не входящий в дочерние разделы.\r\n\r\n## Текущий поиск\r\n  русская фраза  \r\n\r\n### Детали\r\nТолько детали.\r\n\r\n## Пустой\r\n",
        );
        let snapshot = markdown_snapshots(fixture.path())
            .expect("Markdown fixture must parse")
            .pop()
            .expect("one Markdown snapshot");

        assert_eq!(snapshot.locator().path(), "docs/guide.md");
        assert_eq!(
            snapshot.file_hash().as_str(),
            "sha256:v1:bdcc47be2c980e49e83cccb883eff460c897cd609a431e64a5e2b53104aa68b4"
        );
        assert_eq!(
            snapshot.records().len(),
            3,
            "only sections with own body are records"
        );

        let guide = &snapshot.records()[0];
        assert_eq!(guide.id().as_str(), "markdown:docs/guide.md#Руководство");
        assert_eq!(
            guide.searchable_content(),
            "Общий текст, не входящий в дочерние разделы."
        );

        let current = &snapshot.records()[1];
        assert_eq!(
            current.id().as_str(),
            "markdown:docs/guide.md#Руководство/Текущий поиск"
        );
        assert_eq!(current.kind(), RecordKind::MarkdownSection);
        assert_eq!(current.title(), "Текущий поиск");
        assert_eq!(current.searchable_content(), "русская фраза");
        assert_eq!(
            current.metadata().get("alignment").map(String::as_str),
            Some("CURRENT")
        );
        assert_eq!(
            current.metadata().get("owner").map(String::as_str),
            Some("Search team")
        );
        assert_eq!(
            current.metadata().get("tdr_refs").map(String::as_str),
            Some("[TDR-17, TDR-42]")
        );
        assert_eq!(
            current
                .relations()
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>(),
            ["TDR-17", "TDR-42"]
        );
        assert_eq!(
            current.content_hash().as_str(),
            "sha256:v1:8b738824dd3ea8368d6b4c59a797322d393145830d95262512149f29324bd153"
        );
        assert_eq!(
            current.locator().selector(),
            &SourceSelector::MarkdownHeading {
                heading_path: vec!["Руководство".to_owned(), "Текущий поиск".to_owned()]
            }
        );

        let details = &snapshot.records()[2];
        assert_eq!(
            details.id().as_str(),
            "markdown:docs/guide.md#Руководство/Текущий поиск/Детали"
        );
        assert_eq!(details.searchable_content(), "Только детали.");
        assert_eq!(
            details.content_hash().as_str(),
            "sha256:v1:b62d58c21bf327c9786dd14461e558264182023cb8b810dcf616d47f80085c9b"
        );
    }

    #[test]
    fn markdown_parser_rejects_invalid_frontmatter_without_partial_snapshot() {
        for (name, invalid, expected_message) in [
            (
                "duplicate.md",
                "---\nowner: team\nowner: duplicate\n---\n# Broken\ntext",
                "duplicate frontmatter key",
            ),
            (
                "malformed.md",
                "---\nowner\n---\n# Broken\ntext",
                "malformed Markdown frontmatter entry",
            ),
            (
                "block-scalar.md",
                "---\nnotes: |\n  multiline\n---\n# Broken\ntext",
                "scalar key: value",
            ),
            (
                "unterminated.md",
                "---\nowner: team\n# Broken\ntext",
                "unterminated Markdown frontmatter",
            ),
        ] {
            let fixture = Fixture::new();
            fixture.write("valid.md", "# Valid\ntext");
            fixture.write(name, invalid);

            let error = markdown_snapshots(fixture.path())
                .expect_err("one invalid Markdown source rejects the complete snapshot call");

            assert_eq!(error.kind(), &ErrorKind::SourceFailure);
            assert!(error.message().contains(expected_message));
            assert!(
                error.message().contains(name),
                "source diagnostics must identify the failing relative path"
            );
        }
    }

    #[test]
    fn filesystem_source_returns_deterministic_markdown_and_tsv_records() {
        let fixture = Fixture::new();
        fixture.write("docs/guide.md", "# Guide\nMarkdown body");
        fixture.write(
            "registry.tsv",
            "id\ttitle\tstatus\n2433\tTechnical entry\tcurrent\n",
        );

        let source = FilesystemSource::new(fixture.path());
        let snapshots = source.snapshot().expect("combined fixture must parse");
        let records = source.records().expect("combined records must parse");

        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[1].locator().path(), "registry.tsv");
        assert_eq!(
            snapshots[1].file_hash().as_str(),
            "sha256:v1:66d7796440424a405e8d426be285e89753d3066510fb477071cc124bea11ce32"
        );
        assert_eq!(
            records
                .iter()
                .map(|record| record.id().as_str())
                .collect::<Vec<_>>(),
            [
                "markdown:docs/guide.md#Guide",
                "registry:registry.tsv#row=2"
            ]
        );
        let registry = &records[1];
        assert_eq!(registry.kind(), RecordKind::RegistryRow);
        assert_eq!(registry.title(), "2433");
        assert_eq!(
            registry.searchable_content(),
            "2433\tTechnical entry\tcurrent"
        );
        assert_eq!(
            registry.metadata().get("format").map(String::as_str),
            Some("tsv")
        );
        assert_eq!(
            registry.metadata().get("title").map(String::as_str),
            Some("Technical entry")
        );
        assert_eq!(
            registry.metadata().get("status").map(String::as_str),
            Some("current")
        );
        assert_eq!(registry.relations(), []);
        assert_eq!(
            registry.locator().selector(),
            &SourceSelector::RegistryRow {
                row: 2.try_into().expect("non-zero row")
            }
        );
        assert_eq!(
            registry.content_hash().as_str(),
            "sha256:v1:acbf4ea95bff0fc3e7641c553cbc19e8237c95c83a0043902e90348f235d819a"
        );
    }

    #[test]
    fn filesystem_source_rejects_malformed_tsv_without_partial_result() {
        for (name, registry, expected_message) in [
            (
                "blank-header",
                "id\t\n2433\tentry",
                "nonblank title and metadata columns",
            ),
            (
                "duplicate-header",
                "id\tstatus\tstatus\n2433\tcurrent\tagain",
                "headers must be unique",
            ),
            (
                "reserved-header",
                "id\tformat\n2433\tspoofed",
                "must not override format",
            ),
            (
                "wrong-arity",
                "id\tstatus\n2433",
                "does not match header arity",
            ),
            (
                "blank-title",
                "id\tstatus\n\tcurrent",
                "title must not be blank",
            ),
        ] {
            let fixture = Fixture::new();
            fixture.write("valid.md", "# Valid\ntext");
            fixture.write("registry.tsv", registry);

            let error = FilesystemSource::new(fixture.path())
                .records()
                .expect_err(name);

            assert_eq!(error.kind(), &ErrorKind::SourceFailure);
            assert!(error.message().contains(expected_message));
        }
    }

    #[test]
    fn filesystem_source_rejects_duplicate_stable_ids() {
        let fixture = Fixture::new();
        fixture.write("duplicate.md", "# Repeated\nfirst\n# Repeated\nsecond");

        let error = FilesystemSource::new(fixture.path())
            .records()
            .expect_err("duplicate records must never be silently overwritten");

        assert_eq!(error.kind(), &ErrorKind::DuplicateStableId);
    }

    #[cfg(windows)]
    fn create_directory_junction(target: &std::path::Path, link: &std::path::Path) {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .expect("start mklink /J for Windows containment fixture");
        assert!(
            output.status.success(),
            "mklink /J failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    fn create_directory_junction(target: &std::path::Path, link: &std::path::Path) {
        std::os::unix::fs::symlink(target, link).expect("create symlink sentinel");
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("fastsearch-b1-{}-{serial}", std::process::id()));
            fs::create_dir_all(&root).expect("fixture root");
            Self { root }
        }

        fn path(&self) -> &std::path::Path {
            &self.root
        }

        fn write(&self, relative: &str, content: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture parent");
            fs::write(path, content).expect("fixture file");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
