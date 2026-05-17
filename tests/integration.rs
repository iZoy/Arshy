//! Integration tests — spawn real daemon + proxy processes and validate end-to-end.
//!
//! These tests require the `arshyd` binary to be built (`cargo build --bin arshyd`).
//! They use temporary directories for socket and database files.

#[cfg(test)]
mod integration_tests {
    use std::process::Command;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Find the arshyd binary — prefers `target/debug/arshyd` (freshly built),
    /// falls back to `~/.cargo/bin/arshyd` (installed).
    fn arshyd_binary() -> std::path::PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let candidates = [
            std::path::PathBuf::from("target/debug/arshyd"),
            std::path::PathBuf::from(&home).join(".cargo/bin/arshyd"),
        ];
        for path in &candidates {
            if path.exists() {
                return path.clone();
            }
        }
        panic!(
            "arshyd not found. Build it first: cargo build --bin arshyd\n\
             Searched: {:?}",
            candidates
        );
    }

    /// Spawn the real arshyd daemon, wait for it to be ready, return the
    /// socket path and a handle that kills the daemon on drop.
    fn spawn_daemon() -> (std::path::PathBuf, std::process::Child, TempDir) {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let db_path = tmp.path().join("test.db");

        let mut child = Command::new(arshyd_binary())
            .env("ARSHY_DAEMON_SOCKET_PATH", socket_path.to_string_lossy().as_ref())
            .env("ARSHY_STORE_DB_PATH", db_path.to_string_lossy().as_ref())
            .env("ARSHY_DAEMON_LOG_LEVEL", "error")
            .env("ARSHY_TEST_NO_LIFECYCLE", "1")
            .spawn()
            .expect("failed to spawn arshyd");

        // Wait up to 5 seconds for the socket to appear
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !socket_path.exists() {
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                panic!("arshyd did not create socket within 5s");
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        (socket_path, child, tmp)
    }

    #[test]
    #[ignore = "requires no existing arshyd daemon running"]
    fn daemon_starts_and_responds_to_health() {
        let (socket_path, mut child, _tmp) = spawn_daemon();

        // Connect via Unix stream
        let stream = std::os::unix::net::UnixStream::connect(&socket_path)
            .expect("failed to connect to daemon");

        // Send a health check
        use std::io::{BufRead, BufReader, Write};
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "daemon/health",
            "params": {}
        });
        writeln!(writer, "{}", serde_json::to_string(&request).unwrap()).unwrap();
        writer.flush().unwrap();

        let mut response = String::new();
        reader.read_line(&mut response).unwrap();
        let resp: serde_json::Value =
            serde_json::from_str(&response).expect("invalid JSON response");

        // Verify the response structure
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["status"], "ok");
        assert!(resp["result"]["store_ok"].as_bool().unwrap());
        assert!(resp["result"]["uptime_secs"].as_f64().unwrap() >= 0.0);
        // Telemetry counters should be present
        assert!(resp["result"]["counters"].is_object());

        // Clean shutdown
        let shutdown = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "daemon/shutdown",
            "params": {}
        });
        writeln!(writer, "{}", serde_json::to_string(&shutdown).unwrap()).unwrap();
        writer.flush().unwrap();

        child.wait().expect("daemon did not exit cleanly");
    }

    #[test]
    #[ignore = "requires no existing arshyd daemon running"]
    fn run_echo_and_get_result() {
        let (socket_path, mut child, _tmp) = spawn_daemon();

        let stream = std::os::unix::net::UnixStream::connect(&socket_path)
            .expect("failed to connect to daemon");

        use std::io::{BufRead, BufReader, Write};
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;

        // Send a run request
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "task/run",
            "params": {
                "command": "echo hello-integration-test",
                "mode": "sync"
            }
        });
        writeln!(writer, "{}", serde_json::to_string(&request).unwrap()).unwrap();
        writer.flush().unwrap();

        let mut response = String::new();
        reader.read_line(&mut response).unwrap();
        let resp: serde_json::Value =
            serde_json::from_str(&response).expect("invalid JSON response");

        // Verify structured output
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["status"], "completed");
        assert_eq!(resp["result"]["exit_code"], 0);
        assert!(resp["result"]["task_id"].is_string());
        assert!(resp["result"]["event_count"].as_u64().unwrap() >= 1);

        // Clean shutdown
        let shutdown = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "daemon/shutdown",
            "params": {}
        });
        writeln!(writer, "{}", serde_json::to_string(&shutdown).unwrap()).unwrap();
        writer.flush().unwrap();

        child.wait().expect("daemon did not exit cleanly");
    }

    #[test]
    #[ignore = "requires no existing arshyd daemon running"]
    fn run_command_with_parser() {
        let (socket_path, mut child, _tmp) = spawn_daemon();

        let stream = std::os::unix::net::UnixStream::connect(&socket_path)
            .expect("failed to connect to daemon");

        use std::io::{BufRead, BufReader, Write};
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;

        // Write a Rust file with a compile error
        let src = std::env::temp_dir().join("arshy_int_test.rs");
        std::fs::write(&src, "fn main() {\n    let x: u32 = \"hello\";\n}\n").unwrap();

        // Run rustc
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "task/run",
            "params": {
                "command": format!("rustc {}", src.display()),
                "mode": "sync"
            }
        });
        writeln!(writer, "{}", serde_json::to_string(&request).unwrap()).unwrap();
        writer.flush().unwrap();

        let mut response = String::new();
        reader.read_line(&mut response).unwrap();
        let resp: serde_json::Value =
            serde_json::from_str(&response).expect("invalid JSON response");

        // Should have failed with a diagnostic event
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["status"], "failed");
        assert!(resp["result"]["exit_code"].as_i64().unwrap() != 0);
        assert!(resp["result"]["event_count"].as_u64().unwrap() >= 1);

        // Cleanup
        let _ = std::fs::remove_file(&src);

        // Shutdown
        let shutdown = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "daemon/shutdown",
            "params": {}
        });
        writeln!(writer, "{}", serde_json::to_string(&shutdown).unwrap()).unwrap();
        writer.flush().unwrap();

        child.wait().expect("daemon did not exit cleanly");
    }
}
