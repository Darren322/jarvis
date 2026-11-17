use super::*;

#[test]
fn no_temperature_sensor_means_no_temperature() {
    assert_eq!(cpu_temperature(&Components::new()), None);
}

#[tokio::test]
async fn the_real_machine_reads_memory_that_fits_in_total() {
    let mut machine = SystemMachine::new();
    let reading = machine.read().await.expect("a reading");
    assert!(reading.ram_total_bytes > 0);
    assert!(reading.ram_used_bytes <= reading.ram_total_bytes);
    assert!(reading.cpu_percent.is_finite());
    // Never 0 for a missing sensor: absent or a real number.
    assert!(reading.temperature_c.is_none_or(f32::is_finite));
}

#[test]
fn a_disk_that_reports_no_capacity_is_no_storage_never_zero() {
    assert_eq!(storage_usage(0, 0), None);
    assert_eq!(storage_usage(5, 0), None);
    assert_eq!(
        storage_usage(0, 1),
        Some(StorageUsage {
            available_bytes: 0,
            total_bytes: 1
        })
    );
}

#[test]
fn no_disk_at_the_mount_point_means_no_storage() {
    assert_eq!(root_storage(&mut Disks::new()), None);
}

#[tokio::test]
async fn the_real_machine_reads_storage_that_fits_in_total_or_none() {
    let mut machine = SystemMachine::new();
    let reading = machine.read().await.expect("a reading");
    if let Some(storage) = reading.storage {
        assert!(storage.total_bytes > 0);
        assert!(storage.available_bytes <= storage.total_bytes);
    }
}
