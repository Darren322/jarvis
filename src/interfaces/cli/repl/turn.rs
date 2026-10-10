use std::io;

use tokio::sync::mpsc;

use crate::assistant::{Assistant, AssistantTextDelta};
use crate::conversation::ConversationSession;
use crate::interfaces::cli::presentation::AssistantPresenter;
use crate::memory::MemoryService;

pub(super) async fn stream_turn<P>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    memory: &mut Option<MemoryService>,
    presenter: &mut P,
    prompt: &str,
) -> io::Result<crate::conversation::ConversationTurn>
where
    P: AssistantPresenter,
{
    let (sender, mut receiver) = mpsc::channel(32);
    // The response future and bounded receiver stay in this foreground scope. A
    // terminal write failure drops the native Rig run; new input remains queued
    // until this serial foreground response completes.
    let mut response = Box::pin(session.respond_stream(assistant, memory.as_mut(), prompt, sender));
    let mut receiver_open = true;

    enum Event {
        Delta(Option<AssistantTextDelta>),
        Response(Box<crate::conversation::ConversationTurn>),
    }

    loop {
        let event = tokio::select! {
            delta = receiver.recv(), if receiver_open => Event::Delta(delta),
            turn = &mut response => Event::Response(Box::new(turn)),
        };

        match event {
            Event::Delta(Some(delta)) => {
                if let Err(error) = presenter.delta(&delta.text) {
                    drop(response);
                    let _ = session.record_interrupted(prompt).await;
                    return Err(error);
                }
            }
            Event::Delta(None) => receiver_open = false,
            Event::Response(turn) => {
                drop(response);
                drain_deltas(&mut receiver, presenter)?;
                return Ok(*turn);
            }
        }
    }
}

fn drain_deltas<P: AssistantPresenter>(
    receiver: &mut mpsc::Receiver<AssistantTextDelta>,
    presenter: &mut P,
) -> io::Result<()> {
    while let Ok(delta) = receiver.try_recv() {
        presenter.delta(&delta.text)?;
    }
    Ok(())
}
