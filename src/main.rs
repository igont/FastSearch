use std::process::ExitCode;

use fastsearch::application::{CliError, execute_cli};

fn main() -> ExitCode {
    match execute_cli(std::env::args().skip(1).collect()) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(CliError::Usage) => {
            eprintln!("{}", CliError::usage());
            ExitCode::from(2)
        }
        Err(CliError::Runtime(message)) => {
            eprintln!("fastsearch failed: {message}");
            ExitCode::from(1)
        }
    }
}
