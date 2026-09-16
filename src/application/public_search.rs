use std::path::Path;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::domain::{CanonicalRecord, SearchMode, SearchQuery, SearchResponse};

pub const MAX_PUBLIC_QUERY_CHARS: usize = 1024;
pub const MAX_PUBLIC_RESULTS: usize = 5;
pub const MAX_SERIALIZED_RESPONSE_BYTES: usize = 131_072;

/// Public project-state filter. These are the only values accepted at the boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectScope {
    Current,
    Future,
    General,
}

impl ProjectScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Future => "future",
            Self::General => "general",
        }
    }
}

impl FromStr for ProjectScope {
    type Err = PublicSearchError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "current" => Ok(Self::Current),
            "future" => Ok(Self::Future),
            "general" => Ok(Self::General),
            _ => Err(PublicSearchError::invalid_request(
                "unknown project_scope filter",
            )),
        }
    }
}

/// Public document-lifecycle filter. These are the only values accepted at the boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    Draft,
    Actual,
}

impl DocumentStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Actual => "actual",
        }
    }
}

impl FromStr for DocumentStatus {
    type Err = PublicSearchError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "actual" => Ok(Self::Actual),
            _ => Err(PublicSearchError::invalid_request(
                "unknown document_status filter",
            )),
        }
    }
}

/// Validated public request shared by machine-facing adapters.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PublicSearchRequest {
    query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_scope: Option<ProjectScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    document_status: Option<DocumentStatus>,
}

impl PublicSearchRequest {
    pub fn from_wire_values(
        query: impl Into<String>,
        project_scope: Option<&str>,
        document_status: Option<&str>,
    ) -> Result<Self, PublicSearchError> {
        let project_scope = project_scope.map(str::parse).transpose()?;
        let document_status = document_status.map(str::parse).transpose()?;
        Self::new(query, project_scope, document_status)
    }

    pub fn new(
        query: impl Into<String>,
        project_scope: Option<ProjectScope>,
        document_status: Option<DocumentStatus>,
    ) -> Result<Self, PublicSearchError> {
        let query = query.into();
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Err(PublicSearchError::invalid_request(
                "search query is blank after trimming",
            ));
        }
        if trimmed_query.chars().count() > MAX_PUBLIC_QUERY_CHARS {
            return Err(PublicSearchError::invalid_request(
                "search query exceeds 1024 Unicode scalar values",
            ));
        }

        Ok(Self {
            query,
            project_scope,
            document_status,
        })
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    #[must_use]
    pub const fn project_scope(&self) -> Option<ProjectScope> {
        self.project_scope
    }

    #[must_use]
    pub const fn document_status(&self) -> Option<DocumentStatus> {
        self.document_status
    }

    /// Converts the public request without exposing an internal mode selector.
    pub fn to_search_query(&self) -> Result<SearchQuery, PublicSearchError> {
        SearchQuery::new(self.query.trim(), SearchMode::Balanced)
            .map_err(|error| PublicSearchError::invalid_request(error.message()))
    }

    /// Applies only filters that were explicitly present in the public request.
    pub fn admits(&self, record: &CanonicalRecord) -> Result<bool, PublicSearchError> {
        let (scope, status) = effective_metadata(record)?;
        Ok(self.project_scope.is_none_or(|expected| expected == scope)
            && self
                .document_status
                .is_none_or(|expected| expected == status))
    }
}

impl<'de> Deserialize<'de> for PublicSearchRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireRequest {
            query: String,
            project_scope: Option<ProjectScope>,
            document_status: Option<DocumentStatus>,
        }

        let wire = WireRequest::deserialize(deserializer)?;
        Self::new(wire.query, wire.project_scope, wire.document_status)
            .map_err(serde::de::Error::custom)
    }
}

/// One public ranked fragment. Internal scores and provenance deliberately have no fields here.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PublicSearchResult {
    rank: usize,
    title: String,
    path: String,
    project_scope: ProjectScope,
    document_status: DocumentStatus,
    content: String,
}

