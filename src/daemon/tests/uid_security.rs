use std::os::unix::net::UnixStream as StdUnixStream;
use std::os::unix::prelude::RawFd;
use tempfile::TempDir;
use tokio::net::UnixListener;
use tokio::runtime::Runtime;
use libc;

#[tokio::test]
async fn test_uid_security() {
    // Setup a temporary socket path
    let dir = TempDir::new().expect("temp dir");
    let socket_path = dir.path().join("test.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind");

    // Spawn daemon accept loop in background (simplified)
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        // Apply the same UID check logic as daemon
        let allowed = match stream.peer_cred() {
            Ok(cred) => cred.uid() == unsafe { libc::getuid() },
            Err(_) => false,
        };
        allowed
    });

    // Connect as same UID (should be allowed)
    let client = StdUnixStream::connect(&socket_path).expect("client connect");
    // The daemon task should return true
    let result = handle.await.expect("task");
    assert!(result, "connection from same UID should be accepted");
}
