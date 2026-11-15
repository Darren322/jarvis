use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn the_socket_is_for_its_owner_only_and_is_removed_with_the_server() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("orb.sock");
    let server = UnixEndpoint::bind(&path).unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, SOCKET_MODE);
    drop(server);
    assert!(!path.exists());
}

#[tokio::test]
async fn a_stale_socket_file_is_replaced() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("orb.sock");
    // A listener that is gone leaves its file behind, as a killed Jarvis would.
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists());

    let server = UnixEndpoint::bind(&path).unwrap();
    assert!(UnixStream::connect(&path).await.is_ok());
    drop(server);
}

#[tokio::test]
async fn a_socket_someone_is_serving_is_left_alone() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("orb.sock");
    let first = UnixEndpoint::bind(&path).unwrap();
    assert!(matches!(UnixEndpoint::bind(&path), Err(OrbError::InUse(_))));
    // The refused second server must not have removed the first one's file.
    assert!(path.exists());
    drop(first);
}

#[tokio::test]
async fn a_file_that_is_not_a_socket_is_never_deleted() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.txt");
    fs::write(&path, "keep me").unwrap();
    assert!(matches!(
        UnixEndpoint::bind(&path),
        Err(OrbError::NotASocket(_))
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
}

#[tokio::test]
async fn a_socket_in_a_missing_directory_is_reported() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("missing").join("orb.sock");
    assert!(matches!(
        UnixEndpoint::bind(&path),
        Err(OrbError::Bind { .. })
    ));
}
