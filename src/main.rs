use std::process::ExitCode;

use fastsearch::application::{
    CliError, OutputFormat, execute_cli_formatted, help_text, run_interactive, version_text,
};

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() || arguments.as_slice() == ["chat"] {
        return match run_interactive() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("fastsearch failed: interactive input/output: {error}");
                ExitCode::from(1)
            }
        };
    }
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "help" | "--help" | "-h") {
        println!("{}", help_text());
        return ExitCode::SUCCESS;
    }
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "version" | "--version" | "-V") {
        println!("{}", version_text());
        return ExitCode::SUCCESS;
    }
    let (arguments, json) = parse_global_format(arguments);
    let format = if json {
        OutputFormat::Json
    } else {
        OutputFormat::Technical
    };
    match execute_cli_formatted(arguments, format) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(CliError::Usage) => {
            if json {
                eprintln!("{}", CliError::Usage.render_json());
            } else {
                eprintln!(
                    "Ошибка: команда или аргументы не распознаны.\n\n{}",
                    CliError::usage()
                );
                eprintln!("\nПодробная справка: fastsearch --help");
            }
            ExitCode::from(2)
        }
        Err(error) => {
            if json {
                eprintln!("{}", error.render_json());
            } else {
                eprintln!("fastsearch failed: {}", error.message());
            }
            ExitCode::from(error.exit_code())
        }
    }
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
