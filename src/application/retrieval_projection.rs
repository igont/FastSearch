use std::collections::BTreeMap;

use crate::{
    domain::{ErrorKind, FastSearchError, RetrievalChannel, SearchHit, SearchResponse},
    ports::StateStore,
};

use super::chunking::parent_id;

pub(crate) fn canonicalize_projection_hits(
    state: &dyn StateStore,
    response: SearchResponse,
) -> Result<SearchResponse, FastSearchError> {
    let freshness = response.freshness();
    let mut best = BTreeMap::<(u8, String), SearchHit>::new();
    for hit in response.hits() {
        let Some(parent) = parent_id(hit.record())? else {
            best.insert(
                (
                    channel_order(hit.channel()),
                    hit.record().id().as_str().to_owned(),
                ),
                hit.clone(),
            );
            continue;
        };
        let canonical = state.get(&parent)?.ok_or_else(|| {
            FastSearchError::new(
                ErrorKind::ProjectionFailure,
                format!("projection chunk refers to missing canonical record {parent:?}"),
            )
        })?;
        let mut canonical_hit = SearchHit::new(canonical, hit.channel(), hit.score());
        if let Some(provenance) = hit.projection_provenance() {
            canonical_hit = canonical_hit.with_projection_provenance(provenance.clone());
        }
        let key = (
            channel_order(canonical_hit.channel()),
            canonical_hit.record().id().as_str().to_owned(),
        );
        if best
            .get(&key)
            .is_none_or(|current| canonical_hit.score() > current.score())
        {
            best.insert(key, canonical_hit);
        }
    }
    let mut hits = best.into_values().collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score()
            .total_cmp(&left.score())
            .then_with(|| left.record().id().cmp(right.record().id()))
    });
    Ok(SearchResponse::with_freshness(hits, freshness))
}

const fn channel_order(channel: RetrievalChannel) -> u8 {
    match channel {
        RetrievalChannel::Exact => 0,
        RetrievalChannel::Lexical => 1,
        RetrievalChannel::Vector => 2,
        RetrievalChannel::CodeMap => 3,
        RetrievalChannel::Symbol => 4,
    }
}
