use crate::tools::system_status::SystemStatus;
use std::time::SystemTime;

#[test]
fn system_status_can_represent_unavailable_hostname() {
    let status = SystemStatus {
        source_node: None,
        observed_at: SystemTime::UNIX_EPOCH,
        uptime_seconds: 100,
        total_memory_bytes: 1024,
        used_memory_bytes: 512,
    };
    let json = rig_core::serde_json::to_value(&status).unwrap();
    assert_eq!(status.source_node, None);
    assert_eq!(json["observed_at_unix_ms"], 0);
}
