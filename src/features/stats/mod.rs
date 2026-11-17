//! The stats Dashboard's data: this Pi's CPU, temperature, RAM and root-disk space, and whether
//! llama and BGE on the 16 GB Pi answer their health checks.
//!
//! [`StatsSource::run`] pushes one `ToOrb::Stats` every [`STATS_INTERVAL`] while
//! the Dashboard is shown and nothing while it is hidden. The UI never polls.

pub(crate) mod machine;

use std::time::Duration;

use orb_protocol::{CpuPercent, PanelName, ServiceHealth, ToOrb};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{Instant, MissedTickBehavior},
};

use crate::clients::health::{HealthCheck, HealthStatus};
use machine::{Machine, MachineReading};

/// Time between two Stats messages while the Dashboard is shown.
pub(crate) const STATS_INTERVAL: Duration = Duration::from_secs(1);

/// Whether the stats Dashboard is on screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatsVisibility {
    Shown,
    Hidden,
}

impl StatsVisibility {
    /// The Dashboard is on screen exactly while the stats panel is the shown one.
    /// Exhaustive on purpose: a new panel must say whether it needs these stats.
    pub(crate) fn of_panel(shown: Option<PanelName>) -> Self {
        match shown {
            Some(PanelName::Stats) => Self::Shown,
            None => Self::Hidden,
        }
    }
}

/// Mirrors the panel state into `visibility` until the panel state is gone. The
/// panel state publishes only which panel is shown; this is where that becomes
/// "stats are wanted". A change that leaves the answer the same wakes nobody.
pub(crate) async fn mirror_panel(
    mut shown: watch::Receiver<Option<PanelName>>,
    visibility: watch::Sender<StatsVisibility>,
) {
    loop {
        let wanted = StatsVisibility::of_panel(*shown.borrow_and_update());
        visibility.send_if_modified(|current| {
            let changed = *current != wanted;
            *current = wanted;
            changed
        });
        if shown.changed().await.is_err() {
            return;
        }
    }
}

/// One server's last known health, and the check that will refresh it. At most
/// one check is in flight, and none ever blocks a tick: a tick collects a
/// finished check, starts the next, and reports what it knows.
struct Probe {
    check: HealthCheck,
    latest: HealthStatus,
    in_flight: Option<JoinHandle<HealthStatus>>,
}

impl Probe {
    fn new(check: HealthCheck) -> Self {
        Self {
            check,
            // Down until a check says otherwise: nothing has answered yet.
            latest: HealthStatus::Down,
            in_flight: None,
        }
    }

    /// Takes the answer of a finished check (never waits for an unfinished one),
    /// then starts a new check if none is running.
    async fn refresh(&mut self) {
        if let Some(handle) = self.in_flight.take_if(|handle| handle.is_finished()) {
            // A finished handle resolves at once. A panicked check is Down.
            self.latest = handle.await.unwrap_or(HealthStatus::Down);
        }
        if self.in_flight.is_none() {
            let check = self.check.clone();
            self.in_flight = Some(tokio::spawn(async move { check.check().await }));
        }
    }

    /// Abandons a running check (the Dashboard was hidden); `latest` is kept.
    fn stop(&mut self) {
        if let Some(handle) = self.in_flight.take() {
            handle.abort();
        }
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Reads this machine and the model servers, and feeds the orb's Dashboard.
pub(crate) struct StatsSource<M> {
    machine: M,
    llama: Probe,
    bge: Probe,
}

impl<M: Machine> StatsSource<M> {
    pub(crate) fn new(machine: M, llama: HealthCheck, bge: HealthCheck) -> Self {
        Self {
            machine,
            llama: Probe::new(llama),
            bge: Probe::new(bge),
        }
    }

    /// Runs until `visibility`'s sender is dropped or `to_orb` is closed.
    ///
    /// A full `to_orb` queue drops that reading: the next one is a second away
    /// and a stale one is worth nothing. Every await is safe to drop.
    pub(crate) async fn run(
        mut self,
        mut visibility: watch::Receiver<StatsVisibility>,
        to_orb: mpsc::Sender<ToOrb>,
    ) {
        loop {
            while *visibility.borrow_and_update() == StatsVisibility::Hidden {
                if visibility.changed().await.is_err() {
                    return;
                }
            }
            if !self.show(&mut visibility, &to_orb).await {
                return;
            }
        }
    }

    /// One stretch of the Dashboard being shown. `false` means stop for good.
    async fn show(
        &mut self,
        visibility: &mut watch::Receiver<StatsVisibility>,
        to_orb: &mpsc::Sender<ToOrb>,
    ) -> bool {
        // Warm-up: CPU usage is a difference between readings, so one taken now
        // (and dropped) makes the first tick's reading cover just the last
        // interval, not the whole time the Dashboard was hidden. Checks start now
        // so the first tick, one interval away, already has answers.
        let _ = self.machine.read().await;
        self.llama.refresh().await;
        self.bge.refresh().await;
        let mut ticks = tokio::time::interval_at(Instant::now() + STATS_INTERVAL, STATS_INTERVAL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let keep_running = loop {
            tokio::select! {
                // Hiding wins over a tick that is due at the same moment.
                biased;
                changed = visibility.changed() => {
                    if changed.is_err() {
                        break false;
                    }
                    if *visibility.borrow_and_update() == StatsVisibility::Hidden {
                        break true;
                    }
                }
                _ = ticks.tick() => {
                    if !self.publish(to_orb).await {
                        break false;
                    }
                }
            }
        };
        self.llama.stop();
        self.bge.stop();
        keep_running
    }

    /// Sends one reading. `false` means the orb side is gone.
    async fn publish(&mut self, to_orb: &mpsc::Sender<ToOrb>) -> bool {
        self.llama.refresh().await;
        self.bge.refresh().await;
        // A failed reading skips this tick; a made-up one would be shown as fact.
        let Ok(reading) = self.machine.read().await else {
            return true;
        };
        let message = stats_message(reading, self.llama.latest, self.bge.latest);
        !matches!(
            to_orb.try_send(message),
            Err(mpsc::error::TrySendError::Closed(_))
        )
    }
}

/// Makes a raw reading valid for the wire: CPU within 0 to 100, no temperature
/// instead of a bad one, storage only when read, RAM used never above RAM total.
fn stats_message(reading: MachineReading, llama: HealthStatus, bge: HealthStatus) -> ToOrb {
    ToOrb::Stats {
        cpu_percent: CpuPercent::clamped(reading.cpu_percent),
        temperature_c: reading.temperature_c.filter(|celsius| celsius.is_finite()),
        ram_used_bytes: reading.ram_used_bytes.min(reading.ram_total_bytes),
        ram_total_bytes: reading.ram_total_bytes,
        storage: reading.storage,
        llama: service_health(llama),
        bge: service_health(bge),
    }
}

fn service_health(status: HealthStatus) -> ServiceHealth {
    match status {
        HealthStatus::Up => ServiceHealth::Up,
        HealthStatus::Down => ServiceHealth::Down,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/stats/stats_tests.rs"]
mod tests;
