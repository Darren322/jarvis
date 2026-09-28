use std::time::SystemTime;

use rig_core::serde::Serialize;
use sysinfo::System;

#[derive(Debug, Serialize)]
pub struct SystemStatus {
    pub source_node: Option<String>,

    #[serde(skip)]
    pub observed_at: SystemTime,
    pub uptime_seconds: u64,
    pub total_memory_bytes: u64,
    pub used_memory_bytes: u64,
}

impl SystemStatus {
    pub fn total_memory_gib(&self) -> f64 {
        self.total_memory_bytes as f64 / 1024_f64.powi(3)
    }

    pub fn used_memory_gib(&self) -> f64 {
        self.used_memory_bytes as f64 / 1024_f64.powi(3)
    }
    pub fn formatted_uptime(&self) -> String {
        let days = self.uptime_seconds / (24 * 60 * 60);
        let hours = (self.uptime_seconds % (24 * 60 * 60)) / (60 * 60);
        let minutes = (self.uptime_seconds % (60 * 60)) / 60;
        let seconds = self.uptime_seconds % 60;

        format!("{days}d {hours:02}h {minutes:02}m {seconds:02}s")
    }
}
pub fn collect_system_status() -> SystemStatus {
    let mut system = System::new();

    system.refresh_memory();

    SystemStatus {
        source_node: System::host_name(),
        observed_at: SystemTime::now(),
        uptime_seconds: System::uptime(),
        total_memory_bytes: system.total_memory(),
        used_memory_bytes: system.used_memory(),
    }
}
