use tokio::sync::mpsc::error::TryRecvError;

use super::*;

/// A sentence of more than 40 characters that the chunker releases as soon as the
/// space after it arrives.
fn sentence(number: usize) -> String {
    format!("This is sentence number {number:03} of the answer.")
}

fn feed_with_queue(capacity: usize) -> (ChunkFeed, mpsc::Receiver<String>) {
    let (sender, receiver) = mpsc::channel(capacity);
    (ChunkFeed::new(sender), receiver)
}

#[test]
fn a_finished_chunk_is_queued_before_the_text_is_complete() {
    let (mut feed, mut queue) = feed_with_queue(4);

    feed.push(&format!("{} And a start", sentence(1)))
        .expect("queue has room");

    assert_eq!(queue.try_recv(), Ok(sentence(1)));
    assert_eq!(
        queue.try_recv(),
        Err(TryRecvError::Empty),
        "the queue stays open while text is still arriving"
    );
}

#[test]
fn finishing_queues_the_unfinished_rest_and_closes_the_queue() {
    let (mut feed, mut queue) = feed_with_queue(4);
    feed.push("Half a sentence").expect("queue has room");
    assert_eq!(queue.try_recv(), Err(TryRecvError::Empty));

    feed.finish().expect("queue has room");

    assert_eq!(queue.try_recv(), Ok("Half a sentence".to_owned()));
    assert_eq!(queue.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn the_queue_holds_its_capacity_and_the_next_chunk_overflows_once() {
    let capacity = 3;
    let (mut feed, mut queue) = feed_with_queue(capacity);
    for number in 0..capacity {
        feed.push(&format!("{} ", sentence(number)))
            .expect("the queue has room for its capacity");
    }

    assert_eq!(
        feed.push(&format!("{} ", sentence(capacity))),
        Err(SpeechQueueFull),
        "the first chunk beyond the capacity does not fit"
    );
    assert_eq!(
        feed.push(&format!("{} ", sentence(capacity + 1))),
        Ok(()),
        "after an overflow the feed is closed and says nothing more"
    );
    assert_eq!(feed.finish(), Ok(()));

    let mut queued = Vec::new();
    while let Ok(chunk) = queue.try_recv() {
        queued.push(chunk);
    }
    assert_eq!(
        queued,
        (0..capacity).map(sentence).collect::<Vec<_>>(),
        "what fit is spoken, nothing is reordered"
    );
    assert_eq!(queue.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn a_speech_side_that_has_gone_away_is_not_an_overflow() {
    let (mut feed, queue) = feed_with_queue(1);
    drop(queue);

    assert_eq!(feed.push(&format!("{} ", sentence(1))), Ok(()));
    assert_eq!(feed.finish(), Ok(()));
}

/// Speech double that never reads its queue, so chunks pile up.
struct StalledSpeech;

impl SpeechControl for StalledSpeech {
    type Error = std::io::Error;

    fn is_disabled(&self) -> bool {
        false
    }

    async fn speak(
        &mut self,
        chunks: mpsc::Receiver<String>,
        _playback_started: oneshot::Sender<()>,
    ) -> Result<(), Self::Error> {
        let _chunks = chunks;
        std::future::pending().await
    }

    async fn cancel(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn an_answer_may_queue_64_unspoken_chunks_but_not_65() {
    assert_eq!(SPEECH_CHUNK_QUEUE, 64);
    let mut speech = StalledSpeech;
    let mut answer = SpokenAnswer::start(&mut speech);

    for number in 0..SPEECH_CHUNK_QUEUE {
        assert_eq!(
            answer.feed.push(&format!("{} ", sentence(number))),
            Ok(()),
            "chunk {}",
            number + 1
        );
    }
    assert_eq!(
        answer
            .feed
            .push(&format!("{} ", sentence(SPEECH_CHUNK_QUEUE))),
        Err(SpeechQueueFull),
        "chunk {}",
        SPEECH_CHUNK_QUEUE + 1
    );
}
