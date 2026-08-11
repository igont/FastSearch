use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use fastsearch::{
    adapters::lexical::TantivyLexical,
    domain::{
        CanonicalRecord, ContentHash, RecordKind, RetrievalChannel, SearchMode, SearchQuery,
        SourceLocator, StableId,
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
        let path = std::env::temp_dir().join(format!("fastsearch-d2-{suffix}"));
        fs::create_dir_all(&path).expect("temporary index directory");
        Self(path)
    }
}

impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn record(id: &str, metadata: BTreeMap<String, String>) -> CanonicalRecord {
    CanonicalRecord::new(
        StableId::parse(id).unwrap(),
        RecordKind::MarkdownSection,
        SourceLocator::markdown(format!("fixtures/{id}.md"), ["Реальный поиск"]).unwrap(),
        "Реальный поиск",
        "Одинаковая русская фраза для ranking: документальный поиск.",
        metadata,
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
fn phrase_ranking_is_mode_specific_stable_and_exact_remains_dominant() {
    let temporary = TemporaryIndex::new();
    let index = TantivyLexical::open(&temporary.0).unwrap();
    let design = record(
        "a-design",
        BTreeMap::from([("alignment".into(), "DESIGN".into())]),
    );
    let neutral = record("b-neutral", BTreeMap::new());
    let unknown = record(
        "c-unknown",
        BTreeMap::from([
            ("alignment".into(), "ARCHIVE".into()),
            ("lifecycle".into(), "future".into()),
            ("normativity".into(), "mandatory".into()),
        ]),
    );
    let current_alignment = record(
        "y-current-alignment",
        BTreeMap::from([("alignment".into(), "CURRENT".into())]),
    );
    let current_lifecycle = record(
        "z-current-lifecycle",
        BTreeMap::from([("lifecycle".into(), "current".into())]),
    );
    let technical = CanonicalRecord::new(
        StableId::parse("registry-2433").unwrap(),
        RecordKind::RegistryRow,
        SourceLocator::registry_row("fixtures/registry.tsv", 1.try_into().unwrap()).unwrap(),
        "Техническая запись 2433",
        "Точный технический идентификатор.",
        BTreeMap::new(),
        Vec::new(),
        ContentHash::parse("hash-registry-2433").unwrap(),
    )
    .unwrap();

    index
        .apply_projection(
            &[
                design.clone(),
                neutral.clone(),
                unknown.clone(),
                current_alignment.clone(),
                current_lifecycle.clone(),
                technical,
            ],
            1,
        )
        .unwrap();

    let phrase = "\"документальный поиск\"";
    assert_eq!(
        ids(&index
            .search(&SearchQuery::new(phrase, SearchMode::Balanced).unwrap())
            .unwrap()),
        vec![
            "a-design",
            "b-neutral",
            "c-unknown",
            "y-current-alignment",
            "z-current-lifecycle"
        ]
    );
    assert_eq!(
        ids(&index
            .search(&SearchQuery::new(phrase, SearchMode::Current).unwrap())
            .unwrap()),
        vec![
            "y-current-alignment",
            "z-current-lifecycle",
            "a-design",
            "b-neutral",
            "c-unknown"
        ]
    );
    assert_eq!(
        ids(&index
            .search(&SearchQuery::new(phrase, SearchMode::Design).unwrap())
            .unwrap()),
        vec![
            "a-design",
            "b-neutral",
            "c-unknown",
            "y-current-alignment",
            "z-current-lifecycle"
        ]
    );

    for mode in [
        SearchMode::Balanced,
        SearchMode::Current,
        SearchMode::Design,
    ] {
        let exact = index
            .search(&SearchQuery::new("2433", mode).unwrap())
            .unwrap();
        assert_eq!(ids(&exact), vec!["registry-2433"]);
        assert_eq!(exact.hits()[0].channel(), RetrievalChannel::Exact);
    }
}
