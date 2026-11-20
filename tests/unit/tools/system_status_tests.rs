use std::time::Duration;

use orb_protocol::StorageUsage;
use rig_core::serde_json;
use tokio::time::Instant;

use crate::features::stats::machine::{Machine, MachineError, MachineReading};
use crate::tools::system_status::{CPU_WARM_UP, Figure, SystemStatus, collect_system_status};

const GB: u64 = 1_000_000_000;
const GIB: u64 = 1_073_741_824;

/// The 8 GB Pi's RAM and 256 GB drive, the same constants `ui/tests/panels/format.test.ts` uses.
const PI_RAM: u64 = 8_464_105_472;
const PI_DISK_TOTAL: u64 = 251_416_899_584;
const PI_DISK_AVAILABLE: u64 = 207_517_036_544;

/// The most tokens a tool result may take, by ticket 16.
const MAX_TOOL_OUTPUT_TOKENS: usize = 100;
/// Rough bytes per token for JSON; an estimate, not a tokenizer count.
const BYTES_PER_TOKEN_ESTIMATE: usize = 4;

/// Answers every read with the same result, or fails every read, and notes when each happened.
struct Scripted {
    reading: Option<MachineReading>,
    reads: Vec<Instant>,
}

impl Scripted {
    fn reading(reading: MachineReading) -> Self {
        Self {
            reading: Some(reading),
            reads: Vec::new(),
        }
    }

    fn failing() -> Self {
        Self {
            reading: None,
            reads: Vec::new(),
        }
    }
}

impl Machine for Scripted {
    async fn read(&mut self) -> Result<MachineReading, MachineError> {
        self.reads.push(Instant::now());
        match self.reading {
            Some(reading) => Ok(reading),
            None => Err(MachineError::Task(cancelled_task().await)),
        }
    }
}

async fn cancelled_task() -> tokio::task::JoinError {
    let task = tokio::spawn(std::future::pending::<()>());
    task.abort();
    task.await.expect_err("an aborted task does not finish")
}

fn reading() -> MachineReading {
    MachineReading {
        cpu_percent: 12.5,
        temperature_c: Some(54.5),
        ram_used_bytes: 2_469_606_195,
        ram_total_bytes: PI_RAM,
        storage: Some(StorageUsage {
            available_bytes: PI_DISK_AVAILABLE,
            total_bytes: PI_DISK_TOTAL,
        }),
    }
}

fn with_storage(available_bytes: u64, total_bytes: u64) -> MachineReading {
    MachineReading {
        storage: Some(StorageUsage {
            available_bytes,
            total_bytes,
        }),
        ..reading()
    }
}

async fn status_of(reading: MachineReading) -> serde_json::Value {
    let status = collect_system_status(&mut Scripted::reading(reading)).await;
    serde_json::to_value(status).expect("a status serializes")
}

#[tokio::test(start_paused = true)]
async fn a_fixed_reading_is_reported_in_the_dashboards_units() {
    let json = status_of(reading()).await;

    assert_eq!(json["cpu_percent"], 13);
    assert_eq!(json["temperature_c"], 55);
    assert_eq!(json["memory_used_gb"], 2.3);
    assert_eq!(json["memory_total_gb"], 7.9);
    assert_eq!(json["storage_used_gb"], 44);
    assert_eq!(json["storage_total_gb"], 251);
    assert!(json["uptime_seconds"].is_u64());
    assert!(json.get("hostname").is_some());
}

#[tokio::test(start_paused = true)]
async fn memory_matches_the_dashboard_at_its_rounding_boundaries() {
    // The same inputs and answers as `formatRamGb` in `ui/tests/panels/format.test.ts`,
    // plus an exact tie (a quarter GB), which must round up as `toFixed(1)` does.
    let cases: [(u64, f64); 7] = [
        (0, 0.0),
        (GIB, 1.0),
        (GIB / 4, 0.3),
        ((1.049 * GIB as f64) as u64, 1.0),
        ((1.051 * GIB as f64) as u64, 1.1),
        (PI_RAM, 7.9),
        (GIB / 4 - 1, 0.2),
    ];
    for (used, shown) in cases {
        let json = status_of(MachineReading {
            ram_used_bytes: used,
            ..reading()
        })
        .await;
        assert_eq!(json["memory_used_gb"], shown, "{used} bytes");
    }
}

