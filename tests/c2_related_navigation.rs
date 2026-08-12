use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use fastsearch::adapters::{
    maps::{CodeMapRelated, CodeMapSource},
    source::FilesystemSource,
    symbols::SymbolSource,
};
use fastsearch::domain::{LogicalRootId, RelatedQuery, StableId};
use fastsearch::ports::{CodeMapPort, SourcePort};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "fastsearch-c2-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn map(targets: &[StableId]) -> String {
    format!(
        "---\ncfmap: v1\nmode: CURATED\n---\n# Navigation map\n{}",
        targets
            .iter()
            .map(|id| format!("@related {}\n", id.as_str()))
            .collect::<String>()
    )
}

#[test]
fn explicit_map_relations_resolve_exact_d2_symbol_ids_in_stable_order_with_traceable_provenance() {
    let f = Fixture::new();
    f.write("docs/navigation.md", "# Navigation\nDocument target.\n");
    f.write(
        "code/navigator.rs",
        "pub fn stable_navigation() {}\npub fn other() {}\n",
    );
    let documents = FilesystemSource::new(f.0.join("docs")).records().unwrap();
    let document = documents[0].id().clone();
    let symbols = SymbolSource::new(
        LogicalRootId::parse("code-fastsearch").unwrap(),
        f.0.join("code"),
    )
    .records()
    .unwrap();
    let wanted = symbols
        .iter()
        .find(|record| record.title() == "stable_navigation")
        .unwrap()
        .id()
        .clone();
    let other = symbols
        .iter()
        .find(|record| record.title() == "other")
        .unwrap()
        .id()
        .clone();
    f.write(
        "maps/navigation.cfmap.md",
        &map(&[
            wanted.clone(),
            document.clone(),
            other.clone(),
            wanted.clone(),
        ]),
    );
    let maps = CodeMapSource::new(f.0.join("maps")).records().unwrap();
    let navigation =
        CodeMapRelated::new([maps.clone(), documents, symbols.clone()].concat()).unwrap();
    let query = RelatedQuery::new(maps[0].id().clone());

    let mut expected = vec![
        document.as_str().to_owned(),
        other.as_str().to_owned(),
        wanted.as_str().to_owned(),
    ];
    expected.sort();
    for _ in 0..5 {
        let related = navigation.related_maps(&query).unwrap();
        assert_eq!(
            related
                .iter()
                .map(|record| record.id().as_str().to_owned())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            related
                .iter()
                .all(|record| record.metadata().contains_key("relation_provenance"))
        );
    }
}

