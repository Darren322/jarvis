use crate::tools::system_status::SystemStatus;
use std::time::SystemTime;

#[test]
fn system_status_can_represent_unavailable_hostname() {
    let status = SystemStatus {
        source_node: None,
        observed_at: SystemTime::now(),
        uptime_seconds: 100,
        total_memory_bytes: 1024,
        used_memory_bytes: 512,
    };

    assert_eq!(status.source_node, None);
}