#[tokio::test(start_paused = true)]
async fn storage_matches_the_dashboard_at_its_rounding_boundaries() {
    const TEN_GB: u64 = 10 * GB;
    // [available, total, used shown, total shown]: `formatStorage` in the UI tests,
    // so half a GB used rounds up and a drive with nothing available is all used.
    let cases: [(u64, u64, u64, u64); 9] = [
        (PI_DISK_TOTAL, PI_DISK_TOTAL, 0, 251),
        (0, PI_DISK_TOTAL, 251, 251),
        (PI_DISK_AVAILABLE, PI_DISK_TOTAL, 44, 251),
        (TEN_GB - 499_999_999, TEN_GB, 0, 10),
        (TEN_GB - 500_000_000, TEN_GB, 1, 10),
        (TEN_GB - 1_499_999_999, TEN_GB, 1, 10),
        (TEN_GB - 1_500_000_000, TEN_GB, 2, 10),
        (PI_DISK_TOTAL + 1, PI_DISK_TOTAL, 0, 251),
        (0, 9_999 * GB, 9_999, 9_999),
    ];
    for (available, total, used_shown, total_shown) in cases {
        let json = status_of(with_storage(available, total)).await;
        assert_eq!(
            json["storage_used_gb"], used_shown,
            "{available} of {total}"
        );
        assert_eq!(
            json["storage_total_gb"], total_shown,
            "{available} of {total}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn cpu_and_temperature_round_to_whole_numbers() {
    let cases: [(f32, f32, u32, i32); 4] = [
        (0.49, 54.49, 0, 54),
        (0.5, 54.5, 1, 55),
        (99.6, 99.6, 100, 100),
        (100.0, -0.4, 100, 0),
    ];
    for (cpu, celsius, cpu_shown, celsius_shown) in cases {
        let json = status_of(MachineReading {
            cpu_percent: cpu,
            temperature_c: Some(celsius),
            ..reading()
        })
        .await;
        assert_eq!(json["cpu_percent"], cpu_shown, "{cpu}%");
        assert_eq!(json["temperature_c"], celsius_shown, "{celsius} C");
    }
}

#[tokio::test(start_paused = true)]
async fn an_unreadable_temperature_is_unavailable_and_never_0() {
    let json = status_of(MachineReading {
        temperature_c: None,
        ..reading()
    })
    .await;

    assert_eq!(json["temperature_c"], "unavailable");
    assert_eq!(json["cpu_percent"], 13);
}

#[tokio::test(start_paused = true)]
async fn unreadable_storage_is_unavailable_and_never_0() {
    let json = status_of(MachineReading {
        storage: None,
        ..reading()
    })
    .await;

    assert_eq!(json["storage_used_gb"], "unavailable");
    assert_eq!(json["storage_total_gb"], "unavailable");
    assert_eq!(json["memory_total_gb"], 7.9);
}

#[tokio::test(start_paused = true)]
async fn a_memory_total_of_zero_is_unavailable() {
    let json = status_of(MachineReading {
        ram_total_bytes: 0,
        ..reading()
    })
    .await;

    assert_eq!(json["memory_used_gb"], "unavailable");
    assert_eq!(json["memory_total_gb"], "unavailable");
}

#[tokio::test(start_paused = true)]
async fn a_failed_reading_makes_every_machine_figure_unavailable_but_keeps_the_uptime() {
    let json = serde_json::to_value(collect_system_status(&mut Scripted::failing()).await)
        .expect("a status serializes");

    for field in [
        "cpu_percent",
        "temperature_c",
        "memory_used_gb",
        "memory_total_gb",
        "storage_used_gb",
        "storage_total_gb",
    ] {
        assert_eq!(json[field], "unavailable", "{field}");
    }
    assert!(json["uptime_seconds"].is_u64());
    assert!(json.get("hostname").is_some());
}

#[tokio::test(start_paused = true)]
async fn the_cpu_is_read_twice_at_least_the_minimum_interval_apart() {
    let mut machine = Scripted::reading(reading());

    collect_system_status(&mut machine).await;

    assert_eq!(machine.reads.len(), 2);
    assert!(machine.reads[1] - machine.reads[0] >= CPU_WARM_UP);
    assert!(CPU_WARM_UP >= Duration::from_millis(200));
}

#[test]
fn the_widest_output_stays_under_100_tokens() {
    // A 63-byte hostname (the DNS label limit), the largest uptime, and either the
    // widest numbers or six "unavailable" figures, whichever serializes longer.
    let hostname = Some("h".repeat(63));
    let numbers = SystemStatus {
        hostname: hostname.clone(),
        uptime_seconds: u64::MAX,
        cpu_percent: Figure::Known(100),
        temperature_c: Figure::Known(-100),
        memory_used_gb: Figure::Known(15.9),
        memory_total_gb: Figure::Known(15.9),
        storage_used_gb: Figure::Known(9_999),
        storage_total_gb: Figure::Known(9_999),
    };
    let unavailable = SystemStatus {
        hostname,
        uptime_seconds: u64::MAX,
        cpu_percent: Figure::Unavailable,
        temperature_c: Figure::Unavailable,
        memory_used_gb: Figure::Unavailable,
        memory_total_gb: Figure::Unavailable,
        storage_used_gb: Figure::Unavailable,
        storage_total_gb: Figure::Unavailable,
    };

    for status in [numbers, unavailable] {
        let bytes = serde_json::to_string(&status)
            .expect("a status serializes")
            .len();
        let tokens = bytes.div_ceil(BYTES_PER_TOKEN_ESTIMATE);
        println!("widest output: {bytes} bytes, about {tokens} tokens");
        assert!(tokens < MAX_TOOL_OUTPUT_TOKENS, "{bytes} bytes");
    }
}
