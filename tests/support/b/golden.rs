use fastsearch::domain::{Capability, ErrorKind, RelatedQuery, SearchQuery};

use super::PortContractFixture;

/// Сверяет synthetic flow с текстовыми golden без выбора adapter-технологии.
#[allow(clippy::too_many_arguments)]
pub fn assert_golden_flow(
    fixture: &impl PortContractFixture,
    happy_query: &str,
    no_hit_query: &str,
    happy_golden: &str,
    no_hit_golden: &str,
    unavailable_golden: &str,
    status_golden: &str,
) {
    let happy_query = SearchQuery::new(happy_query.trim(), Default::default()).unwrap();
    let happy_response = fixture.agent().search(&happy_query).unwrap();
    assert_eq!(
        render_response(&happy_query, &happy_response),
        normalized_golden(happy_golden)
    );

    let no_hit_query = SearchQuery::new(no_hit_query.trim(), Default::default()).unwrap();
    let no_hit_response = fixture.agent().search(&no_hit_query).unwrap();
    assert_eq!(
        render_response(&no_hit_query, &no_hit_response),
        normalized_golden(no_hit_golden)
    );

    let id = fixture.expected_record().id().clone();
    let unavailable = fixture.agent().related(&RelatedQuery::new(id)).unwrap_err();
    assert_eq!(
        unavailable.kind(),
        &ErrorKind::CapabilityUnavailable {
            capability: Capability::CodeMaps,
        }
    );
    assert_eq!(
        "capability=CodeMaps\nerror=CapabilityUnavailable",
        normalized_golden(unavailable_golden)
    );

    assert_eq!(render_status(fixture), normalized_golden(status_golden));
}

fn normalized_golden(golden: &str) -> String {
    golden.trim().replace("\r\n", "\n")
}

fn render_response(query: &SearchQuery, response: &fastsearch::domain::SearchResponse) -> String {
    let mut lines = vec![
        format!("query={}", query.text()),
        format!("hits={}", response.hits().len()),
    ];

    if let Some(hit) = response.hits().first() {
        lines.push(format!("channel={:?}", hit.channel()));
        lines.push(format!("score={}", hit.score()));
        lines.push(format!("record={}", hit.record().id().as_str()));
    }

    lines.join("\n")
}

fn render_status(fixture: &impl PortContractFixture) -> String {
    fixture
        .agent()
        .status()
        .into_iter()
        .map(|status| match status.state() {
            fastsearch::domain::CapabilityState::Available { backend } => {
                format!("{:?}={backend:?}", status.capability())
            }
            fastsearch::domain::CapabilityState::Unavailable { .. } => {
                format!("{:?}=Unavailable", status.capability())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
