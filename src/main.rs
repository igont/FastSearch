use std::{
    io::{self, Write},
    process::ExitCode,
};

use fastsearch::application::{
    CliError, OutputFormat, WorkspaceStore, execute_cli_formatted, help_text, run_interactive,
    version_text,
};
use terminal_dialogue::write_line;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() || arguments.as_slice() == ["chat"] {
        configure_interactive_console();
        return match run_interactive() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                emit_stderr(&format!(
                    "fastsearch failed: interactive input/output: {error}"
                ));
                ExitCode::from(1)
            }
        };
    }
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "help" | "--help" | "-h") {
        return emit_stdout(help_text());
    }
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "version" | "--version" | "-V") {
        return emit_stdout(&version_text());
    }
    if arguments.first().is_some_and(|value| value == "index")
        && arguments.get(1).is_some_and(|value| value == "inspect")
    {
        return run_direct_inspection(&arguments[2..]);
    }
    let (arguments, json) = parse_global_format(arguments);
    let format = if json {
        OutputFormat::Json
    } else {
        OutputFormat::Technical
    };
    match execute_cli_formatted(arguments, format) {
        Ok(output) => emit_stdout(&output),
        Err(CliError::Usage) => {
            if json {
                emit_stderr(&CliError::Usage.render_json());
            } else {
                emit_stderr(&format!(
                    "Ошибка: команда или аргументы не распознаны.\n\n{}\n\nПодробная справка: fastsearch --help",
                    CliError::usage()
                ));
            }
            ExitCode::from(2)
        }
        Err(error) => {
            if json {
                emit_stderr(&error.render_json());
            } else {
                emit_stderr(&format!("fastsearch failed: {}", error.message()));
            }
            ExitCode::from(error.exit_code())
        }
    }
}

fn run_direct_inspection(arguments: &[String]) -> ExitCode {
    let output = match arguments {
        [] => None,
        [output] if !matches!(output.as_str(), "current" | "preview") => {
            Some(std::path::Path::new(output))
        }
        _ => return inspection_usage(),
    };
    let result = std::env::current_dir()
        .map_err(|error| error.to_string())
        .and_then(|directory| {
            WorkspaceStore::open(&directory).map_err(|error| error.message().to_owned())
        })
        .and_then(|workspace| {
            workspace
                .inspect_chunks(output)
                .map_err(|error| error.message().to_owned())
        });
    match result {
        Ok(report) => emit_stdout(&format!(
            "Выгрузка создана: {}. Файлов включено: {}, исключено: {}, чанков: {}.",
            report.display_inputs_path(),
            report.included_files(),
            report.excluded_files(),
            report.chunks()
        )),
        Err(error) => {
            emit_stderr(&format!("fastsearch failed: {error}"));
            ExitCode::from(1)
        }
    }
}

fn inspection_usage() -> ExitCode {
    emit_stderr("Использование: fastsearch index inspect [папка]");
    ExitCode::from(2)
}

/// Makes a directly launched Windows console wide enough for the search-result columns.
/// Pseudoconsoles and redirected output do not expose a console buffer, so they are left alone.
#[cfg(windows)]
fn configure_interactive_console() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        COORD, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetLargestConsoleWindowSize,
        GetStdHandle, SMALL_RECT, STD_OUTPUT_HANDLE, SetConsoleMode, SetConsoleScreenBufferSize,
        SetConsoleWindowInfo,
    };

    const COLUMNS: i16 = 200;
    const ROWS: i16 = 45;

    // SAFETY: the standard-output handle and output structs are provided by Windows; failures
    // deliberately leave the caller's terminal untouched.
    unsafe {
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        if output.is_null() || output == INVALID_HANDLE_VALUE {
            return;
        }
        let maximum = GetLargestConsoleWindowSize(output);
        if maximum.X <= 0 || maximum.Y <= 0 {
            return;
        }
        let columns = COLUMNS.min(maximum.X);
        let rows = ROWS.min(maximum.Y);
        if columns <= 0 || rows <= 0 {
            return;
        }
        let _ = SetConsoleScreenBufferSize(
            output,
            COORD {
                X: columns,
                Y: rows,
            },
        );
        let _ = SetConsoleWindowInfo(
            output,
            1,
            &SMALL_RECT {
                Left: 0,
                Top: 0,
                Right: columns - 1,
                Bottom: rows - 1,
            },
        );
        let mut mode = 0;
        if GetConsoleMode(output, &mut mode) != 0
            && SetConsoleMode(output, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
        {
            // CSI 8 is handled by Windows Terminal itself and resizes its visible window.
            let escape = char::from_u32(27).expect("ASCII escape character");
            let _ = io::stdout()
                .lock()
                .write_all(format!("{escape}[8;{rows};{columns}t").as_bytes());
        }
    }
}

#[cfg(not(windows))]
fn configure_interactive_console() {}

fn emit_stdout(text: &str) -> ExitCode {
    let stdout = io::stdout();
    if write_line(&mut stdout.lock(), text).is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn emit_stderr(text: &str) {
    let stderr = io::stderr();
    let _ = write_line(&mut stderr.lock(), text);
}

fn parse_global_format(arguments: Vec<String>) -> (Vec<String>, bool) {
    let mut positional = Vec::with_capacity(arguments.len());
    let mut json = false;
    let mut parse_options = true;
    for argument in arguments {
        if parse_options && argument == "--" {
            parse_options = false;
        } else if parse_options && argument == "--json" {
            json = true;
        } else {
            positional.push(argument);
        }
    }
    (positional, json)
}
