//! Deterministic application-level calibration and fusion of retrieval channels.
//!
//! Adapters retain authority over candidate production. This module consumes only
//! their public hits and lifecycle status. Raw adapter scores are deliberately not
//! used because lexical and vector score spaces are not comparable.

use std::collections::BTreeMap;

use crate::domain::{
    CanonicalRecord, CapabilityState, CapabilityStatus, ErrorKind, FastSearchError, IndexFreshness,
    ProjectionProvenance, RetrievalChannel, SearchHit, SearchMode, SearchQuery, SearchResponse,
    StableId,
};

const RECIPROCAL_RANK_OFFSET: f64 = 60.0;
const EXACT_BUCKET: f64 = 1.0;

/// An already-ranked candidate sequence from exactly one accepted retrieval channel.
///
/// The order is authoritative for calibration. `SearchHit::score` is intentionally
/// ignored. The caller must not mix channels inside one sequence.
#[derive(Clone, Debug)]
pub struct ChannelCandidates {
    channel: RetrievalChannel,
    hits: Vec<SearchHit>,
    freshness: IndexFreshness,
}

impl ChannelCandidates {
    pub fn new(
        channel: RetrievalChannel,
        hits: Vec<SearchHit>,
        freshness: IndexFreshness,
    ) -> Result<Self, FastSearchError> {
        if hits.iter().any(|hit| hit.channel() != channel) {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "channel candidate sequence must not mix retrieval channels",
            ));
        }
        Ok(Self {
            channel,
            hits,
            freshness,
        })
    }
}

/// Stateless, deterministic fusion policy used by the application composition.
pub struct FusionCoordinator;

impl FusionCoordinator {
    /// Fuses ranked channel candidates using weighted reciprocal rank.
    ///
    /// Exact hits always occupy a separate dominant bucket. Non-exact channel
    /// contributions are monotonic in rank and mode weight. Duplicate stable IDs
    /// accumulate evidence but produce one hit. Final ties use ascending StableId.
    #[must_use]
    pub fn fuse(
        query: &SearchQuery,
        candidate_sets: Vec<ChannelCandidates>,
        capability_statuses: &[CapabilityStatus],
    ) -> SearchResponse {
        let mut fused = BTreeMap::<StableId, Accumulator>::new();
        let mut freshness = if candidate_sets.is_empty() {
            IndexFreshness::Current
        } else {
            IndexFreshness::NotConfigured
        };

        for candidates in candidate_sets {
            freshness = merge_freshness(freshness, candidates.freshness);
            let weight = channel_weight(query.mode(), candidates.channel);
            for (zero_based_rank, hit) in candidates.hits.into_iter().enumerate() {
                let rank = zero_based_rank as f64 + 1.0;
                let contribution = weight / (RECIPROCAL_RANK_OFFSET + rank);
                let id = hit.record().id().clone();
                let candidate_provenance = hit.projection_provenance().cloned();
                let entry = fused.entry(id).or_insert_with(|| Accumulator {
                    record: hit.record().clone(),
                    representative_channel: hit.channel(),
                    representative_weight: weight,
                    calibrated_score: 0.0,
                    exact: false,
                    provenance: candidate_provenance.clone(),
                });

                entry.calibrated_score += contribution;
                entry.exact |= hit.channel() == RetrievalChannel::Exact;
                if candidate_provenance.is_some() && entry.provenance.is_none() {
                    entry.provenance = candidate_provenance;
                }
                if representative_precedes(
                    hit.channel(),
                    weight,
                    entry.representative_channel,
                    entry.representative_weight,
                ) {
                    entry.record = hit.record().clone();
                    entry.representative_channel = hit.channel();
                    entry.representative_weight = weight;
                }
            }
        }

        for status in capability_statuses {
            freshness = merge_freshness(freshness, status_freshness(status));
        }

        let mut ranked = fused.into_values().collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .exact
                .cmp(&left.exact)
                .then_with(|| right.calibrated_score.total_cmp(&left.calibrated_score))
                .then_with(|| left.record.id().cmp(right.record.id()))
        });
        let hits = ranked
            .into_iter()
            .map(|candidate| {
                let score =
                    candidate.calibrated_score + if candidate.exact { EXACT_BUCKET } else { 0.0 };
                let hit = SearchHit::new(candidate.record, candidate.representative_channel, score);
                match candidate.provenance {
                    Some(provenance) => hit.with_projection_provenance(provenance),
                    None => hit,
                }
            })
            .collect();

        SearchResponse::with_freshness(hits, freshness)
    }
}

struct Accumulator {
    record: CanonicalRecord,
    representative_channel: RetrievalChannel,
    representative_weight: f64,
    calibrated_score: f64,
    exact: bool,
    provenance: Option<ProjectionProvenance>,
}

fn representative_precedes(
    candidate_channel: RetrievalChannel,
    candidate_weight: f64,
    current_channel: RetrievalChannel,
    current_weight: f64,
) -> bool {
    candidate_channel == RetrievalChannel::Exact && current_channel != RetrievalChannel::Exact
        || candidate_weight > current_weight
        || (candidate_weight == current_weight
            && channel_order(candidate_channel) < channel_order(current_channel))
}

fn channel_weight(mode: SearchMode, channel: RetrievalChannel) -> f64 {
    match (mode, channel) {
        (_, RetrievalChannel::Exact) => 1.0,
        (SearchMode::Balanced, RetrievalChannel::Lexical) => 1.0,
        (SearchMode::Balanced, RetrievalChannel::Vector) => 0.95,
        (SearchMode::Balanced, RetrievalChannel::CodeMap | RetrievalChannel::Symbol) => 0.8,
        (SearchMode::Current, RetrievalChannel::Lexical) => 1.0,
        (SearchMode::Current, RetrievalChannel::Symbol) => 0.9,
        (SearchMode::Current, RetrievalChannel::Vector) => 0.7,
        (SearchMode::Current, RetrievalChannel::CodeMap) => 0.6,
        (SearchMode::Design, RetrievalChannel::CodeMap) => 1.0,
        (SearchMode::Design, RetrievalChannel::Symbol) => 0.95,
        (SearchMode::Design, RetrievalChannel::Vector) => 0.85,
        (SearchMode::Design, RetrievalChannel::Lexical) => 0.7,
    }
}

fn channel_order(channel: RetrievalChannel) -> u8 {
    match channel {
        RetrievalChannel::Exact => 0,
        RetrievalChannel::Lexical => 1,
        RetrievalChannel::Vector => 2,
        RetrievalChannel::CodeMap => 3,
        RetrievalChannel::Symbol => 4,
    }
}

fn status_freshness(status: &CapabilityStatus) -> IndexFreshness {
    match status.state() {
        CapabilityState::Available { .. } => IndexFreshness::Current,
        CapabilityState::Stale { .. } | CapabilityState::Unavailable { .. } => {
            IndexFreshness::Stale
        }
        CapabilityState::Degraded { .. } => IndexFreshness::Degraded,
    }
}

fn merge_freshness(left: IndexFreshness, right: IndexFreshness) -> IndexFreshness {
    if left == IndexFreshness::Degraded || right == IndexFreshness::Degraded {
        IndexFreshness::Degraded
    } else if left == IndexFreshness::Stale || right == IndexFreshness::Stale {
        IndexFreshness::Stale
    } else if left == IndexFreshness::Current || right == IndexFreshness::Current {
        IndexFreshness::Current
    } else {
        IndexFreshness::NotConfigured
    }
}
