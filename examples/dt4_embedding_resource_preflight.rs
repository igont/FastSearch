use std::{fs, path::PathBuf, time::Instant};

use fastsearch::{
    adapters::vector::benchmark_embedding_cold_warm,
    application::{ensure_embedding_model, model_descriptor},
    domain::EmbeddingModelId,
};
use serde::Serialize;

#[derive(Serialize)]
struct Measurement {
    input_characters: usize,
    batch_size: usize,
    cold_duration_ms: u128,
    warm_duration_ms: u128,
    working_set_bytes: Option<u64>,
}

#[derive(Serialize)]
struct Evidence {
    schema: u32,
    gate: &'static str,
    status: &'static str,
    role: String,
    repository: &'static str,
    revision: &'static str,
    cold_open_ms: u128,
    free_physical_memory_before_bytes: Option<u64>,
    free_physical_memory_after_bytes: Option<u64>,
    measurements: Vec<Measurement>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = std::env::args()
        .nth(1)
        .and_then(|value| EmbeddingModelId::parse(&value))
        .ok_or("embedding model slug required")?;
    let output = PathBuf::from(std::env::args_os().nth(2).ok_or("output path required")?);
    let before = free_physical_memory_bytes();
    let open_started = Instant::now();
    let availability = ensure_embedding_model(model, false)?;
    let cold_open_ms = open_started.elapsed().as_millis();
    let mut measurements = Vec::new();
    for input_characters in [32_usize, 256, 1024] {
        let text = "модельный вход ".repeat(input_characters / 14 + 1);
        let text: String = text.chars().take(input_characters).collect();
        let texts = vec![text.clone(), text];
        for batch_size in [1_usize, 2] {
            let result =
                benchmark_embedding_cold_warm(model, availability.root(), &texts, batch_size)?;
            measurements.push(Measurement {
                input_characters,
                batch_size,
                cold_duration_ms: result.cold_duration_ms(),
                warm_duration_ms: result.warm_duration_ms(),
                working_set_bytes: result.working_set_bytes(),
            });
        }
    }
    let descriptor = model_descriptor(model);
    let evidence = Evidence {
        schema: 1,
        gate: "G-RESOURCE-PREFLIGHT@A2",
        status: "PASS",
        role: model.slug().to_owned(),
        repository: descriptor.repository,
        revision: descriptor.revision,
        cold_open_ms,
        free_physical_memory_before_bytes: before,
        free_physical_memory_after_bytes: free_physical_memory_bytes(),
        measurements,
    };
    fs::write(output, serde_json::to_vec_pretty(&evidence)?)?;
    Ok(())
}

#[cfg(windows)]
fn free_physical_memory_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).ok()?,
        ..MEMORYSTATUSEX::default()
    };
    // SAFETY: status is a correctly sized writable structure for this call.
    (unsafe { GlobalMemoryStatusEx(&raw mut status) } != 0).then_some(status.ullAvailPhys)
}

#[cfg(not(windows))]
fn free_physical_memory_bytes() -> Option<u64> {
    None
}
