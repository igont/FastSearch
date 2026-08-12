use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use super::{ErrorKind, FastSearchError};

/// Стабильный идентификатор записи, независимый от конкретного индекса.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StableId(String);

impl StableId {
    pub fn parse(value: impl Into<String>) -> Result<Self, FastSearchError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidIdentifier,
                "stable identifier must not be blank",
            ));
        }

        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable, user-selected root identity. Machine paths never enter public identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LogicalRootId(String);

impl LogicalRootId {
    pub fn parse(value: impl Into<String>) -> Result<Self, FastSearchError> {
        let value = value.into();
        if value.trim().is_empty()
            || value.contains(['/', '\\'])
            || value.chars().any(char::is_control)
        {
            return Err(FastSearchError::new(
                ErrorKind::InvalidIdentifier,
                "logical root identifier must be a nonblank path-free label",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Хеш содержимого; алгоритм и lifecycle намеренно уточняются evidence-спайком D3.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn parse(value: impl Into<String>) -> Result<Self, FastSearchError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "content hash must not be blank",
            ));
        }

        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Хеш исходного файла; не заменяет record hash канонической записи.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileHash(String);

impl FileHash {
    pub fn parse(value: impl Into<String>) -> Result<Self, FastSearchError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "file hash must not be blank",
            ));
        }
        Ok(Self(value))
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Вид источника одной канонической записи.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordKind {
    MarkdownSection,
    RegistryRow,
    CodeMap,
    CodeSymbol,
}

/// Точное положение записи внутри исходного файла.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceSelector {
    MarkdownHeading { heading_path: Vec<String> },
    RegistryRow { row: NonZeroUsize },
    CodeSymbol { symbol: String },
    WholeFile,
}

/// Путь к исходному файлу и его локатор, сохраняемые независимо от storage adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLocator {
    path: String,
    selector: SourceSelector,
}

impl SourceLocator {
    pub fn markdown<I, S>(path: impl Into<String>, headings: I) -> Result<Self, FastSearchError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let heading_path = headings.into_iter().map(Into::into).collect::<Vec<_>>();
        if heading_path.is_empty() || heading_path.iter().any(|heading| heading.trim().is_empty()) {
            return Err(FastSearchError::new(
                ErrorKind::InvalidLocator,
                "Markdown locator requires a non-empty heading path",
            ));
        }

        Self::new(path, SourceSelector::MarkdownHeading { heading_path })
    }

    pub fn registry_row(
        path: impl Into<String>,
        row: NonZeroUsize,
    ) -> Result<Self, FastSearchError> {
        Self::new(path, SourceSelector::RegistryRow { row })
    }

    pub fn code_symbol(
        path: impl Into<String>,
        symbol: impl Into<String>,
    ) -> Result<Self, FastSearchError> {
        let symbol = symbol.into();
        if symbol.trim().is_empty() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidLocator,
                "code symbol locator requires a symbol",
            ));
        }

        Self::new(path, SourceSelector::CodeSymbol { symbol })
    }

    pub fn whole_file(path: impl Into<String>) -> Result<Self, FastSearchError> {
        Self::new(path, SourceSelector::WholeFile)
    }

    fn new(path: impl Into<String>, selector: SourceSelector) -> Result<Self, FastSearchError> {
        let path = path.into().replace('\\', "/");
        if path.trim().is_empty()
            || path.starts_with('/')
            || path.contains(':')
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(FastSearchError::new(
                ErrorKind::InvalidLocator,
                "source locator must be a normalized relative path",
            ));
        }

        Ok(Self { path, selector })
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub const fn selector(&self) -> &SourceSelector {
        &self.selector
    }
}

/// Public named-root identity for a source locator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootedSourceLocator {
    root: LogicalRootId,
    locator: SourceLocator,
}

impl RootedSourceLocator {
    pub fn new(root: LogicalRootId, locator: SourceLocator) -> Result<Self, FastSearchError> {
        Ok(Self { root, locator })
    }

    #[must_use]
    pub const fn root(&self) -> &LogicalRootId {
        &self.root
    }

    #[must_use]
    pub const fn locator(&self) -> &SourceLocator {
        &self.locator
    }

