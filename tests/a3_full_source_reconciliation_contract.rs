use fastsearch::adapters::mock::MockState;
use fastsearch::domain::{ErrorKind, FileHash, SourceLocator, SourceSnapshot};
use fastsearch::ports::StateStore;

fn empty_snapshot(path: &str) -> SourceSnapshot {
    SourceSnapshot::new(
        SourceLocator::whole_file(path).expect("valid source locator"),
        FileHash::parse("file-v1").expect("valid file hash"),
        Vec::new(),
    )
}

fn reconcile_via_object(
    store: &mut dyn StateStore,
    snapshots: &[SourceSnapshot],
) -> Result<(), fastsearch::domain::FastSearchError> {
    store.reconcile_snapshots(snapshots).map(|_| ())
}

#[test]
fn complete_scan_is_object_safe_and_mock_rejects_it_as_state_failure() {
    let mut state = MockState::default();

    let empty = reconcile_via_object(&mut state, &[])
        .expect_err("mock must not claim an empty durable reconciliation");
    assert_eq!(empty.kind(), &ErrorKind::StateFailure);

    let full = reconcile_via_object(&mut state, &[empty_snapshot("docs/guide.md")])
        .expect_err("mock must not claim durable source reconciliation");

    assert_eq!(full.kind(), &ErrorKind::StateFailure);
}