#[test]
fn dangling_cycle_delete_and_reopen_are_structured_without_fabricated_records() {
    let f = Fixture::new();
    let dangling =
        StableId::parse("named-root-v1:code-fastsearch:missing.rs:symbol:rust:function:missing:0")
            .unwrap();
    f.write(
        "maps/a.cfmap.md",
        &map(&[StableId::parse("cfmap-v1:b.cfmap.md").unwrap()]),
    );
    f.write(
        "maps/b.cfmap.md",
        &map(&[StableId::parse("cfmap-v1:a.cfmap.md").unwrap()]),
    );
    f.write("maps/dangling.cfmap.md", &map(&[dangling]));
    let source = CodeMapSource::new(f.0.join("maps"));
    let maps = source.records().unwrap();
    let related = CodeMapRelated::new(maps.clone()).unwrap();
    let a = maps
        .iter()
        .find(|record| record.id().as_str() == "cfmap-v1:a.cfmap.md")
        .unwrap();
    assert_eq!(
        related
            .related_maps(&RelatedQuery::new(a.id().clone()))
            .unwrap()[0]
            .id()
            .as_str(),
        "cfmap-v1:b.cfmap.md"
    );
    let dangling_map = maps
        .iter()
        .find(|record| record.id().as_str() == "cfmap-v1:dangling.cfmap.md")
        .unwrap();
    assert_eq!(
        related
            .related_maps(&RelatedQuery::new(dangling_map.id().clone()))
            .unwrap_err()
            .kind(),
        &fastsearch::domain::ErrorKind::NotFound
    );

    fs::remove_file(f.0.join("maps/b.cfmap.md")).unwrap();
    let maps_after_delete = source.records().unwrap();
    let after_delete = CodeMapRelated::new(maps_after_delete.clone()).unwrap();
    let a_after_delete = maps_after_delete
        .iter()
        .find(|record| record.id().as_str() == "cfmap-v1:a.cfmap.md")
        .unwrap()
        .id()
        .clone();
    assert_eq!(
        after_delete
            .related_maps(&RelatedQuery::new(a_after_delete))
            .unwrap_err()
            .kind(),
        &fastsearch::domain::ErrorKind::NotFound
    );
    f.write("maps/b.cfmap.md", &map(&[]));
    let reopened = source.records().unwrap();
    let reopened_navigation = CodeMapRelated::new(reopened.clone()).unwrap();
    let a_reopened = reopened
        .iter()
        .find(|record| record.id().as_str() == "cfmap-v1:a.cfmap.md")
        .unwrap();
    assert_eq!(
        reopened_navigation
            .related_maps(&RelatedQuery::new(a_reopened.id().clone()))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn stale_map_refuses_related_navigation() {
    let f = Fixture::new();
    f.write(
        "maps/stale.cfmap.md",
        "---\ncfmap: v1\nmode: AUTO\nsource: missing.md#Gone\n---\n# Stale\n",
    );
    let maps = CodeMapSource::new(f.0.join("maps")).records().unwrap();
    let related = CodeMapRelated::new(maps.clone()).unwrap();
    assert_eq!(
        related
            .related_maps(&RelatedQuery::new(maps[0].id().clone()))
            .unwrap_err()
            .kind(),
        &fastsearch::domain::ErrorKind::StateFailure
    );
}

#[test]
fn map_to_symbol_replay_keeps_identity_provenance_and_order_after_rename_reopen_and_delete() {
    let f = Fixture::new();
    f.write(
        "code/navigator.rs",
        "pub fn stable_navigation() {}\npub fn later() {}\n",
    );
    let symbols = SymbolSource::new(
        LogicalRootId::parse("code-fastsearch").unwrap(),
        f.0.join("code"),
    )
    .records()
    .unwrap();
    let stable = symbols
        .iter()
        .find(|record| record.title() == "stable_navigation")
        .unwrap()
        .id()
        .clone();
    let later = symbols
        .iter()
        .find(|record| record.title() == "later")
        .unwrap()
        .id()
        .clone();
    f.write(
        "maps/navigation.cfmap.md",
        &map(&[later.clone(), stable.clone()]),
    );
    let source = CodeMapSource::new(f.0.join("maps"));
    let before_maps = source.records().unwrap();
    let before_id = before_maps[0].id().clone();
    let expected = related_ids(&before_maps, &symbols, &before_id);

    fs::rename(
        f.0.join("maps/navigation.cfmap.md"),
        f.0.join("maps/renamed.cfmap.md"),
    )
    .unwrap();
    let renamed_maps = source.records().unwrap();
    let renamed_id = renamed_maps[0].id().clone();
    assert_ne!(renamed_id, before_id);
    assert_eq!(related_ids(&renamed_maps, &symbols, &renamed_id), expected);

    fs::remove_file(f.0.join("maps/renamed.cfmap.md")).unwrap();
    assert!(source.records().unwrap().is_empty());
    f.write("maps/renamed.cfmap.md", &map(&[later, stable]));
    let reopened_maps = source.records().unwrap();
    assert_eq!(reopened_maps[0].id(), &renamed_id);
    assert_eq!(related_ids(&reopened_maps, &symbols, &renamed_id), expected);
}

fn related_ids(
    maps: &[fastsearch::domain::CanonicalRecord],
    symbols: &[fastsearch::domain::CanonicalRecord],
    map_id: &StableId,
) -> Vec<String> {
    CodeMapRelated::new([maps, symbols].concat())
        .unwrap()
        .related_maps(&RelatedQuery::new(map_id.clone()))
        .unwrap()
        .into_iter()
        .map(|record| {
            assert!(record.metadata().contains_key("relation_provenance"));
            record.id().as_str().to_owned()
        })
        .collect()
}
