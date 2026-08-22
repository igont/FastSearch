use std::collections::BTreeMap;

use fastsearch::{
    application::{
        DocumentStatus, ProjectScope, PublicSearchError, PublicSearchErrorCode,
        PublicSearchRequest, PublicSearchResponse, PublicSearchResult,
    },
    domain::{
        CanonicalRecord, ContentHash, RecordKind, RetrievalChannel, SearchHit, SearchMode,
        SearchResponse, SourceLocator, StableId,
    },
};
use serde_json::{Value, json};

fn request() -> PublicSearchRequest {
    PublicSearchRequest::new("  navigation  ", None, None).unwrap()
}

fn absolute_path(suffix: &str) -> String {
    if cfg!(windows) {
        format!("C:/workspace/docs/{suffix}.md")
    } else {
        format!("/workspace/docs/{suffix}.md")
    }
}

fn result(rank: usize, suffix: &str, content: impl Into<String>) -> PublicSearchResult {
    PublicSearchResult::new(
        rank,
        format!("Title {suffix}"),
        absolute_path(suffix),
        ProjectScope::General,
        DocumentStatus::Actual,
        content,
    )
    .unwrap()
}

fn record(id: &str, scope: Option<&str>, status: Option<&str>) -> CanonicalRecord {
    let mut metadata = BTreeMap::new();
    if let Some(scope) = scope {
        metadata.insert("project_scope".to_owned(), scope.to_owned());
    }
    if let Some(status) = status {
        metadata.insert("document_status".to_owned(), status.to_owned());
    }
    CanonicalRecord::new(
        StableId::parse(id).unwrap(),
        RecordKind::MarkdownSection,
        SourceLocator::markdown(format!("docs/{id}.md"), ["Heading"]).unwrap(),
        format!("Title {id}"),
        format!("Content {id}"),
        metadata,
        Vec::new(),
        ContentHash::parse(format!("hash-{id}")).unwrap(),
    )
    .unwrap()
}

#[test]
fn request_preserves_original_query_but_validates_and_searches_trimmed_balanced_text() {
    let request = request();
    assert_eq!(request.query(), "  navigation  ");
    let internal = request.to_search_query().unwrap();
    assert_eq!(internal.text(), "navigation");
    assert_eq!(internal.mode(), SearchMode::Balanced);

    let wire = serde_json::to_vec(&request).unwrap();
    let roundtrip: PublicSearchRequest = serde_json::from_slice(&wire).unwrap();
    assert_eq!(roundtrip, request);
    assert_eq!(roundtrip.query(), "  navigation  ");
    assert_eq!(roundtrip.to_search_query().unwrap().text(), "navigation");
    let response = PublicSearchResponse::from_ranked_results(&roundtrip, []).unwrap();
    assert_eq!(response.query(), "  navigation  ");
    assert_eq!(
        serde_json::to_value(response).unwrap()["query"],
        "  navigation  "
    );

    let boundary = format!("  {}  ", "я".repeat(1024));
    assert!(PublicSearchRequest::new(boundary, None, None).is_ok());
    let error =
        PublicSearchRequest::new(format!("  {}  ", "я".repeat(1025)), None, None).unwrap_err();
    assert_eq!(error.code(), PublicSearchErrorCode::InvalidRequest);
    assert!(!error.retryable());
    assert!(PublicSearchRequest::new(" \n\t ", None, None).is_err());
}

