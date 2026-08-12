use std::{collections::BTreeMap, sync::Arc, thread};

use fastsearch::{
    application::fusion::{ChannelCandidates, FusionCoordinator},
    domain::{
        BackendKind, CanonicalRecord, Capability, CapabilityStatus, ContentHash, IndexFreshness,
        ModelIdentity, ProjectionProvenance, RecordKind, RetrievalChannel, SearchHit, SearchMode,
        SearchQuery, SourceLocator, StableId,
    },
};

fn record(id: &str, title: &str) -> CanonicalRecord {
    CanonicalRecord::new(
        StableId::parse(id).unwrap(),
        RecordKind::MarkdownSection,
        SourceLocator::whole_file(format!("{title}.md")).unwrap(),
        title,
        title,
        BTreeMap::new(),
        Vec::new(),
        ContentHash::parse(format!("sha256:{id}")).unwrap(),
    )
    .unwrap()
}

fn hit(id: &str, channel: RetrievalChannel, raw_score: f64) -> SearchHit {
    SearchHit::new(record(id, id), channel, raw_score)
}

fn ids(response: &fastsearch::domain::SearchResponse) -> Vec<&str> {
    response
        .hits()
        .iter()
        .map(|hit| hit.record().id().as_str())
        .collect()
}

#[test]
fn exact_dominates_and_raw_channel_scores_are_never_compared() {
    let query = SearchQuery::new("Navigation contract", SearchMode::Balanced).unwrap();
    let response = FusionCoordinator::fuse(
        &query,
        vec![
            ChannelCandidates::new(
                RetrievalChannel::Exact,
                vec![hit("exact", RetrievalChannel::Exact, -1_000_000.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
            ChannelCandidates::new(
                RetrievalChannel::Vector,
                vec![hit("vector", RetrievalChannel::Vector, 1_000_000.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
        ],
        &[CapabilityStatus::available(
            Capability::VectorRetrieval,
            BackendKind::Real,
        )],
    );

    assert_eq!(ids(&response), ["exact", "vector"]);
    assert!(response.hits()[0].score() > response.hits()[1].score());
}

#[test]
fn modes_apply_declared_monotonic_channel_calibration() {
    let candidates = || {
        vec![
            ChannelCandidates::new(
                RetrievalChannel::Lexical,
                vec![hit("lexical", RetrievalChannel::Lexical, 0.1)],
                IndexFreshness::Current,
            )
            .unwrap(),
            ChannelCandidates::new(
                RetrievalChannel::CodeMap,
                vec![hit("map", RetrievalChannel::CodeMap, 999.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
        ]
    };
    let balanced = FusionCoordinator::fuse(
        &SearchQuery::new("navigation", SearchMode::Balanced).unwrap(),
        candidates(),
        &[],
    );
    let current = FusionCoordinator::fuse(
        &SearchQuery::new("navigation", SearchMode::Current).unwrap(),
        candidates(),
        &[],
    );
    let design = FusionCoordinator::fuse(
        &SearchQuery::new("navigation", SearchMode::Design).unwrap(),
        candidates(),
        &[],
    );

    assert_eq!(ids(&balanced), ["lexical", "map"]);
    assert_eq!(ids(&current), ["lexical", "map"]);
    assert_eq!(ids(&design), ["map", "lexical"]);
}

#[test]
fn stable_id_dedupe_preserves_vector_provenance_and_ties_use_source_key() {
    let model = ModelIdentity::new("e5", "revision", "a".repeat(64)).unwrap();
    let provenance = ProjectionProvenance::new(model, 7, 11);
    let duplicate = SearchHit::new(
        record("duplicate", "duplicate-vector"),
        RetrievalChannel::Vector,
        0.01,
    )
    .with_projection_provenance(provenance.clone());
    let response = FusionCoordinator::fuse(
        &SearchQuery::new("navigation", SearchMode::Balanced).unwrap(),
        vec![
            ChannelCandidates::new(
                RetrievalChannel::Lexical,
                vec![
                    hit("duplicate", RetrievalChannel::Lexical, 100.0),
                    hit("z-source", RetrievalChannel::Lexical, 1.0),
                ],
                IndexFreshness::Current,
            )
            .unwrap(),
            ChannelCandidates::new(
                RetrievalChannel::Vector,
                vec![duplicate, hit("a-source", RetrievalChannel::Vector, 1.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
        ],
        &[],
    );

    assert_eq!(response.hits().len(), 3);
    let duplicate = response
        .hits()
        .iter()
        .find(|hit| hit.record().id().as_str() == "duplicate")
        .unwrap();
    assert_eq!(duplicate.projection_provenance(), Some(&provenance));

    let tied = FusionCoordinator::fuse(
        &SearchQuery::new("navigation", SearchMode::Balanced).unwrap(),
        vec![
            ChannelCandidates::new(
                RetrievalChannel::Lexical,
                vec![
                    hit("a-source", RetrievalChannel::Lexical, -10.0),
                    hit("b-source", RetrievalChannel::Lexical, 10.0),
                ],
                IndexFreshness::Current,
            )
            .unwrap(),
        ],
        &[],
    );
    // Different ranks are not a tie. Equal calibrated ranks across channels are.
    assert_eq!(ids(&tied), ["a-source", "b-source"]);
    let equal = FusionCoordinator::fuse(
        &SearchQuery::new("navigation", SearchMode::Balanced).unwrap(),
        vec![
            ChannelCandidates::new(
                RetrievalChannel::CodeMap,
                vec![hit("b-source", RetrievalChannel::CodeMap, 999.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
            ChannelCandidates::new(
                RetrievalChannel::Symbol,
                vec![hit("a-source", RetrievalChannel::Symbol, -999.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
        ],
        &[],
    );
    assert_eq!(ids(&equal), ["a-source", "b-source"]);
}

#[test]
fn e1_quality_rows_preserve_all_admitted_selectors_without_must_not_or_duplicates() {
    let query = SearchQuery::new("stable navigation", SearchMode::Balanced).unwrap();
    let response = FusionCoordinator::fuse(
        &query,
        vec![
            ChannelCandidates::new(
                RetrievalChannel::Exact,
                vec![hit("Q01-navigation-contract", RetrievalChannel::Exact, 0.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
            ChannelCandidates::new(
                RetrievalChannel::Lexical,
                vec![hit("Q02-registry-row-1", RetrievalChannel::Lexical, 500.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
            ChannelCandidates::new(
                RetrievalChannel::Vector,
                vec![hit("Q03-paraphrase", RetrievalChannel::Vector, -500.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
            ChannelCandidates::new(
                RetrievalChannel::CodeMap,
                vec![hit("Q04-auto-map", RetrievalChannel::CodeMap, 8.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
            ChannelCandidates::new(
                RetrievalChannel::Symbol,
                vec![hit("Q05-stable-navigation", RetrievalChannel::Symbol, 7.0)],
                IndexFreshness::Current,
            )
            .unwrap(),
        ],
        &[],
    );
    let selectors = ids(&response);
    assert_eq!(selectors[0], "Q01-navigation-contract");
    for expected in [
        "Q01-navigation-contract",
        "Q02-registry-row-1",
        "Q03-paraphrase",
        "Q04-auto-map",
        "Q05-stable-navigation",
    ] {
        assert!(selectors.contains(&expected), "missing {expected}");
    }
    assert_eq!(selectors.len(), 5);
    assert!(!selectors.iter().any(|id| id.contains("must-not")));

    let no_hit = FusionCoordinator::fuse(
        &SearchQuery::new("Q06-missing", SearchMode::Balanced).unwrap(),
        Vec::new(),
        &[],
    );
    assert!(no_hit.hits().is_empty());
}

#[test]
fn unavailable_and_degraded_vector_fallback_is_truthful() {
    let query = SearchQuery::new("Navigation contract", SearchMode::Balanced).unwrap();
    let remaining = || {
        vec![
            ChannelCandidates::new(
                RetrievalChannel::Lexical,
                vec![hit("lexical", RetrievalChannel::Lexical, 0.7)],
                IndexFreshness::Current,
            )
            .unwrap(),
        ]
    };
    let unavailable = FusionCoordinator::fuse(
        &query,
        remaining(),
        &[CapabilityStatus::unavailable(
            Capability::VectorRetrieval,
            "provider is offline",
        )],
    );
    assert_eq!(ids(&unavailable), ["lexical"]);
    assert_eq!(unavailable.freshness(), IndexFreshness::Stale);

    let degraded = FusionCoordinator::fuse(
        &query,
        remaining(),
        &[CapabilityStatus::degraded(
            Capability::VectorRetrieval,
            "projection recovery required",
        )],
    );
    assert_eq!(ids(&degraded), ["lexical"]);
    assert_eq!(degraded.freshness(), IndexFreshness::Degraded);
}

#[test]
fn empty_input_is_empty_and_five_parallel_repeats_are_identical() {
    let query = SearchQuery::new("missing", SearchMode::Balanced).unwrap();
    let empty = FusionCoordinator::fuse(&query, Vec::new(), &[]);
    assert!(empty.hits().is_empty());

    let coordinator = Arc::new(query);
    let repeats = (0..5)
        .map(|_| {
            let query = Arc::clone(&coordinator);
            thread::spawn(move || {
                let response = FusionCoordinator::fuse(
                    &query,
                    vec![
                        ChannelCandidates::new(
                            RetrievalChannel::Vector,
                            vec![
                                hit("same", RetrievalChannel::Vector, 0.9),
                                hit("b", RetrievalChannel::Vector, 0.8),
                            ],
                            IndexFreshness::Current,
                        )
                        .unwrap(),
                        ChannelCandidates::new(
                            RetrievalChannel::Lexical,
                            vec![
                                hit("same", RetrievalChannel::Lexical, 12.0),
                                hit("a", RetrievalChannel::Lexical, 11.0),
                            ],
                            IndexFreshness::Current,
                        )
                        .unwrap(),
                    ],
                    &[],
                );
                response
                    .hits()
                    .iter()
                    .map(|hit| {
                        format!(
                            "{}|{:?}|{:.12}|{:?}",
                            hit.record().id().as_str(),
                            hit.channel(),
                            hit.score(),
                            hit.projection_provenance()
                        )
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let outputs = repeats
        .into_iter()
        .map(|repeat| repeat.join().unwrap())
        .collect::<Vec<_>>();
    assert!(outputs.windows(2).all(|window| window[0] == window[1]));
}
