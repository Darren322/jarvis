use std::io;

use tokio::io::AsyncBufRead;

use crate::features::memory::{EmbedError, IdlePass, MemoryError, MemoryService};
use crate::interfaces::cli::input::{InputLine, InputReader, read_input_line};

use super::InputFuture;

/// Waits for the next input line, using the wait to embed pending memories.
/// Returns the line and an optional memory warning to print.
pub(super) async fn read_line_during_idle_work<R>(
    input: &mut InputReader<R>,
    memory: Option<&MemoryService>,
) -> io::Result<(InputLine, Option<String>)>
where
    R: AsyncBufRead + Unpin,
{
    let mut input_future: InputFuture<'_> = Box::pin(read_input_line(input));
    let Some(memory) = memory else {
        return input_future.await.map(|line| (line, None));
    };
    enum Event {
        Input(io::Result<InputLine>),
        Idle(Result<IdlePass, MemoryError>),
        WorkerIdle,
    }

    loop {
        // LEARNING: `biased` polls input first, so a ready line always wins.
        // Dropping the losing idle future is safe: every stored vector commits
        // on its own, and dropping an in-flight embedding request only stops
        // Jarvis waiting for it, so nothing durable is left half-done.
        let event = tokio::select! {
            biased;
            line = &mut input_future => Event::Input(line),
            result = memory.run_idle_pass() => Event::Idle(result),
        };
        let event = match event {
            Event::Idle(Err(MemoryError::Embedding(EmbedError::Busy))) => tokio::select! {
                biased;
                line = &mut input_future => Event::Input(line),
                () = memory.wait_until_embedder_idle() => Event::WorkerIdle,
            },
            other => other,
        };

        match event {
            Event::Input(line) => return line.map(|line| (line, None)),
            Event::WorkerIdle => {}
            Event::Idle(Ok(pass)) if pass.has_made_progress() => {}
            Event::Idle(Ok(_)) => return input_future.await.map(|line| (line, None)),
            Event::Idle(Err(error)) => {
                let line = input_future.await?;
                return Ok((line, Some(format!("local memory indexing paused: {error}"))));
            }
        }
    }
}