#[test]
fn request_wire_contract_has_only_exact_fields_and_filter_values() {
    let request = PublicSearchRequest::new(
        "query",
        Some(ProjectScope::Future),
        Some(DocumentStatus::Draft),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({
            "query": "query",
            "project_scope": "future",
            "document_status": "draft"
        })
    );
    assert!(
        serde_json::from_value::<PublicSearchRequest>(json!({
            "query": "query",
            "project_scope": "CURRENT"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<PublicSearchRequest>(json!({
            "query": "query",
            "mode": "current"
        }))
        .is_err()
    );
    let error = PublicSearchRequest::from_wire_values("query", Some("CURRENT"), None).unwrap_err();
    assert_eq!(error.code(), PublicSearchErrorCode::InvalidRequest);
}

#[test]
fn absent_filters_admit_all_valid_metadata_and_missing_metadata_uses_defaults() {
    let unfiltered = request();
    assert!(
        unfiltered
            .admits(&record("future", Some("future"), Some("draft")))
            .unwrap()
    );

    let filtered = PublicSearchRequest::new(
        "navigation",
        Some(ProjectScope::General),
        Some(DocumentStatus::Actual),
    )
    .unwrap();
    assert!(filtered.admits(&record("defaults", None, None)).unwrap());
    assert!(
        !filtered
            .admits(&record("draft", None, Some("draft")))
            .unwrap()
    );

    let error = unfiltered
        .admits(&record("invalid", Some("experimental"), None))
        .unwrap_err();
    assert_eq!(error.code(), PublicSearchErrorCode::SearchFailed);
}

#[test]
fn internal_response_maps_to_exact_public_fields_filters_and_sequential_ranks() {
    let records = [
        record("current", Some("current"), Some("actual")),
        record("future", Some("future"), Some("actual")),
    ];
    let internal = SearchResponse::new(
        records
            .iter()
            .cloned()
            .map(|record| SearchHit::new(record, RetrievalChannel::Vector, 0.75))
            .collect(),
    );
    let request = PublicSearchRequest::new(
        "navigation",
        Some(ProjectScope::Future),
        Some(DocumentStatus::Actual),
    )
    .unwrap();
    let response = PublicSearchResponse::from_internal(&request, &internal, |record| {
        let prefix = if cfg!(windows) {
            "C:/workspace"
        } else {
            "/workspace"
        };
        Ok(format!("{prefix}/{}", record.locator().path()))
    })
    .unwrap();

    assert_eq!(response.query(), "navigation");
    assert_eq!(response.count(), 1);
    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(
        value,
        json!({
            "query": "navigation",
            "count": 1,
            "results": [{
                "rank": 1,
                "title": "Title future",
            "path": absolute_path("future"),
                "project_scope": "future",
                "document_status": "actual",
                "content": "Content future"
            }]
        })
    );
    let object = value["results"][0].as_object().unwrap();
    assert_eq!(object.len(), 6, "internal score/provenance must not leak");
}

#[test]
fn response_returns_only_first_six_and_reassigns_ranks() {
    let response = PublicSearchResponse::from_ranked_results(
        &request(),
        (0..8).map(|index| result(99, &index.to_string(), "content")),
    )
    .unwrap();
    assert_eq!(response.count(), 6);
    assert_eq!(
        response
            .results()
            .iter()
            .map(PublicSearchResult::rank)
            .collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6]
    );
    assert_eq!(response.results()[5].title(), "Title 5");
}

#[test]
fn oversized_response_fails_atomically_without_partial_list() {
    let error = PublicSearchResponse::from_ranked_results(
        &request(),
        [result(1, "large", "x".repeat(131_072))],
    )
    .unwrap_err();
    assert_eq!(error.code(), PublicSearchErrorCode::ResponseTooLarge);
    assert_eq!(error.message(), "The search response is too large.");
    assert!(!error.retryable());
}

#[test]
fn every_public_error_has_stable_code_message_retryability_and_hides_domain_cause() {
    let cases = [
        PublicSearchError::invalid_request("C:/secret/request.txt"),
        PublicSearchError::not_ready("C:/secret/model.bin"),
        PublicSearchError::stale_index("generation 41 != 42"),
        PublicSearchError::busy("queue contains private request text"),
        PublicSearchError::timeout("internal stage qwen"),
        PublicSearchError::response_too_large("serialized 999999 bytes"),
        PublicSearchError::search_failed("stack and neighboring record"),
    ];
    let expected = [
        ("INVALID_REQUEST", "The search request is invalid.", false),
        ("NOT_READY", "Search is not ready.", true),
        ("STALE_INDEX", "The search index is stale.", true),
        ("BUSY", "Search is busy.", true),
        ("TIMEOUT", "The search timed out.", true),
        (
            "RESPONSE_TOO_LARGE",
            "The search response is too large.",
            false,
        ),
        ("SEARCH_FAILED", "The search failed.", true),
    ];

    for (error, (code, message, retryable)) in cases.iter().zip(expected) {
        let value = serde_json::to_value(error).unwrap();
        assert_eq!(
            value,
            json!({"code": code, "message": message, "retryable": retryable})
        );
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert!(error.domain_cause().is_some());
        assert!(!value.to_string().contains("secret"));
        assert!(!value.to_string().contains("qwen"));
    }
}

#[test]
fn public_result_rejects_non_absolute_internal_or_unnormalized_paths() {
    let platform_double_separator = if cfg!(windows) {
        "C:/workspace//docs/guide.md"
    } else {
        "/workspace//docs/guide.md"
    };
    let platform_trailing_separator = if cfg!(windows) {
        "C:/workspace/docs/"
    } else {
        "/workspace/docs/"
    };
    let platform_internal_path = if cfg!(windows) {
        "C:/workspace/.FASTSEARCH/index/record.md"
    } else {
        "/workspace/.fastsearch/index/record.md"
    };
    let platform_parent = if cfg!(windows) {
        "C:/workspace/docs/../secret.md"
    } else {
        "/workspace/docs/../secret.md"
    };
    let platform_current = if cfg!(windows) {
        "C:/workspace/./docs/guide.md"
    } else {
        "/workspace/./docs/guide.md"
    };
    for path in [
        "docs/guide.md",
        "C:\\workspace\\docs\\guide.md",
        platform_double_separator,
        platform_trailing_separator,
        platform_internal_path,
        platform_parent,
        platform_current,
    ] {
        let error = PublicSearchResult::new(
            1,
            "Guide",
            path,
            ProjectScope::General,
            DocumentStatus::Actual,
            "content",
        )
        .unwrap_err();
        assert_eq!(error.code(), PublicSearchErrorCode::SearchFailed);
    }
}

#[test]
fn every_public_result_creation_path_uses_the_same_path_validation() {
    let invalid = if cfg!(windows) {
        "C:/workspace//docs/guide.md"
    } else {
        "/workspace//docs/guide.md"
    };
    let record = record("guide", None, None);
    assert!(
        PublicSearchResult::from_record(1, &record, invalid)
            .unwrap_err()
            .domain_cause()
            .unwrap()
            .contains("normalized")
    );

    let internal = SearchResponse::new(vec![SearchHit::new(record, RetrievalChannel::Vector, 1.0)]);
    let error =
        PublicSearchResponse::from_internal(&request(), &internal, |_| Ok(invalid.to_owned()))
            .unwrap_err();
    assert_eq!(error.code(), PublicSearchErrorCode::SearchFailed);
}

#[cfg(windows)]
#[test]
fn windows_rejects_internal_component_case_insensitively() {
    let error = PublicSearchResult::new(
        1,
        "Guide",
        "C:/workspace/.FASTSEARCH/index/record.md",
        ProjectScope::General,
        DocumentStatus::Actual,
        "content",
    )
    .unwrap_err();
    assert_eq!(error.code(), PublicSearchErrorCode::SearchFailed);
}

#[cfg(windows)]
#[test]
fn windows_accepts_normalized_unc_source_path() {
    let result = PublicSearchResult::new(
        1,
        "Guide",
        "//server/share/docs/guide.md",
        ProjectScope::General,
        DocumentStatus::Actual,
        "content",
    )
    .unwrap();
    assert_eq!(result.path(), "//server/share/docs/guide.md");
}

#[cfg(windows)]
#[test]
fn windows_rejects_malformed_unc_source_paths() {
    for path in [
        "//server",
        "///server/share/docs/guide.md",
        "//server//docs/guide.md",
        "//server/share/docs/",
        "//server/share/.FASTSEARCH/index/record.md",
    ] {
        let error = PublicSearchResult::new(
            1,
            "Guide",
            path,
            ProjectScope::General,
            DocumentStatus::Actual,
            "content",
        )
        .unwrap_err();
        assert_eq!(error.code(), PublicSearchErrorCode::SearchFailed, "{path}");
    }
}

#[test]
fn bounded_serializer_accepts_exact_contract_shape_below_limit() {
    let response = PublicSearchResponse::from_ranked_results(
        &request(),
        [result(1, "small", "bounded content")],
    )
    .unwrap();
    let bytes = response.serialize_bounded().unwrap();
    assert!(bytes.len() <= 131_072);
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["count"], 1);
}
