use std::io;

use tokio::io::AsyncBufRead;

use crate::interfaces::cli::input::{InputLine, InputReader, read_input_line};
use crate::memory::MemoryService;

use super::InputFuture;

pub(super) async fn wait_for_input_or_idle_job<R>(
    input: &mut InputReader<R>,
    memory: &mut Option<MemoryService>,
) -> io::Result<(InputLine, Option<String>)>
where
    R: AsyncBufRead + Unpin,
{
    let mut input_future: InputFuture<'_> = Box::pin(read_input_line(input));
    enum Event {
        Input(io::Result<InputLine>),
        Idle(Result<crate::memory::JobRun, crate::memory::MemoryError>),
    }
    enum WorkerWaitEvent {
        Input(io::Result<InputLine>),
        Finished,
    }

    loop {
        if memory.is_none() {
            return input_future.await.map(|line| (line, None));
        }
        // LEARNING: `select!` polls the same owned input future on every bounded
        // idle pass. `biased` gives an already-ready line priority, and dropping
        // the losing memory future lets its service settle leases without
        // abandoning its separately owned native worker.
        let event = {
            let memory_service = memory
                .as_mut()
                .expect("memory presence checked immediately above");
            tokio::select! {
                biased;
                line = &mut input_future => Event::Input(line),
                result = memory_service.run_one_idle_job() => Event::Idle(result),
            }
        };

        match event {
            Event::Input(line) => {
                let warning = if let Some(memory) = memory.as_mut() {
                    memory.interrupt_idle_work().await.err().map(|error| {
                        format!("local memory work is still settling after input: {error}")
                    })
                } else {
                    None
                };
                return line.map(|line| (line, warning));
            }
            Event::Idle(Ok(job)) if job.made_progress() => {}
            Event::Idle(Ok(_)) => return input_future.await.map(|line| (line, None)),
            Event::Idle(Err(crate::memory::MemoryError::WorkerBusy)) => {
                // LEARNING: WorkerBusy means the service still owns native
                // inference. This borrowed wait observes its completion; if
                // input wins, dropping the wait future keeps the worker owned
                // while interrupt_idle_work performs only bounded bookkeeping.
                let event = {
                    let memory_service = memory
                        .as_mut()
                        .expect("memory presence checked immediately above");
                    tokio::select! {
                        biased;
                        line = &mut input_future => WorkerWaitEvent::Input(line),
                        _ = memory_service.wait_for_native_worker() => WorkerWaitEvent::Finished,
                    }
                };
                match event {
                    WorkerWaitEvent::Input(line) => {
                        let warning = if let Some(memory) = memory.as_mut() {
                            memory.interrupt_idle_work().await.err().map(|error| {
                                format!("local memory work is still settling after input: {error}")
                            })
                        } else {
                            None
                        };
                        return line.map(|line| (line, warning));
                    }
                    WorkerWaitEvent::Finished => continue,
                }
            }
            Event::Idle(Err(error)) => {
                let line = input_future.await?;
                return Ok((
                    line,
                    Some(format!("local memory work did not complete: {error}")),
                ));
            }
        }
    }
}
