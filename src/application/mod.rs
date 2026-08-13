//! Application coordination and production compositions for FastSearch.

mod cli;
mod compatibility;
mod console;
pub mod fusion;
mod production;

pub use cli::{CliError, OutputFormat, execute_cli, execute_cli_formatted};
pub use compatibility::RealRuntime;
pub use console::{help_text, run_interactive, version_text};
pub use production::{ProductionConfig, ProductionRuntime};
