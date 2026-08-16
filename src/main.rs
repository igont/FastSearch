use std::{io, process::ExitCode};

use fastsearch::application::{
    CliError, OutputFormat, execute_cli_formatted, help_text, run_interactive, version_text,
};
use terminal_dialogue::write_line;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() || arguments.as_slice() == ["chat"] {
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
