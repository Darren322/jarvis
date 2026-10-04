use serde::Serialize;
use serde_with::{TimestampMilliSeconds, serde_as};
use std::time::SystemTime;
use sysinfo::System;

#[serde_as]
#[derive(Debug, Serialize)]
pub struct SystemStatus {
    pub source_node: Option<String>,

    #[serde_as(as = "TimestampMilliSeconds<i64>")]
    #[serde(rename = "observed_at_unix_ms")]
    pub observed_at: SystemTime,
    pub uptime_seconds: u64,
    pub total_memory_bytes: u64,
    pub used_memory_bytes: u64,
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
