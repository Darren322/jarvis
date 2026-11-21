use std::io;

use tokio::io::AsyncBufRead;

use crate::features::memory::{EmbedError, IdlePass, MemoryError, MemoryService};
use crate::interfaces::cli::input::{InputLine, InputReader, read_input_line};
use crate::interfaces::cli::presentation::AssistantPresenter;
use crate::storage::{Examination, FactId};

use super::InputFuture;

/// Waits for the next input line, using the wait to embed pending memories and then
/// to note facts from turns not yet examined. A typed line always wins: it is
/// returned at once, and whatever was left undone is done in a later idle.
/// Returns the line and an optional warning to print.
///
/// Each fact noted is shown with `presenter` as soon as it is stored; they are
/// terminal lines, not spoken. `last_noted` is set to the last fact shown, so the
/// caller can make it the "forget that" focus; it is an out-parameter, not part of the
/// result, so it still holds the fact when a tap drops this future before it returns.
pub(super) async fn read_line_during_idle_work<R, P>(
    input: &mut InputReader<R>,
    memory: Option<&MemoryService>,
    presenter: &mut P,
    last_noted: &mut Option<FactId>,
) -> io::Result<(InputLine, Option<String>)>
where
    R: AsyncBufRead + Unpin,
    P: AssistantPresenter,
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
    // At most one line per kind of trouble in this idle period, printed with the line.
    let mut warnings: Vec<String> = Vec::new();

    // Embedding first. It ends when nothing is left to embed or embedding cannot
    // continue; neither stops the noting below, which talks to another server.
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
            Event::Input(line) => return with_warnings(line, warnings),
            Event::WorkerIdle => {}
            Event::Idle(Ok(pass)) if pass.has_made_progress() => {}
            Event::Idle(Ok(_)) => break,
            Event::Idle(Err(error)) => {
                warnings.push(format!("local memory indexing paused: {error}"));
                break;
            }
        }
    }

    // Then facts, one turn at a time, oldest first.
    if let Some(noter) = memory.fact_noter() {
        loop {
            // The model run is the part that waits, and the only part input may cut
            // short: it stores nothing, so a dropped run leaves its turn unexamined.
            let extracted = tokio::select! {
                biased;
                line = &mut input_future => return with_warnings(line, warnings),
                extracted = noter.extract_next_turn() => extracted,
            };
            let turn = match extracted {
                Ok(Some(turn)) => turn,
                Ok(None) => break,
                Err(error) => {
                    // The message is fixed and carries no text from the turn or the
                    // model. The loop ends here, so a failing turn is not retried
                    // before the next idle period.
                    warnings.push(format!("fact noting paused: {error}"));
                    break;
                }
            };
            // Storing is not raced with input: it is one short transaction that
            // finishes even if its caller stops waiting, so it is awaited here, and
            // a line typed meanwhile is read right after.
            match noter.commit(turn).await {
                Ok(examination) => {
                    show_noted(presenter, &examination)?;
                    if let Some(fact) = examination.noted.last() {
                        *last_noted = Some(fact.id);
                    }
                }
                Err(error) => {
                    warnings.push(format!("fact noting paused: {error}"));
                    break;
                }
            }
        }
    }

    with_warnings(input_future.await, warnings)
}

fn with_warnings(
    line: io::Result<InputLine>,
    warnings: Vec<String>,
) -> io::Result<(InputLine, Option<String>)> {
    let warning = (!warnings.is_empty()).then(|| warnings.join(" "));
    line.map(|line| (line, warning))
}

/// Shows the facts one examination kept, and in one more line which older noted facts
/// it dropped to make room.
fn show_noted<P: AssistantPresenter>(
    presenter: &mut P,
    examination: &Examination,
) -> io::Result<()> {
    for fact in &examination.noted {
        presenter.notice(&format!("Noted: {}", fact.text))?;
    }
    if !examination.dropped.is_empty() {
        let dropped: Vec<&str> = examination
            .dropped
            .iter()
            .map(|fact| fact.text.as_str())
            .collect();
        presenter.notice(&format!(
            "Dropped to make room for them: {}",
            dropped.join("; ")
        ))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/unit/interfaces/cli/repl/idle_tests.rs"]
mod tests;
