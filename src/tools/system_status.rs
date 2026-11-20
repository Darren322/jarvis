//! What `system_status` tells the model: the figures the Dashboard shows, in the
//! Dashboard's units (`ui/src/panels/format.ts`), so the screen and the answer agree.

use std::time::Duration;

use serde::{Serialize, Serializer};
use sysinfo::System;

use crate::features::stats::machine::{Machine, MachineReading};

/// Wait between the two readings that make a CPU figure: `sysinfo` measures CPU
/// as the difference between two refreshes at least this far apart.
pub(crate) const CPU_WARM_UP: Duration = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL;

/// Bytes in one GB of memory as the Dashboard counts it: 2^30.
const RAM_BYTES_PER_GB: f64 = 1_073_741_824.0;

/// Bytes in one GB of storage as the Dashboard counts it: 10^9.
const STORAGE_BYTES_PER_GB: u64 = 1_000_000_000;

/// What a figure that could not be read is called on the wire. Never 0.
const UNAVAILABLE: &str = "unavailable";

/// One figure, or the statement that it could not be read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Figure<T> {
    Known(T),
    Unavailable,
}

impl<T: Serialize> Serialize for Figure<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Known(value) => value.serialize(serializer),
            Self::Unavailable => serializer.serialize_str(UNAVAILABLE),
        }
    }
}

/// The Pi's state as the model reads it. The unit is in each field's name.
#[derive(Debug, PartialEq, Serialize)]
pub struct SystemStatus {
    pub hostname: Option<String>,
    pub uptime_seconds: u64,
    pub cpu_percent: Figure<u32>,
    pub temperature_c: Figure<i32>,
    pub memory_used_gb: Figure<f64>,
    pub memory_total_gb: Figure<f64>,
    pub storage_used_gb: Figure<u64>,
    pub storage_total_gb: Figure<u64>,
}

impl SystemStatus {
    fn from_reading(reading: Option<MachineReading>) -> Self {
        let mut status = Self {
            hostname: System::host_name(),
            uptime_seconds: System::uptime(),
            cpu_percent: Figure::Unavailable,
            temperature_c: Figure::Unavailable,
            memory_used_gb: Figure::Unavailable,
            memory_total_gb: Figure::Unavailable,
            storage_used_gb: Figure::Unavailable,
            storage_total_gb: Figure::Unavailable,
        };
        let Some(reading) = reading else {
            return status;
        };
        status.apply(&reading);
        status
    }

    fn apply(&mut self, reading: &MachineReading) {
        if reading.cpu_percent.is_finite() {
            // Clamped to 0..=100 first, so the value fits `u32`.
            self.cpu_percent = Figure::Known(reading.cpu_percent.clamp(0.0, 100.0).round() as u32);
        }
        if let Some(celsius) = reading.temperature_c.filter(|celsius| celsius.is_finite()) {
            // `as` saturates; a real temperature is far inside `i32`.
            self.temperature_c = Figure::Known(celsius.round() as i32);
        }
        // A total of 0 is how `sysinfo` reports a failed read, not an empty machine.
        if reading.ram_total_bytes > 0 {
            self.memory_used_gb = Figure::Known(ram_gb(reading.ram_used_bytes));
            self.memory_total_gb = Figure::Known(ram_gb(reading.ram_total_bytes));
        }
        if let Some(storage) = reading.storage {
            let used = storage.total_bytes.saturating_sub(storage.available_bytes);
            self.storage_used_gb = Figure::Known(storage_gb(used));
            self.storage_total_gb = Figure::Known(storage_gb(storage.total_bytes));
        }
    }
}

/// Memory bytes as GB of 2^30 to one decimal place; a tie rounds up, as `toFixed(1)` does.
fn ram_gb(bytes: u64) -> f64 {
    // Precision loss above 2^53 bytes is far beyond any memory.
    (bytes as f64 / RAM_BYTES_PER_GB * 10.0).round() / 10.0
}

/// Storage bytes as whole GB of 10^9; half a GB rounds up, as `Math.round` does.
fn storage_gb(bytes: u64) -> u64 {
    bytes.saturating_add(STORAGE_BYTES_PER_GB / 2) / STORAGE_BYTES_PER_GB
}

/// Reads the machine twice, [`CPU_WARM_UP`] apart, so the CPU figure covers that
/// interval, and reports the second reading. A failed reading leaves every
/// machine figure unavailable; the hostname and uptime do not come from it.
pub(crate) async fn collect_system_status(machine: &mut impl Machine) -> SystemStatus {
    SystemStatus::from_reading(read_warm(machine).await)
}

async fn read_warm(machine: &mut impl Machine) -> Option<MachineReading> {
    machine.read().await.ok()?;
    tokio::time::sleep(CPU_WARM_UP).await;
    machine.read().await.ok()
}

#[cfg(test)]
#[path = "../../tests/unit/tools/system_status_tests.rs"]
mod tests;