    #[must_use]
    pub fn stable_id(&self) -> StableId {
        fn append_component(encoded: &mut String, value: &str) {
            encoded.push_str(&value.len().to_string());
            encoded.push(':');
            encoded.push_str(value);
        }
        let selector = match self.locator.selector() {
            SourceSelector::MarkdownHeading { heading_path } => {
                let mut encoded = String::from("markdown:");
                for heading in heading_path {
                    append_component(&mut encoded, heading);
                }
                encoded
            }
            SourceSelector::RegistryRow { row } => format!("registry:{}", row),
            SourceSelector::CodeSymbol { symbol } => format!("symbol:{symbol}"),
            SourceSelector::WholeFile => "file".to_owned(),
        };
        StableId(format!(
            "named-root-v1:{}:{}:{selector}",
            self.root.as_str(),
            self.locator.path()
        ))
    }
}

/// Classification is fixed before concrete map/code adapters are introduced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceAdmission {
    Markdown,
    Registry,
    CodeMap,
    CodeCandidate,
    Unsupported,
}

impl SourceAdmission {
    #[must_use]
    pub fn classify(path: &str) -> Self {
        if path.ends_with(".cfmap.md") {
            Self::CodeMap
        } else if path.ends_with(".md") {
            Self::Markdown
        } else if path.ends_with(".tsv") {
            Self::Registry
        } else if path.ends_with(".rs") || path.ends_with(".py") {
            Self::CodeCandidate
        } else {
            Self::Unsupported
        }
    }
}

/// Наблюдаемый снимок одного исходного файла до state/lexical projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSnapshot {
    root: Option<LogicalRootId>,
    locator: SourceLocator,
    file_hash: FileHash,
    records: Vec<CanonicalRecord>,
}

impl SourceSnapshot {
    #[must_use]
    pub fn new(locator: SourceLocator, file_hash: FileHash, records: Vec<CanonicalRecord>) -> Self {
        Self {
            root: None,
            locator,
            file_hash,
            records,
        }
    }
    #[must_use]
    pub fn for_root(
        root: LogicalRootId,
        locator: SourceLocator,
        file_hash: FileHash,
        records: Vec<CanonicalRecord>,
    ) -> Self {
        Self {
            root: Some(root),
            locator,
            file_hash,
            records,
        }
    }
    #[must_use]
    pub const fn root(&self) -> Option<&LogicalRootId> {
        self.root.as_ref()
    }
    #[must_use]
    pub const fn locator(&self) -> &SourceLocator {
        &self.locator
    }
    #[must_use]
    pub const fn file_hash(&self) -> &FileHash {
        &self.file_hash
    }
    #[must_use]
    pub fn records(&self) -> &[CanonicalRecord] {
        &self.records
    }
}

/// Единая индексируемая сущность для документов, реестров, карт и symbols.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalRecord {
    id: StableId,
    kind: RecordKind,
    locator: SourceLocator,
    title: String,
    searchable_content: String,
    metadata: BTreeMap<String, String>,
    relations: Vec<StableId>,
    content_hash: ContentHash,
}

impl CanonicalRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: StableId,
        kind: RecordKind,
        locator: SourceLocator,
        title: impl Into<String>,
        searchable_content: impl Into<String>,
        metadata: BTreeMap<String, String>,
        relations: Vec<StableId>,
        content_hash: ContentHash,
    ) -> Result<Self, FastSearchError> {
        let title = title.into();
        let searchable_content = searchable_content.into();
        if title.trim().is_empty() || searchable_content.trim().is_empty() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "canonical record requires title and searchable content",
            ));
        }
        if metadata.keys().any(|key| key.trim().is_empty()) {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "metadata keys must not be blank",
            ));
        }

        Ok(Self {
            id,
            kind,
            locator,
            title,
            searchable_content,
            metadata,
            relations,
            content_hash,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &StableId {
        &self.id
    }
    #[must_use]
    pub const fn kind(&self) -> RecordKind {
        self.kind
    }
    #[must_use]
    pub const fn locator(&self) -> &SourceLocator {
        &self.locator
    }
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
    #[must_use]
    pub fn searchable_content(&self) -> &str {
        &self.searchable_content
    }
    #[must_use]
    pub const fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }
    #[must_use]
    pub fn relations(&self) -> &[StableId] {
        &self.relations
    }
    #[must_use]
    pub const fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }
}
