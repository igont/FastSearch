use std::{collections::BTreeMap, num::NonZeroUsize, sync::Arc, thread};

use fastsearch::{
    application::fusion::{ChannelCandidates, FusionCoordinator},
    domain::{
        BackendKind, CanonicalRecord, Capability, CapabilityStatus, ContentHash, IndexFreshness,
        LogicalRootId, ModelIdentity, ProjectionProvenance, RecordKind, RetrievalChannel,
        RootedSourceLocator, SearchHit, SearchMode, SearchQuery, SourceLocator, SourceSelector,
        StableId,
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
fn fusion_smoke_preserves_one_representative_of_each_admitted_channel() {
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

#[derive(Clone, Debug)]
struct QualityRow {
    label: String,
    intent: String,
    section: String,
    logical_root_id: String,
    relative_locator: String,
    selector_kind: String,
    selector_value: String,
    top_k: usize,
    required_rank_max: usize,
    must_not: String,
    review: String,
}

fn quality_rows() -> Vec<QualityRow> {
    let rows = include_str!("../evidence/dt3/foundation/queries.tsv")
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields = line.split('\t').collect::<Vec<_>>();
            assert_eq!(fields.len(), 11, "invalid immutable quality row: {line}");
            QualityRow {
                label: fields[0].to_owned(),
                intent: fields[1].to_owned(),
                section: fields[2].to_owned(),
                logical_root_id: fields[3].to_owned(),
                relative_locator: fields[4].to_owned(),
                selector_kind: fields[5].to_owned(),
                selector_value: fields[6].to_owned(),
                top_k: fields[7].parse().unwrap(),
                required_rank_max: fields[8].parse().unwrap(),
                must_not: fields[9].to_owned(),
                review: fields[10].to_owned(),
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 24, "all immutable A1 rows must be present");
    assert!(rows.iter().all(|row| row.review == "READY"));
    assert!(rows.iter().any(|row| row.section == "C1/C2"));
    assert!(rows.iter().any(|row| row.section == "D1/D2"));
    rows
}

fn row_locator(row: &QualityRow) -> SourceLocator {
    match row.selector_kind.as_str() {
        "heading" => {
            SourceLocator::markdown(row.relative_locator.clone(), [row.selector_value.clone()])
                .unwrap()
        }
        "row" => SourceLocator::registry_row(
            row.relative_locator.clone(),
            NonZeroUsize::new(row.selector_value.parse().unwrap()).unwrap(),
        )
        .unwrap(),
        "symbol" => {
            SourceLocator::code_symbol(row.relative_locator.clone(), row.selector_value.clone())
                .unwrap()
        }
        "locator" => SourceLocator::whole_file(row.relative_locator.clone()).unwrap(),
        other => panic!("unsupported immutable selector kind {other}"),
    }
}

fn row_channel(row: &QualityRow) -> RetrievalChannel {
    match row.intent.as_str() {
        "exact" => RetrievalChannel::Exact,
        "lexical" | "vector-unavailable" => RetrievalChannel::Lexical,
        "paraphrase" => RetrievalChannel::Vector,
        "doc-map" => RetrievalChannel::CodeMap,
        "symbol" => RetrievalChannel::Symbol,
        "no-hit" => RetrievalChannel::Lexical,
        other => panic!("unsupported immutable intent {other}"),
    }
}

fn row_record(row: &QualityRow) -> CanonicalRecord {
    let locator = row_locator(row);
    let id = RootedSourceLocator::new(
        LogicalRootId::parse(row.logical_root_id.clone()).unwrap(),
        locator.clone(),
    )
    .unwrap()
    .stable_id();
    let kind = match row.selector_kind.as_str() {
        "row" => RecordKind::RegistryRow,
        "symbol" => RecordKind::CodeSymbol,
        _ if row.intent == "doc-map" => RecordKind::CodeMap,
        _ => RecordKind::MarkdownSection,
    };
    CanonicalRecord::new(
        id,
        kind,
        locator,
        row.selector_value.clone(),
        format!("{} {}", row.label, row.selector_value),
        BTreeMap::new(),
        Vec::new(),
        ContentHash::parse(format!("sha256:{}", row.label)).unwrap(),
    )
    .unwrap()
}

fn response_for_quality_row(row: &QualityRow) -> fastsearch::domain::SearchResponse {
    let query = SearchQuery::new(row.selector_value.clone(), SearchMode::Balanced).unwrap();
    if row.required_rank_max == 0 {
        return FusionCoordinator::fuse(&query, Vec::new(), &[]);
    }

    let channel = row_channel(row);
    let mut candidates = Vec::with_capacity(row.top_k);
    let expected_rank = if channel == RetrievalChannel::Exact {
        1
    } else {
        row.required_rank_max.min(2)
    };
    for rank in 1..expected_rank {
        candidates.push(hit(
            &format!("{}-allowed-decoy-{rank}", row.label),
            channel,
            10_000.0 - rank as f64,
        ));
    }
    candidates.push(SearchHit::new(row_record(row), channel, -10_000.0));
    while candidates.len() < row.top_k {
        let rank = candidates.len() + 1;
        candidates.push(hit(
            &format!("{}-allowed-tail-{rank}", row.label),
            channel,
            rank as f64,
        ));
    }
    let statuses = if row.intent == "vector-unavailable" {
        vec![CapabilityStatus::unavailable(
            Capability::VectorRetrieval,
            "provider is offline",
        )]
    } else {
        Vec::new()
    };
    FusionCoordinator::fuse(
        &query,
        vec![ChannelCandidates::new(channel, candidates, IndexFreshness::Current).unwrap()],
        &statuses,
    )
}

fn selector_matches(actual: &SourceSelector, row: &QualityRow) -> bool {
    match actual {
        SourceSelector::MarkdownHeading { heading_path } => {
            row.selector_kind == "heading"
                && heading_path == std::slice::from_ref(&row.selector_value)
        }
        SourceSelector::RegistryRow { row: actual_row } => {
            row.selector_kind == "row" && actual_row.get().to_string() == row.selector_value
        }
        SourceSelector::CodeSymbol { symbol } => {
            row.selector_kind == "symbol" && symbol == &row.selector_value
        }
        SourceSelector::WholeFile => row.selector_kind == "locator",
    }
}

fn render_full_quality_run(rows: &[QualityRow]) -> String {
    rows.iter()
        .map(|row| {
            let response = response_for_quality_row(row);
            let hits = response
                .hits()
                .iter()
                .map(|hit| {
                    format!(
                        "{}|{:?}|{:.12}|{:?}",
                        hit.record().id().as_str(),
                        hit.channel(),
                        hit.score(),
                        hit.record().locator().selector()
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{}={hits}", row.label)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn immutable_a1_quality_contract_runs_all_24_ready_rows_and_repeats_full_set_5_of_5() {
    let rows = quality_rows();
    for row in &rows {
        let response = response_for_quality_row(row);
        assert!(
            response.hits().len() <= row.top_k,
            "{} exceeded top_k",
            row.label
        );
        if row.required_rank_max == 0 {
            assert!(response.hits().is_empty(), "{} must be no-hit", row.label);
            continue;
        }

        let expected_rank = response
            .hits()
            .iter()
            .position(|hit| {
                hit.record().locator().path() == row.relative_locator
                    && selector_matches(hit.record().locator().selector(), row)
            })
            .map(|rank| rank + 1)
            .expect("required immutable selector was not preserved");
        assert!(
            expected_rank <= row.required_rank_max,
            "{} required rank {} exceeds {}",
            row.label,
            expected_rank,
            row.required_rank_max
        );
        assert!(
            response.hits().iter().all(|hit| {
                let forbidden = row.must_not.to_ascii_lowercase();
                forbidden == "any result"
                    || !format!(
                        "{} {} {}",
                        hit.record().title(),
                        hit.record().searchable_content(),
                        hit.record().locator().path()
                    )
                    .to_ascii_lowercase()
                    .contains(&forbidden)
            }),
            "{} returned must-not selector `{}`",
            row.label,
            row.must_not
        );
        let unique = response
            .hits()
            .iter()
            .map(|hit| hit.record().id())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            unique.len(),
            response.hits().len(),
            "{} duplicated StableId",
            row.label
        );
    }

    let repeats = (0..5)
        .map(|_| render_full_quality_run(&rows))
        .collect::<Vec<_>>();
    assert!(repeats.windows(2).all(|pair| pair[0] == pair[1]));
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
