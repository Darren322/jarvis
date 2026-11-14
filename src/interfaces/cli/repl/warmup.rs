use tokio::task::JoinHandle;

use crate::features::assistant::{Assistant, CacheWarmup};
use crate::interfaces::cli::diagnostics::error_chain;

/// The cache warm-up in flight, if any.
///
/// A warm-up runs beside the loop, so it overlaps the spoken answer, the wait for
/// the next input, listening and transcribing. Only the next model run ends it
/// early: the model server should be reading one prompt at a time, and the prompt
/// somebody is waiting for wins. Stopping it is always safe, because a warm-up
/// stores nothing and whatever the server had read stays cached.
#[derive(Default)]
pub(super) struct WarmupTask {
    task: Option<JoinHandle<()>>,
}

impl WarmupTask {
    /// Starts sending `warmup`, replacing any warm-up still running. A failure is
    /// reported as one diagnostics line; the only cost is a slower next answer.
    pub(super) fn start(&mut self, assistant: &Assistant, warmup: CacheWarmup) {
        self.cancel();
        let warming = assistant.warm_cache(warmup);
        self.task = Some(tokio::spawn(async move {
            if let Err(error) = warming.await {
                eprintln!(
                    "Cache warm-up did not finish, so the next answer may start slowly: {}",
                    error_chain(&error)
                );
            }
        }));
    }

    /// Stops the warm-up, if one is running.
    pub(super) fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for WarmupTask {
    fn drop(&mut self) {
        self.cancel();
    }
}
