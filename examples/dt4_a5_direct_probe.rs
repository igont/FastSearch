use std::{fs, path::PathBuf, sync::atomic::AtomicBool, time::Instant};

use fastsearch::application::{PublicSearchRequest, ThinSearchCoordinator};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("workspace root required")?,
    );
    let query_path = PathBuf::from(std::env::args_os().nth(2).ok_or("query path required")?);
    let output = PathBuf::from(std::env::args_os().nth(3).ok_or("output path required")?);
    let request = PublicSearchRequest::new(fs::read_to_string(query_path)?, None, None)?;
    let mut coordinator = ThinSearchCoordinator::open(&workspace)?;
    let (response, audit) =
        coordinator.search(&request, &AtomicBool::new(false), Instant::now())?;
    fs::write(
        output,
        serde_json::to_vec_pretty(&json!({
            "schema": 1,
            "gate": "TS-DT4-01-intermediate",
            "candidate_contract": {
                "models": 3,
                "per_model": 5,
                "capacity": 15,
                "public_max": 6
            },
            "audit": audit,
            "response": response
        }))?,
    )?;
    Ok(())
}
