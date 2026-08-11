use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use fastsearch::{
    adapters::lexical::TantivyLexical,
    domain::{
        CanonicalRecord, ContentHash, ErrorKind, IndexFreshness, RecordKind, RetrievalChannel,
        SearchMode, SearchQuery, SourceLocator, StableId,
    },
    ports::LexicalRetrieval,
};

struct TemporaryIndex(PathBuf);

impl TemporaryIndex {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fastsearch-d1-{suffix}"));
        fs::create_dir_all(&path).expect("temporary index directory");
        Self(path)
    }
}

impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn record(id: &str, title: &str, content: &str) -> CanonicalRecord {
    CanonicalRecord::new(
        StableId::parse(id).unwrap(),
        RecordKind::MarkdownSection,
        SourceLocator::markdown(format!("fixtures/{id}.md"), [title]).unwrap(),
        title,
        content,
        BTreeMap::new(),
        Vec::new(),
        ContentHash::parse(format!("hash-{id}")).unwrap(),
    )
    .unwrap()
}

fn ids(response: &fastsearch::domain::SearchResponse) -> Vec<&str> {
    response
        .hits()
        .iter()
        .map(|hit| hit.record().id().as_str())
        .collect()
}

#[test]
fn exact_phrase_no_hit_mutation_reopen_degrade_and_rebuild_are_observable() {
    let temporary = TemporaryIndex::new();
    let index = TantivyLexical::open(&temporary.0).unwrap();
    let technical = record(
        "registry-2433",
        "Техническая запись 2433",
        "Технические факты для точного идентификатора.",
    );
    let russian = record(
        "guide-current",
        "Реальный поиск",
        "Точная русская фраза для поиска: документальный поиск.",
    );

    assert_eq!(
        index
            .apply_projection(&[technical.clone(), russian.clone()], 7)
            .unwrap()
            .freshness(),
        IndexFreshness::Current
    );
    let exact = index
        .search(&SearchQuery::new("2433", SearchMode::Balanced).unwrap())
        .unwrap();
    assert_eq!(ids(&exact), vec!["registry-2433"]);
    assert_eq!(exact.hits()[0].channel(), RetrievalChannel::Exact);

    let phrase = index
        .search(&SearchQuery::new("\"документальный поиск\"", SearchMode::Balanced).unwrap())
        .unwrap();
    assert_eq!(ids(&phrase), vec!["guide-current"]);
    assert_eq!(phrase.hits()[0].channel(), RetrievalChannel::Lexical);
    assert!(
        index
            .search(&SearchQuery::new("\"несуществующая фраза\"", SearchMode::Balanced).unwrap())
            .unwrap()
            .hits()
            .is_empty()
    );

    assert_eq!(
        index
            .apply_projection(std::slice::from_ref(&technical), 8)
            .unwrap()
            .projection_generation(),
        Some(8)
    );
    assert!(
        index
            .search(&SearchQuery::new("\"документальный поиск\"", SearchMode::Balanced).unwrap())
            .unwrap()
            .hits()
            .is_empty()
    );

    let reopened = TantivyLexical::open(&temporary.0).unwrap();
    assert_eq!(
        reopened.lifecycle_status().freshness(),
        IndexFreshness::Current
    );
    assert_eq!(
        ids(&reopened
            .search(&SearchQuery::new("2433", SearchMode::Balanced).unwrap())
            .unwrap()),
        vec!["registry-2433"]
    );

    let duplicate = reopened
        .apply_projection(&[technical.clone(), technical.clone()], 9)
        .unwrap_err();
    assert_eq!(duplicate.kind(), &ErrorKind::ProjectionFailure);
    assert_eq!(
        reopened.lifecycle_status().freshness(),
        IndexFreshness::Degraded
    );
    let recovered = reopened
        .search(&SearchQuery::new("2433", SearchMode::Balanced).unwrap())
        .unwrap();
    assert_eq!(recovered.freshness(), IndexFreshness::Degraded);
    assert_eq!(ids(&recovered), vec!["registry-2433"]);

    assert_eq!(
        reopened.rebuild(&[technical], 10).unwrap().freshness(),
        IndexFreshness::Current
    );
    assert_eq!(
        TantivyLexical::open(&temporary.0)
            .unwrap()
            .lifecycle_status()
            .projection_generation(),
        Some(10)
    );
}
