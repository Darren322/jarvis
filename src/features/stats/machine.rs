//! Reads the 8 GB Pi's own CPU, temperature and memory through `sysinfo`.

use std::sync::{Arc, Mutex, PoisonError};

use sysinfo::{Components, MemoryRefreshKind, System};
use thiserror::Error;
use tokio::task::JoinError;

/// One raw reading of this machine. Values are as `sysinfo` gave them; the
/// stats source makes them valid before they reach the wire.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MachineReading {
    /// Percent of all CPUs busy, since the previous reading.
    pub(crate) cpu_percent: f32,
    /// `None` when the machine has no temperature sensor that answers.
    pub(crate) temperature_c: Option<f32>,
    pub(crate) ram_used_bytes: u64,
    pub(crate) ram_total_bytes: u64,
}

#[derive(Debug, Error)]
pub(crate) enum MachineError {
    #[error("the system reading did not finish")]
    Task(#[source] JoinError),
}

/// Where readings come from. The real one is [`SystemMachine`]; tests use a
/// scripted one so the one-second cadence is the only thing under test.
pub(crate) trait Machine: Send + 'static {
    async fn read(&mut self) -> Result<MachineReading, MachineError>;
}

struct Sensors {
    system: System,
    components: Components,
}

/// The real machine. It refreshes only what it reads: CPU usage, memory and
/// temperatures, never processes.
pub(crate) struct SystemMachine {
    sensors: Arc<Mutex<Sensors>>,
}

impl SystemMachine {
    /// `sysinfo` computes CPU usage from the difference between two refreshes, so
    /// the first one alone is meaningless: this makes it. A reading at least
    /// `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL` (200 ms) later is the first real one;
    /// the stats interval is longer than that.
    pub(crate) fn new() -> Self {
        let mut system = System::new();
        system.refresh_cpu_usage();
        Self {
            sensors: Arc::new(Mutex::new(Sensors {
                system,
                components: Components::new_with_refreshed_list(),
            })),
        }
    }
}

impl Machine for SystemMachine {
    async fn read(&mut self) -> Result<MachineReading, MachineError> {
        let sensors = Arc::clone(&self.sensors);
        // procfs and sysfs reads: small, but file I/O, so off the async threads.
        // The lock lives only inside the blocking closure, never across an await.
        tokio::task::spawn_blocking(move || {
            let mut sensors = sensors.lock().unwrap_or_else(PoisonError::into_inner);
            sensors.system.refresh_cpu_usage();
            sensors
                .system
                .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
            sensors.components.refresh(false);
            MachineReading {
                cpu_percent: sensors.system.global_cpu_usage(),
                temperature_c: cpu_temperature(&sensors.components),
                ram_used_bytes: sensors.system.used_memory(),
                ram_total_bytes: sensors.system.total_memory(),
            }
        })
        .await
        .map_err(MachineError::Task)
    }
}

/// The hottest sensor labelled as the CPU's, else the hottest of any. `None`
/// when no sensor reports a number (`sysinfo` reports a failed read as `None` or NaN).
fn cpu_temperature(components: &Components) -> Option<f32> {
    let readings = || {
        components
            .list()
            .iter()
            .filter_map(|component| Some((component.label(), component.temperature()?)))
            .filter(|(_, celsius)| celsius.is_finite())
    };
    readings()
        .filter(|(label, _)| label.to_ascii_lowercase().contains("cpu"))
        .map(|(_, celsius)| celsius)
        .reduce(f32::max)
        .or_else(|| readings().map(|(_, celsius)| celsius).reduce(f32::max))
}

#[cfg(test)]
#[path = "../../../tests/unit/features/stats/machine_tests.rs"]
mod tests;
