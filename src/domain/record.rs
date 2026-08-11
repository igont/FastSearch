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
        let path = path.into();
        if path.trim().is_empty() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidLocator,
                "source path must not be blank",
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

/// Наблюдаемый снимок одного исходного файла до state/lexical projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSnapshot {
    locator: SourceLocator,
    file_hash: FileHash,
    records: Vec<CanonicalRecord>,
}

impl SourceSnapshot {
    #[must_use]
    pub const fn new(
        locator: SourceLocator,
        file_hash: FileHash,
        records: Vec<CanonicalRecord>,
    ) -> Self {
        Self {
            locator,
            file_hash,
            records,
        }
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
