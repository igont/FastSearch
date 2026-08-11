use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use crate::domain::{
    BackendKind, CanonicalRecord, Capability, CapabilityState, CapabilityStatus, ContentHash,
    ErrorKind, FastSearchError, RecordKind, SearchMode, SearchQuery, SourceLocator, SourceSelector,
    StableId,
};
use crate::ports::{
    AgentSurface, CodeMapPort, LexicalRetrieval, SourcePort, StateStore, SymbolPort,
    VectorRetrieval,
};

#[test]
fn ports_are_object_safe_boundaries() {
    let source: Option<&dyn SourcePort> = None;
    let state: Option<&dyn StateStore> = None;
    let lexical: Option<&dyn LexicalRetrieval> = None;
    let vector: Option<&dyn VectorRetrieval> = None;
    let maps: Option<&dyn CodeMapPort> = None;
    let symbols: Option<&dyn SymbolPort> = None;
    let agent: Option<&dyn AgentSurface> = None;

    assert!(
        source.is_none()
            && state.is_none()
            && lexical.is_none()
            && vector.is_none()
            && maps.is_none()
            && symbols.is_none()
            && agent.is_none()
    );
}

#[test]
fn canonical_record_preserves_identity_locator_content_metadata_relations_and_hash() {
    let record = CanonicalRecord::new(
        StableId::parse("TDR-42#search-boundary").expect("valid stable identifier"),
        RecordKind::MarkdownSection,
        SourceLocator::markdown("docs/search.md", ["Search", "Boundary"])
            .expect("valid Markdown locator"),
        "Search boundary",
        "Exact lookup is distinct from full-text retrieval.",
        BTreeMap::from([("alignment".into(), "A1".into())]),
        vec![StableId::parse("TDR-17").expect("valid relation")],
        ContentHash::parse("sha256:abc123").expect("valid content hash"),
    )
    .expect("complete canonical record");

    assert_eq!(record.id().as_str(), "TDR-42#search-boundary");
    assert_eq!(record.locator().path(), "docs/search.md");
    assert_eq!(
        record.searchable_content(),
        "Exact lookup is distinct from full-text retrieval."
    );
    assert_eq!(record.metadata().get("alignment"), Some(&"A1".to_owned()));
    assert_eq!(record.relations()[0].as_str(), "TDR-17");
    assert_eq!(record.content_hash().as_str(), "sha256:abc123");
}

#[test]
fn registry_rows_and_code_symbols_have_precise_source_locators() {
    let row = SourceLocator::registry_row("registry.tsv", NonZeroUsize::new(7).expect("non-zero"))
        .expect("valid TSV locator");
    let symbol = SourceLocator::code_symbol("src/lib.rs", "fastsearch::scaffold_status")
        .expect("valid symbol locator");

    assert_eq!(row.path(), "registry.tsv");
    assert_eq!(symbol.path(), "src/lib.rs");
    assert!(matches!(
        row.selector(),
        SourceSelector::RegistryRow { row } if row.get() == 7
    ));
    assert!(matches!(
        symbol.selector(),
        SourceSelector::CodeSymbol { symbol } if symbol == "fastsearch::scaffold_status"
    ));
}

#[test]
fn search_modes_and_capability_status_do_not_conflate_mock_unavailable_and_real() {
    assert_eq!(SearchMode::default(), SearchMode::Balanced);
    assert_eq!(
        SearchQuery::new("worker scheduling", SearchMode::Current)
            .expect("query")
            .mode(),
        SearchMode::Current
    );

    let mock = CapabilityStatus::available(Capability::LexicalRetrieval, BackendKind::Mock);
    let real = CapabilityStatus::available(Capability::LexicalRetrieval, BackendKind::Real);
    let unavailable =
        CapabilityStatus::unavailable(Capability::VectorRetrieval, "provider is not configured");

    assert_ne!(mock.state(), real.state());
    assert!(matches!(
        mock.state(),
        CapabilityState::Available {
            backend: BackendKind::Mock
        }
    ));
    assert!(matches!(
        unavailable.state(),
        CapabilityState::Unavailable { .. }
    ));
}

#[test]
fn invalid_identifiers_unsupported_sources_and_unavailable_capabilities_are_structured_errors() {
    let invalid_identifier = StableId::parse("   ").expect_err("blank identifier must fail");
    let unsupported_source = FastSearchError::unsupported_source(RecordKind::CodeSymbol);
    let unavailable = CapabilityStatus::unavailable(Capability::VectorRetrieval, "disabled")
        .require_available()
        .expect_err("unavailable capability must fail");

    assert!(matches!(
        invalid_identifier.kind(),
        ErrorKind::InvalidIdentifier
    ));
    assert!(matches!(
        unsupported_source.kind(),
        ErrorKind::UnsupportedSource {
            kind: RecordKind::CodeSymbol
        }
    ));
    assert!(matches!(
        unavailable.kind(),
        ErrorKind::CapabilityUnavailable { .. }
    ));
}
