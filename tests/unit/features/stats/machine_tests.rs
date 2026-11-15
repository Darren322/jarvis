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
