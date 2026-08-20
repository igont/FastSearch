use super::{
    contour_summary, device_assignment_cell, display_path, display_relative_path, full_trigger,
    help_text, relative_match_percent, ui_guidance, workspace_catalog, workspace_help_catalog,
};
use crate::domain::{DeviceCapabilityStatus, ExecutionDevice};
use std::path::Path;
use terminal_dialogue::{CommandResolution, LanguagePack, TerminalDocument, TextStyle};

#[test]
fn model_device_cells_mark_only_the_assignment_and_reject_unavailable_devices() {
    assert_eq!(
        device_assignment_cell(
            ExecutionDevice::Cpu,
            ExecutionDevice::Cpu,
            DeviceCapabilityStatus::Ready,
        ),
        ("✓", TextStyle::Success)
    );
    assert_eq!(
        device_assignment_cell(
            ExecutionDevice::Cpu,
            ExecutionDevice::GpuDirectMl,
            DeviceCapabilityStatus::Ready,
        ),
        ("", TextStyle::Body)
    );
    assert_eq!(
        device_assignment_cell(
            ExecutionDevice::Cpu,
            ExecutionDevice::GpuDirectMl,
            DeviceCapabilityStatus::Unavailable,
        ),
        ("✗", TextStyle::Error)
    );
}

#[test]
fn root_help_omits_navigation_and_model_device_uses_the_longest_command_match() {
    let catalog = workspace_catalog();
    assert!(matches!(
        catalog.resolve("/model device 2 gpu"),
        CommandResolution::Match { arguments, .. } if arguments == "2 gpu"
    ));
    assert!(matches!(
        catalog.resolve("/index clear 2"),
        CommandResolution::Match { arguments, .. } if arguments == "2"
    ));
    let help = workspace_help_catalog()
        .welcome_document("Команды", "Сводка")
        .to_dialogue_document(&LanguagePack::russian())
        .render(false);
    for heading in [
        "ПОИСК",
        "ИСТОЧНИКИ И ИНДЕКС",
        "МОДЕЛИ И СРАВНЕНИЕ",
        "ПРИЛОЖЕНИЕ",
    ] {
        assert!(help.contains(heading), "{help}");
    }
    assert!(!help.contains("НАВИГАЦИЯ"), "{help}");
    assert!(!help.contains("/open <номер>"), "{help}");

    let commands = [
        ui_guidance::model_catalog(),
        ui_guidance::model_detail(),
        ui_guidance::result_detail(),
        ui_guidance::search_results(),
    ]
    .into_iter()
    .flat_map(|next_step| next_step.actions)
    .map(|action| action.command)
    .chain(help_text().lines().map(str::to_owned))
    .collect::<Vec<_>>();
    assert!(
        commands.iter().all(|command| !command.contains(" N")),
        "ambiguous numeric placeholder: {commands:?}"
    );
}

#[test]
fn result_percent_is_relative_and_bounded() {
    assert_eq!(relative_match_percent(0.0156, 0.0156), 100);
    assert_eq!(relative_match_percent(0.0153, 0.0156), 98);
    assert_eq!(relative_match_percent(-1.0, 0.0156), 0);
    assert_eq!(relative_match_percent(f64::NAN, 0.0156), 0);
}

#[test]
fn full_trigger_keeps_every_word_in_a_single_terminal_row() {
    let trigger = full_trigger("Первый абзац.\n\nВторой абзац.");
    assert_eq!(trigger, "Первый абзац. Второй абзац.");
}

#[test]
fn contour_summary_counts_types_not_roots() {
    assert_eq!(contour_summary(0, 0), "не настроены");
    assert_eq!(contour_summary(3, 0), "документация · 3 корней");
    assert_eq!(contour_summary(3, 2), "документация · код · 3 + 2 корней");
}

#[test]
fn source_paths_use_platform_separators_consistently() {
    let separator = std::path::MAIN_SEPARATOR;
    assert_eq!(
        display_relative_path("Governance/01-Decisions\\Scripts"),
        format!("Governance{separator}01-Decisions{separator}Scripts")
    );
}

#[cfg(windows)]
#[test]
fn extended_windows_prefix_is_never_shown_to_the_user() {
    assert_eq!(
        display_path(Path::new(r"\\?\C:\Obsidian\Docs")),
        r"C:\Obsidian\Docs"
    );
}
