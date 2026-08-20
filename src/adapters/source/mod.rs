//! Filesystem boundary for read-only document sources.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::domain::{
    CanonicalRecord, ErrorKind, FastSearchError, FileHash, LogicalRootId, SourceLocator,
    SourceSnapshot,
};
use crate::ports::SourcePort;
use serde::{Deserialize, Serialize};

mod markdown;
mod scanner;
/// Kept for direct parser-contract tests; scanner admission deliberately never
/// returns TSV, so these files cannot enter any FastSearch index.
#[allow(dead_code)]
mod tsv;

use scanner::{ScannedSourceKind, discover_sources_with_exclusions, read_source, scan_sources};

/// The changed subset of one source root plus every source key observed during
/// this scan.  The latter is what makes deletion detection exact.
pub struct IncrementalSnapshots {
    pub snapshots: Vec<SourceSnapshot>,
    pub seen_source_keys: BTreeSet<String>,
    pub decisions: Vec<SourceDecision>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownAdmissionRule {
    property: String,
    expected_value: String,
}

impl MarkdownAdmissionRule {
    pub fn new(
        property: impl Into<String>,
        expected_value: impl Into<String>,
    ) -> Result<Self, FastSearchError> {
        let property = property.into();
        let expected_value = expected_value.into();
        if property.trim().is_empty() || expected_value.trim().is_empty() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "Markdown admission requires a nonblank property and value",
            ));
        }
        Ok(Self {
            property,
            expected_value,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceDecision {
    pub root_id: Option<String>,
    pub path: String,
    pub included: bool,
    pub reason: String,
    pub source_hash: String,
}

/// Reads canonical Markdown snapshots from the verified source root.
pub fn markdown_snapshots(root: &Path) -> Result<Vec<SourceSnapshot>, FastSearchError> {
    collect_snapshots(root, Some(ScannedSourceKind::Markdown), None)
}

/// Read-only filesystem implementation of the source boundary.
#[derive(Debug)]
pub struct FilesystemSource {
    root: PathBuf,
    root_id: Option<LogicalRootId>,
    admission: Option<MarkdownAdmissionRule>,
    exclusions: Option<Vec<String>>,
}

impl FilesystemSource {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            root_id: None,
            admission: None,
            exclusions: None,
        }
    }

    #[must_use]
    pub fn new_named(root_id: LogicalRootId, root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            root_id: Some(root_id),
            admission: None,
            exclusions: None,
        }
    }

    #[must_use]
    pub fn with_markdown_admission(mut self, admission: Option<MarkdownAdmissionRule>) -> Self {
        self.admission = admission;
        self
    }

    #[must_use]
    pub fn with_exclusions(mut self, exclusions: Option<Vec<String>>) -> Self {
        self.exclusions = exclusions;
        self
    }

    fn snapshots(&self) -> Result<Vec<SourceSnapshot>, FastSearchError> {
        let snapshots = self.snapshots_incremental(&BTreeMap::new())?.snapshots;
        ensure_unique_snapshot_ids(&snapshots)?;
        Ok(snapshots)
    }

    /// Reads every eligible file once to calculate its canonical file hash,
    /// but parses Markdown only if that hash differs from the durable
    /// SQLite snapshot. This is deliberately content-hash based rather than
    /// `mtime` based, so copied files and restored timestamps stay correct.
    pub fn snapshots_incremental(
        &self,
        known_hashes: &BTreeMap<String, String>,
    ) -> Result<IncrementalSnapshots, FastSearchError> {
        let mut snapshots = Vec::new();
        let mut seen_source_keys = BTreeSet::new();
        let mut decisions = Vec::new();
        let discovered = match self.exclusions.as_deref() {
            Some(exclusions) => discover_sources_with_exclusions(&self.root, exclusions)?,
            None => scanner::discover_sources(&self.root)?,
        };
        for discovered in discovered {
            let source = read_source(discovered.path, discovered.locator, discovered.kind)?;
            let locator = SourceLocator::whole_file(source.locator.clone())
                .map_err(|error| source_contract_failure(error.message()))?;
            let key = SourceSnapshot::storage_key_for(self.root_id.as_ref(), &locator);
            let file_hash = normalized_file_hash(&source.bytes)?;
            let admission = self
                .admission
                .as_ref()
                .map(|rule| {
                    markdown::admission_value(&source.bytes, &rule.property, &rule.expected_value)
                })
                .transpose()
                .map_err(|error| {
                    FastSearchError::new(
                        error.kind().clone(),
                        format!("{}: {}", source.locator, error.message()),
                    )
                })?;
            let (included, reason) = match admission {
                Some(true) => (
                    true,
                    format!(
                        "property {}={}",
                        self.admission.as_ref().unwrap().property,
                        self.admission.as_ref().unwrap().expected_value
                    ),
                ),
                Some(false) => (
                    false,
                    format!(
                        "property {} is missing or does not equal {}",
                        self.admission.as_ref().unwrap().property,
                        self.admission.as_ref().unwrap().expected_value
                    ),
                ),
                None => (true, "no admission property configured".to_owned()),
            };
            decisions.push(SourceDecision {
                root_id: self.root_id.as_ref().map(|root| root.as_str().to_owned()),
                path: source.locator.clone(),
                included,
                reason,
                source_hash: file_hash.as_str().to_owned(),
            });
            if !included {
                continue;
            }
            seen_source_keys.insert(key.clone());
            if known_hashes.get(&key) == Some(&file_hash.as_str().to_owned()) {
                continue;
            }
            let snapshot = parse_source(
                source.kind,
                &source.locator,
                &source.bytes,
                self.root_id.as_ref(),
            )
            .map_err(|error| {
                FastSearchError::new(
                    error.kind().clone(),
                    format!("{}: {}", source.locator, error.message()),
                )
            })?;
            if snapshot.file_hash() != &file_hash {
                return Err(FastSearchError::new(
                    ErrorKind::SourceFailure,
                    "source parser produced a file hash inconsistent with canonical normalization",
                ));
            }
            snapshots.push(snapshot);
        }
        ensure_unique_snapshot_ids(&snapshots)?;
        Ok(IncrementalSnapshots {
            snapshots,
            seen_source_keys,
            decisions,
        })
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
    root_id: Option<&LogicalRootId>,
) -> Result<Vec<SourceSnapshot>, FastSearchError> {
    scan_sources(root)?
        .iter()
        .filter(|source| kind.is_none_or(|expected| source.kind == expected))
        .map(|source| {
            let parsed = parse_source(source.kind, &source.locator, &source.bytes, root_id);
            parsed.map_err(|error| {
                FastSearchError::new(
                    error.kind().clone(),
                    format!("{}: {}", source.locator, error.message()),
                )
            })
        })
        .collect()
}

