const FORBIDDEN: [&str; 8] = [
    "MockSource",
    "MockState",
    "MockLexical",
    "MockRuntime",
    "MockFacade",
    "MockSymbols",
    "adapters::mock",
    "mock-search",
];

#[test]
fn production_composition_has_no_document_mock_registration_or_route() {
    let production_modules = [
        include_str!("../src/lib.rs"),
        include_str!("../src/main.rs"),
        include_str!("../src/application/mod.rs"),
        include_str!("../src/application/cli.rs"),
        include_str!("../src/adapters/mod.rs"),
        include_str!("../src/adapters/source/mod.rs"),
        include_str!("../src/adapters/state/mod.rs"),
        include_str!("../src/adapters/state/sqlite.rs"),
        include_str!("../src/adapters/lexical/mod.rs"),
    ];

    for module in production_modules {
        for forbidden in FORBIDDEN {
            assert!(
                !module.contains(forbidden),
                "production source still contains {forbidden}"
            );
        }
    }
}
