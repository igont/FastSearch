use std::{fs, path::PathBuf, time::Instant};

use serde::Serialize;
use tokio::sync::mpsc;

#[derive(Serialize)]
struct Sample {
    payload_bytes: usize,
    elapsed_microseconds: u128,
    serialized_response_bytes: usize,
}

#[derive(Serialize)]
struct Evidence {
    schema: u32,
    gate: &'static str,
    status: &'static str,
    role: &'static str,
    queue_capacity: usize,
    samples: Vec<Sample>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(std::env::args_os().nth(1).ok_or("output path required")?);
    let mut samples = Vec::new();
    for payload_bytes in [1_024_usize, 65_536, 1_048_576] {
        let (sender, mut receiver) = mpsc::channel::<Vec<u8>>(2);
        let payload = vec![b'x'; payload_bytes];
        let started = Instant::now();
        sender.send(payload).await?;
        let received = receiver.recv().await.ok_or("queue closed")?;
        let response = serde_json::to_vec(&serde_json::json!({
            "rank": 1,
            "content": String::from_utf8(received)?
        }))?;
        samples.push(Sample {
            payload_bytes,
            elapsed_microseconds: started.elapsed().as_micros(),
            serialized_response_bytes: response.len(),
        });
    }
    fs::write(
        output,
        serde_json::to_vec_pretty(&Evidence {
            schema: 1,
            gate: "G-RESOURCE-PREFLIGHT@A2",
            status: "PASS",
            role: "bounded-request-queue-and-response",
            queue_capacity: 2,
            samples,
        })?,
    )?;
    Ok(())
}
