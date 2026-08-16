const MAIN_SOURCE: &str = include_str!("../src/main.rs");
const CONSOLE_SOURCE: &str = include_str!("../src/application/console.rs");
const COMPARISON_SOURCE: &str = include_str!("../src/application/console/comparison.rs");
const MODEL_SOURCE: &str = include_str!("../src/application/console/model.rs");

#[test]
fn fastsearch_terminal_boundary_never_writes_or_styles_manually() {
    terminal_dialogue::assert_terminal_dialogue_boundary! {
        "src/main.rs" => MAIN_SOURCE,
        "src/application/console.rs" => CONSOLE_SOURCE,
        "src/application/console/comparison.rs" => COMPARISON_SOURCE,
        "src/application/console/model.rs" => MODEL_SOURCE,
    }

    assert!(MAIN_SOURCE.contains("terminal_dialogue::write_line"));
    assert!(CONSOLE_SOURCE.contains("terminal_dialogue::{"));
    assert!(CONSOLE_SOURCE.contains("ChatSession"));
    for source in [CONSOLE_SOURCE, COMPARISON_SOURCE, MODEL_SOURCE] {
        for forbidden in [
            "DialogueDocument::from_text",
            "chat.message(",
            ".heading(",
            ".body_line(",
            "ChatSession::new(",
            ".read_input(",
            ".prompt_with_cancellation(",
        ] {
            assert!(
                !source.contains(forbidden),
                "console uses compatibility UI API {forbidden}"
            );
        }
    }
}

#[test]
fn fastsearch_declares_complete_terminal_capability_matrix() {
    use terminal_dialogue::{ApplicationContract, CapabilityProfile};

    let contract = ApplicationContract::new("FastSearch")
        .with_profile(CapabilityProfile::Guided)
        .with_profile(CapabilityProfile::Mutating)
        .with_profile(CapabilityProfile::ResultNavigation)
        .with_profile(CapabilityProfile::Progress);
    let provided = [
        "welcome",
        "help",
        "unknown-command",
        "cancel",
        "end-of-input",
        "no-color",
        "redirected-input",
        "guided-invalid-retry",
        "guided-cancel-every-step",
        "guided-end-of-input-every-step",
        "preview-confirm-apply",
        "preview-decline-no-apply",
        "preview-cancel-no-apply",
        "result-page",
        "result-empty",
        "result-open",
        "result-page-boundary",
        "progress-terminal",
        "progress-redirected",
        "progress-failure",
    ];

    assert_eq!(contract.missing_scenarios(provided), Vec::<&str>::new());
}
