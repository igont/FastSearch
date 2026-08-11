use std::process::Command;

#[test]
fn executable_runs_mock_search_through_the_application_flow() {
    let output = Command::new(env!("CARGO_BIN_EXE_fastsearch"))
        .args(["mock-search", "stable-id:guide"])
        .output()
        .expect("mock executable starts");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        concat!(
            "query=stable-id:guide\n",
            "hits=1\n",
            "channel=Exact\n",
            "score=1\n",
            "record=stable-id:guide\n",
            "Source=Mock\n",
            "State=Mock\n",
            "LexicalRetrieval=Mock\n",
            "VectorRetrieval=Unavailable\n",
            "CodeMaps=Unavailable\n"
        )
    );
}
