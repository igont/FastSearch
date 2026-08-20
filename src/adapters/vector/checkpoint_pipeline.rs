//! Ordered, bounded persistence pipeline for embedding checkpoints.

use std::{sync::mpsc, thread};

use crate::domain::{ErrorKind, FastSearchError};

const CHECKPOINT_QUEUE_DEPTH: usize = 4;

type CheckpointCallback<'a> = dyn FnMut(usize, &[Vec<f32>]) -> Result<(), FastSearchError> + 'a;

struct PendingCheckpoint {
    completed_before: usize,
    vectors: Vec<Vec<f32>>,
}

/// Keeps durable checkpoint I/O ordered while allowing the inference loop to
/// start preparing and executing the next chunk. The bounded queue caps the
/// duplicated vector memory and applies backpressure if storage falls behind.
pub(super) fn run_with_checkpoint_pipeline<T>(
    embed: impl FnOnce(&mut CheckpointCallback<'_>) -> Result<T, FastSearchError>,
    mut checkpoint: impl FnMut(usize, &[Vec<f32>]) -> Result<(), FastSearchError> + Send,
) -> Result<T, FastSearchError> {
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel::<PendingCheckpoint>(CHECKPOINT_QUEUE_DEPTH);
        let writer = scope.spawn(move || {
            while let Ok(pending) = receiver.recv() {
                checkpoint(pending.completed_before, &pending.vectors)?;
            }
            Ok(())
        });
        let embedded = {
            let mut enqueue = |completed_before: usize, chunk: &[Vec<f32>]| {
                sender
                    .send(PendingCheckpoint {
                        completed_before,
                        vectors: chunk.to_vec(),
                    })
                    .map_err(|_| {
                        FastSearchError::new(
                            ErrorKind::ProjectionFailure,
                            "durable vector checkpoint pipeline stopped unexpectedly",
                        )
                    })
            };
            embed(&mut enqueue)
        };
        drop(sender);
        let checkpointed = writer.join().map_err(|_| {
            FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "durable vector checkpoint writer panicked",
            )
        })?;
        checkpointed?;
        embedded
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn writes_in_order_on_a_background_thread() {
        let caller = thread::current().id();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&writes);

        let embedded = run_with_checkpoint_pipeline(
            |checkpoint| {
                checkpoint(0, &[vec![1.0, 2.0]])?;
                checkpoint(1, &[vec![3.0, 4.0]])?;
                Ok("embedded")
            },
            move |completed_before, chunk| {
                assert_ne!(thread::current().id(), caller);
                captured
                    .lock()
                    .expect("checkpoint capture lock")
                    .push((completed_before, chunk.to_vec()));
                Ok(())
            },
        )
        .expect("checkpoint pipeline");

        assert_eq!(embedded, "embedded");
        assert_eq!(
            *writes.lock().expect("checkpoint capture lock"),
            vec![(0, vec![vec![1.0, 2.0]]), (1, vec![vec![3.0, 4.0]])]
        );
    }

    #[test]
    fn writer_failure_fails_the_embedding_operation() {
        let error = run_with_checkpoint_pipeline(
            |checkpoint| {
                checkpoint(0, &[vec![1.0]])?;
                Ok(())
            },
            |_, _| {
                Err(FastSearchError::new(
                    ErrorKind::ProjectionFailure,
                    "simulated checkpoint failure",
                ))
            },
        )
        .expect_err("checkpoint failure must remain visible");

        assert_eq!(error.message(), "simulated checkpoint failure");
    }
}