fn parse_source(
    kind: ScannedSourceKind,
    locator: &str,
    bytes: &[u8],
    root_id: Option<&LogicalRootId>,
) -> Result<SourceSnapshot, FastSearchError> {
    match kind {
        ScannedSourceKind::Markdown => markdown::parse_with_root(locator, bytes, root_id),
    }
}

pub(super) fn normalize_document(document: &str) -> String {
    document
        .strip_prefix('\u{feff}')
        .unwrap_or(document)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

pub(super) fn normalized_file_hash(bytes: &[u8]) -> Result<FileHash, FastSearchError> {
    let document = std::str::from_utf8(bytes)
        .map_err(|_| FastSearchError::new(ErrorKind::SourceFailure, "source file is not UTF-8"))?;
    FileHash::parse(versioned_hash(
        "file:markdown-v2",
        [normalize_document(document).as_str()],
    ))
    .map_err(|error| source_contract_failure(error.message()))
}

pub(super) fn versioned_hash<'a>(scope: &str, fields: impl IntoIterator<Item = &'a str>) -> String {
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

pub(super) fn source_contract_failure(message: &str) -> FastSearchError {
    FastSearchError::new(ErrorKind::SourceFailure, message)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::domain::{ErrorKind, RecordKind, SourceSelector};
    use crate::ports::SourcePort;

    use super::{
        FilesystemSource, MarkdownAdmissionRule, ScannedSourceKind, markdown, markdown_snapshots,
        scan_sources, tsv,
    };

    static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn markdown_parser_accepts_admitted_content_without_filesystem_discovery() {
        let markdown = markdown::parse("docs/guide.md", b"# Guide\nbody")
            .expect("bounded Markdown content must parse without a filesystem path");

        assert_eq!(markdown.locator().path(), "docs/guide.md");
    }

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
            ["docs/alpha.md", "docs/zeta.md"],
            "scanner must admit Markdown only and sort it lexically"
        );
        assert!(
            scanned
                .iter()
                .all(|source| std::str::from_utf8(&source.bytes).is_ok())
        );
        assert_eq!(
            scanned.iter().map(|source| source.kind).collect::<Vec<_>>(),
            [ScannedSourceKind::Markdown, ScannedSourceKind::Markdown]
        );
    }

    #[test]
    fn scanner_excludes_all_tsv_files() {
        let fixture = Fixture::new();
        let generated_header = "id\tpath\tsummary\ttdr_coverage\ttdr_refs\twarnings\terrors";
        fixture.write(
            "Traceability/Paradigm Coverage Registry.tsv",
            &format!(
                "{generated_header}\nentry\tdocs/entry.md\tduplicated text\tdirect\tTDR-1\t\t"
            ),
        );
        fixture.write(
            "Traceability/Alignment Evidence Registry.tsv",
            "id\tpath\tevidence\nALIGN-1\tdocs/entry.md\tunique evidence",
        );
        fixture.write(
            "Reports/Manual Coverage Registry.tsv",
            &format!("{generated_header}\nentry\tdocs/entry.md\tmanual report\tdirect\tTDR-1\t\t"),
        );

        let scanned = scan_sources(fixture.path()).expect("valid registries must scan");
        let locators = scanned
            .iter()
            .map(|source| source.locator.as_str())
            .collect::<Vec<_>>();

        assert!(locators.is_empty());
    }

    #[test]
    fn incremental_snapshot_scan_parses_only_the_file_with_a_new_content_hash() {
        let fixture = Fixture::new();
        fixture.write("one.md", "# One\noriginal");
        fixture.write("two.md", "# Two\nunchanged");
        let source = FilesystemSource::new(fixture.path());
        let initial = source.snapshot().unwrap();
        let known = initial
            .iter()
            .map(|snapshot| {
                (
                    snapshot.storage_key(),
                    snapshot.file_hash().as_str().to_owned(),
                )
            })
            .collect();

        fixture.write("one.md", "# One\nchanged");
        let delta = source.snapshots_incremental(&known).unwrap();

        assert_eq!(delta.seen_source_keys.len(), 2);
        assert_eq!(delta.snapshots.len(), 1);
        assert_eq!(delta.snapshots[0].locator().path(), "one.md");
    }

    #[test]
    fn configured_frontmatter_property_is_a_fail_closed_admission_rule() {
        let fixture = Fixture::new();
        fixture.write(
            "included.md",
            "---\nИндексация: true\n---\n# Включён\nТекст",
        );
        fixture.write(
            "disabled.md",
            "---\nИндексация: false\n---\n# Исключён\nТекст",
        );
        fixture.write("missing.md", "# Без свойства\nТекст");
        let source = FilesystemSource::new(fixture.path()).with_markdown_admission(Some(
            MarkdownAdmissionRule::new("Индексация", "true").unwrap(),
        ));

        let scan = source.snapshots_incremental(&BTreeMap::new()).unwrap();

        assert_eq!(scan.snapshots.len(), 1);
        assert_eq!(scan.snapshots[0].locator().path(), "included.md");
        assert_eq!(
            scan.decisions.iter().filter(|item| item.included).count(),
            1
        );
        assert_eq!(
            scan.decisions.iter().filter(|item| !item.included).count(),
            2
        );
        assert!(
            !scan.snapshots[0].records()[0]
                .metadata()
                .contains_key("Индексация"),
            "control-plane admission metadata must not enter searchable records"
        );
    }

    #[test]
    fn scanner_rejects_non_utf8_allowed_source_without_partial_result() {
        let fixture = Fixture::new();
        fixture.write("allowed.md", "valid");
        fs::write(fixture.path().join("invalid.md"), [0xff, 0xfe]).expect("invalid UTF-8 fixture");

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
        fixture.write("registry.tsv", "id\ttitle\n2433\tignored");
        fixture.write(".gitignore", "*.md\n!keep.md\nnested/**\n");

        let scanned = scan_sources(fixture.path()).expect("standard gitignore rules must work");
        let locators = scanned
            .iter()
            .map(|source| source.locator.as_str())
            .collect::<Vec<_>>();

        assert_eq!(locators, ["keep.md"]);
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
            "sha256:v1:ae7ca6e4e3aebf9dd07e7927a5e74cd834419bcf8dbf9dc8f896bab299b192b6"
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
            "sha256:v1:78af24b5131bcee7cb7fe7751b171e0ff044fbd3c9c62819b42d12308be06580"
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
            "sha256:v1:0f3b21c0228c6c23c232a0f5f86fad079762353bf02162eb14e547b4c7c3afce"
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
    fn filesystem_source_excludes_tsv_records() {
        let fixture = Fixture::new();
        fixture.write("docs/guide.md", "# Guide\nMarkdown body");
        fixture.write(
            "registry.tsv",
            "id\ttitle\tstatus\n2433\tTechnical entry\tcurrent\n",
        );

        let source = FilesystemSource::new(fixture.path());
        let snapshots = source.snapshot().expect("Markdown fixture must parse");
        let records = source.records().expect("Markdown records must parse");

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].locator().path(), "docs/guide.md");
        assert_eq!(
            records
                .iter()
                .map(|record| record.id().as_str())
                .collect::<Vec<_>>(),
            ["markdown:docs/guide.md#Guide"]
        );
    }

    #[test]
    fn tsv_parser_rejects_malformed_input_without_partial_result() {
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
            let error = tsv::parse("registry.tsv", registry.as_bytes()).expect_err(name);

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
