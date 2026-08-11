use std::process::ExitCode;

use fastsearch::application::MockFacade;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!("usage: fastsearch mock-search <query>");
        return ExitCode::from(2);
    };

    let Some(query) = args.next() else {
        eprintln!("usage: fastsearch mock-search <query>");
        return ExitCode::from(2);
    };

    if command != "mock-search" || args.next().is_some() {
        eprintln!("usage: fastsearch mock-search <query>");
        return ExitCode::from(2);
    }

    match MockFacade::new().render_mock_search(&query) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("mock-search failed: {error}");
            ExitCode::from(1)
        }
    }
}