impl PublicSearchResult {
    pub fn new(
        rank: usize,
        title: impl Into<String>,
        path: impl Into<String>,
        project_scope: ProjectScope,
        document_status: DocumentStatus,
        content: impl Into<String>,
    ) -> Result<Self, PublicSearchError> {
        let title = title.into();
        let path = path.into();
        let content = content.into();
        if rank == 0 || title.trim().is_empty() || content.trim().is_empty() {
            return Err(PublicSearchError::search_failed(
                "public result has an invalid rank or blank content",
            ));
        }
        validate_public_source_path(&path)?;

        Ok(Self {
            rank,
            title,
            path,
            project_scope,
            document_status,
            content,
        })
    }

    pub fn from_record(
        rank: usize,
        record: &CanonicalRecord,
        normalized_absolute_path: impl Into<String>,
    ) -> Result<Self, PublicSearchError> {
        let (scope, status) = effective_metadata(record)?;
        Self::new(
            rank,
            record.title(),
            normalized_absolute_path,
            scope,
            status,
            record.searchable_content(),
        )
    }

    #[must_use]
    pub const fn rank(&self) -> usize {
        self.rank
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub const fn project_scope(&self) -> ProjectScope {
        self.project_scope
    }

    #[must_use]
    pub const fn document_status(&self) -> DocumentStatus {
        self.document_status
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// Public response with the canonical five-result and byte-size limits applied atomically.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PublicSearchResponse {
    query: String,
    count: usize,
    results: Vec<PublicSearchResult>,
}

impl PublicSearchResponse {
    pub fn from_ranked_results(
        request: &PublicSearchRequest,
        results: impl IntoIterator<Item = PublicSearchResult>,
    ) -> Result<Self, PublicSearchError> {
        let mut results = results
            .into_iter()
            .take(MAX_PUBLIC_RESULTS)
            .collect::<Vec<_>>();
        for (index, result) in results.iter_mut().enumerate() {
            result.rank = index + 1;
        }
        let response = Self {
            query: request.query.clone(),
            count: results.len(),
            results,
        };
        response.serialize_bounded()?;
        Ok(response)
    }

    pub fn from_internal<F>(
        request: &PublicSearchRequest,
        response: &SearchResponse,
        mut resolve_path: F,
    ) -> Result<Self, PublicSearchError>
    where
        F: FnMut(&CanonicalRecord) -> Result<String, PublicSearchError>,
    {
        let mut results = Vec::new();
        for hit in response.hits() {
            if !request.admits(hit.record())? {
                continue;
            }
            let rank = results.len() + 1;
            results.push(PublicSearchResult::from_record(
                rank,
                hit.record(),
                resolve_path(hit.record())?,
            )?);
            if results.len() == MAX_PUBLIC_RESULTS {
                break;
            }
        }
        Self::from_ranked_results(request, results)
    }

    pub fn serialize_bounded(&self) -> Result<Vec<u8>, PublicSearchError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| PublicSearchError::search_failed(error.to_string()))?;
        if bytes.len() > MAX_SERIALIZED_RESPONSE_BYTES {
            return Err(PublicSearchError::response_too_large(format!(
                "serialized response is {} bytes",
                bytes.len()
            )));
        }
        Ok(bytes)
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    #[must_use]
    pub fn results(&self) -> &[PublicSearchResult] {
        &self.results
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PublicSearchErrorCode {
    InvalidRequest,
    NotReady,
    StaleIndex,
    Busy,
    Timeout,
    ResponseTooLarge,
    SearchFailed,
}

/// Safe public error. The private domain cause is retained for diagnostics but never serialized.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PublicSearchError {
    code: PublicSearchErrorCode,
    message: String,
    retryable: bool,
    #[serde(skip)]
    #[schemars(skip)]
    domain_cause: Option<String>,
}

impl PublicSearchError {
    #[must_use]
    pub fn invalid_request(cause: impl Into<String>) -> Self {
        Self::new(PublicSearchErrorCode::InvalidRequest, cause)
    }

    #[must_use]
    pub fn not_ready(cause: impl Into<String>) -> Self {
        Self::new(PublicSearchErrorCode::NotReady, cause)
    }

    #[must_use]
    pub fn stale_index(cause: impl Into<String>) -> Self {
        Self::new(PublicSearchErrorCode::StaleIndex, cause)
    }

    #[must_use]
    pub fn busy(cause: impl Into<String>) -> Self {
        Self::new(PublicSearchErrorCode::Busy, cause)
    }

    #[must_use]
    pub fn timeout(cause: impl Into<String>) -> Self {
        Self::new(PublicSearchErrorCode::Timeout, cause)
    }

    #[must_use]
    pub fn response_too_large(cause: impl Into<String>) -> Self {
        Self::new(PublicSearchErrorCode::ResponseTooLarge, cause)
    }

    #[must_use]
    pub fn search_failed(cause: impl Into<String>) -> Self {
        Self::new(PublicSearchErrorCode::SearchFailed, cause)
    }

    fn new(code: PublicSearchErrorCode, cause: impl Into<String>) -> Self {
        let (message, retryable) = match code {
            PublicSearchErrorCode::InvalidRequest => ("The search request is invalid.", false),
            PublicSearchErrorCode::NotReady => ("Search is not ready.", true),
            PublicSearchErrorCode::StaleIndex => ("The search index is stale.", true),
            PublicSearchErrorCode::Busy => ("Search is busy.", true),
            PublicSearchErrorCode::Timeout => ("The search timed out.", true),
            PublicSearchErrorCode::ResponseTooLarge => ("The search response is too large.", false),
            PublicSearchErrorCode::SearchFailed => ("The search failed.", true),
        };
        Self {
            code,
            message: message.to_owned(),
            retryable,
            domain_cause: Some(cause.into()),
        }
    }

    #[must_use]
    pub const fn code(&self) -> PublicSearchErrorCode {
        self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    #[must_use]
    pub fn domain_cause(&self) -> Option<&str> {
        self.domain_cause.as_deref()
    }
}

impl std::fmt::Display for PublicSearchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PublicSearchError {}

fn validate_public_source_path(path: &str) -> Result<(), PublicSearchError> {
    let Some(components) = normalized_public_path_components(path) else {
        return Err(PublicSearchError::search_failed(
            "source path is not normalized and absolute",
        ));
    };

    let internal_component = components.into_iter().any(|component| {
        #[cfg(windows)]
        {
            component.eq_ignore_ascii_case(".fastsearch")
        }
        #[cfg(not(windows))]
        {
            component == ".fastsearch"
        }
    });
    if internal_component {
        return Err(PublicSearchError::search_failed(
            "source path is not admissible for a public result",
        ));
    }

    Ok(())
}

fn valid_public_path_component(component: &str) -> bool {
    !component.is_empty() && component != "." && component != ".."
}

#[cfg(windows)]
fn normalized_public_path_components(path: &str) -> Option<Vec<&str>> {
    if path.contains('\\') || path.ends_with('/') || !Path::new(path).is_absolute() {
        return None;
    }

    if let Some(unc_path) = path.strip_prefix("//") {
        let components = unc_path.split('/').collect::<Vec<_>>();
        if components.len() < 2 || !components.iter().copied().all(valid_public_path_component) {
            return None;
        }
        return Some(components);
    }

    if path.contains("//") {
        return None;
    }
    let components = path.split('/').collect::<Vec<_>>();
    components
        .iter()
        .copied()
        .all(valid_public_path_component)
        .then_some(components)
}

#[cfg(not(windows))]
fn normalized_public_path_components(path: &str) -> Option<Vec<&str>> {
    if path.contains('\\')
        || path.ends_with('/')
        || path.contains("//")
        || !Path::new(path).is_absolute()
    {
        return None;
    }

    let components = path.strip_prefix('/')?.split('/').collect::<Vec<_>>();
    components
        .iter()
        .copied()
        .all(valid_public_path_component)
        .then_some(components)
}

fn effective_metadata(
    record: &CanonicalRecord,
) -> Result<(ProjectScope, DocumentStatus), PublicSearchError> {
    let scope =
        record
            .metadata()
            .get("project_scope")
            .map_or(Ok(ProjectScope::General), |value| {
                value.parse().map_err(|_| {
                    PublicSearchError::search_failed("record has invalid project_scope metadata")
                })
            })?;
    let status =
        record
            .metadata()
            .get("document_status")
            .map_or(Ok(DocumentStatus::Actual), |value| {
                value.parse().map_err(|_| {
                    PublicSearchError::search_failed("record has invalid document_status metadata")
                })
            })?;
    Ok((scope, status))
}
