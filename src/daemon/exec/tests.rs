use super::*;
use super::{is_short_command, Executor, ExecutorConfig};
use crate::config::{ParserConfig, SecurityConfig};
use crate::daemon::bus::EventBus;
use crate::daemon::parser::Engine;
use crate::daemon::store::Store;
use crate::ipc::TaskStatus;
use std::sync::Arc;
use tempfile::TempDir;

fn setup() -> (Arc<Store>, Arc<Engine>, EventBus, TempDir) {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let store = Arc::new(Store::open(&db_path).unwrap());
    store.initialize_schema().unwrap();
    let parser = Arc::new(Engine::new(&ParserConfig::default()).unwrap());
    let bus = EventBus::new();
    (store, parser, bus, tmp)
}

async fn wait_for_task_completion(store: &Store, task_id: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if store.get_task(task_id).unwrap().is_some_and(|task| task.status.is_terminal()) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("task {task_id} did not finish within 5s"));
}

#[test]
fn invalid_access_level_is_rejected() {
    let (store, parser, bus, _tmp) = setup();
    let security = SecurityConfig { access_level: "read_only".into(), ..Default::default() };
    let result = Executor::new(store, parser, bus).with_security(&security);
    assert!(
        matches!(result, Err(crate::ArshyError::Config(message)) if message.contains("security.access_level"))
    );
}

#[test]
fn executor_constructor_enforces_default_security_policy() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store, parser, bus);

    assert!(executor.filter.check("rm -rf /").is_err());
    assert!(executor.filter.check("echo hello").is_ok());
}

#[test]
fn invalid_enabled_rate_limit_is_rejected() {
    for (burst, max_commands_per_second) in [
        (0.5, 10.0),
        (f64::NAN, 10.0),
        (f64::INFINITY, 10.0),
        (1.0, 0.0),
        (1.0, f64::NAN),
        (1.0, f64::INFINITY),
    ] {
        let (store, parser, bus, _tmp) = setup();
        let security = SecurityConfig {
            rate_limit: crate::config::RateLimitConfig {
                enabled: true,
                burst,
                max_commands_per_second,
            },
            ..Default::default()
        };
        let result = Executor::new(store, parser, bus).with_security(&security);
        assert!(
            matches!(result, Err(crate::ArshyError::Config(message)) if message.contains("security.rate_limit"))
        );
    }
}

#[tokio::test]
async fn test_executor_run_echo() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run("echo hello", None, None, "async", None, None, false, None, None)
        .await
        .unwrap();
    assert!(!result.task_id.is_empty());
    assert_eq!(result.status, TaskStatus::Running);

    // Wait for the background task to complete
    wait_for_task_completion(&store, &result.task_id).await;

    // Check the task was updated in the store
    let task = store.get_task(&result.task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
    assert_eq!(task.exit_code, Some(0));
    assert!(task.duration_ms.is_some());

    // Check events were stored
    use crate::ipc::QueryParams;
    let params = QueryParams {
        task_id: Some(result.task_id.clone()),
        event_type: None,
        severity: None,
        code: None,
        file: None,
        limit: 100,
        offset: 0,
        include_logs: true,
    };
    let (events, total) = store.query_events(&params).unwrap();
    assert!(total >= 1, "expected at least 1 event, got {}", total);
    assert_eq!(events[0].message, "hello");
}

#[tokio::test]
async fn test_executor_run_failure() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result =
        executor.run("exit 1", None, None, "async", None, None, false, None, None).await.unwrap();

    wait_for_task_completion(&store, &result.task_id).await;

    let task = store.get_task(&result.task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.exit_code, Some(1));
}

#[tokio::test]
async fn test_executor_tail() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run("printf 'a\nb\nc\n'", None, None, "async", None, None, false, None, None)
        .await
        .unwrap();

    wait_for_task_completion(&store, &result.task_id).await;

    let lines = executor.tail(&result.task_id, 10, "raw").await.unwrap();
    assert_eq!(lines, vec!["a", "b", "c"]);
}

#[tokio::test]
async fn raw_tail_reports_missing_or_unreadable_output_instead_of_empty_success() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let empty =
        executor.run("true", None, None, "sync", None, None, false, None, None).await.unwrap();
    assert!(executor.tail(&empty.task_id, 0, "raw").await.unwrap().is_empty());

    let output = executor
        .run("printf 'captured output'", None, None, "sync", None, None, false, None, None)
        .await
        .unwrap();
    let raw_path = store.store_dir().join("raw").join(format!("{}.txt", output.task_id));
    std::fs::remove_file(&raw_path).unwrap();
    let missing_error = executor.tail(&output.task_id, 0, "raw").await.unwrap_err();
    assert!(
        missing_error.to_string().contains("raw output")
            && missing_error.to_string().contains("missing")
    );

    std::fs::create_dir(&raw_path).unwrap();
    assert!(executor.tail(&output.task_id, 0, "raw").await.is_err());
}

#[tokio::test]
async fn test_executor_timeout() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus)
        .with_config(ExecutorConfig { max_task_duration_ms: 500, ..Default::default() });

    let result = executor
        .run(
            "echo before-timeout; sleep 60",
            None,
            Some(500),
            "async",
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let task = store.get_task(&result.task_id).unwrap().unwrap();
    // Should be either Timeout or Killed (depending on timing)
    assert!(
        task.status == TaskStatus::Timeout || task.status == TaskStatus::Killed,
        "expected Timeout or Killed, got {:?}",
        task.status
    );
    if task.status == TaskStatus::Timeout {
        assert_eq!(task.exit_code, Some(-2));
    }
    let raw = executor.tail(&result.task_id, 0, "raw").await.unwrap().join("\n");
    assert!(raw.contains("before-timeout"), "partial output must survive timeout");
}

#[tokio::test]
async fn requested_timeout_cannot_exceed_daemon_maximum() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store, parser, bus)
        .with_config(ExecutorConfig { max_task_duration_ms: 100, ..Default::default() });
    let result = executor
        .run("sleep 2", None, Some(10_000), "sync", None, None, false, None, None)
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Timeout);
    assert_eq!(result.exit_code, Some(-2));
}

#[tokio::test]
async fn timeout_still_applies_after_both_output_streams_close() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store, parser, bus)
        .with_config(ExecutorConfig { max_task_duration_ms: 100, ..Default::default() });

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        executor.run(
            r#"exec 1>&- 2>&-; sleep 30"#,
            None,
            Some(100),
            "sync",
            None,
            None,
            false,
            None,
            None,
        ),
    )
    .await
    .expect("closed output streams must not disable the task timeout")
    .unwrap();

    assert_eq!(result.status, TaskStatus::Timeout);
    assert_eq!(result.exit_code, Some(-2));
}

#[tokio::test]
async fn short_path_timeout_still_applies_after_output_streams_close() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store, parser, bus)
        .with_config(ExecutorConfig { max_task_duration_ms: 100, ..Default::default() });

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        executor.run_short(r#"exec 1>&- 2>&-; sleep 30"#, None, Some(100), None),
    )
    .await
    .expect("closed output streams must not disable the short-command timeout")
    .unwrap();

    assert_eq!(result.status, TaskStatus::Timeout);
    assert_eq!(result.exit_code, Some(-2));
}

#[tokio::test]
async fn signal_terminated_structured_command_is_failed_not_timeout() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store, parser, bus);
    let result = executor
        .run("kill -TERM $$", None, None, "sync", None, None, false, None, None)
        .await
        .unwrap();

    assert_eq!(result.status, TaskStatus::Failed);
    assert_eq!(result.exit_code, Some(-1));
}

#[tokio::test]
async fn raw_output_byte_count_uses_source_bytes_not_rendered_text() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store, parser, bus);
    let result =
        executor.run_short("printf '\\377x\\nno-newline'", None, Some(2_000), None).await.unwrap();

    assert!(result.raw_output.as_deref().unwrap().contains("[invalid UTF-8 replaced]"));
    assert_eq!(result.raw_output_bytes, Some(13));
}

#[tokio::test]
async fn structured_raw_output_byte_count_uses_source_bytes_not_rendered_text() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);
    let result = executor
        .run(
            "printf '\\377x\\nno-newline'",
            None,
            Some(2_000),
            "sync",
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(result.raw_output_bytes, Some(13));
    assert_eq!(store.get_task_raw_output_bytes(&result.task_id), 13);
}

#[tokio::test]
async fn test_executor_sync_mode() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run("echo sync_test", None, None, "sync", None, None, false, None, None)
        .await
        .unwrap();
    // Sync mode should wait for completion and return full result
    assert_eq!(result.status, TaskStatus::Completed);
    assert_eq!(result.exit_code, Some(0));
    assert!(result.duration_ms.is_some());
}

#[tokio::test]
async fn auto_short_merges_stderr_into_raw_output() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    // git-style failure: the diagnostic goes to stderr while stdout stays
    // empty. The zero-overhead short path must still surface it — losing
    // stderr here would leave the agent with only an exit code.
    let cwd = _tmp.path().to_str().unwrap();
    let result = executor
        .run("git status", Some(cwd), None, "auto", None, None, false, None, None)
        .await
        .unwrap();
    assert!(result.short_command, "git status should take the short path");
    assert_eq!(result.status, TaskStatus::Failed);
    let raw = result.raw_output.as_ref().expect("raw_output populated");
    assert!(raw.contains("fatal"), "stderr must be merged into raw_output, got: {raw:?}");
}

#[tokio::test]
async fn test_executor_sync_mode_failure() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result =
        executor.run("exit 42", None, None, "sync", None, None, false, None, None).await.unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert_eq!(result.exit_code, Some(42));
}

#[tokio::test]
async fn rejects_invalid_mode_and_parser_hint() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store, parser, bus);

    let invalid_mode = executor
        .run("echo hi", None, None, "synx", None, None, false, None, None)
        .await
        .unwrap_err();
    assert!(invalid_mode.to_string().contains("invalid execution mode"));

    let invalid_hint = executor
        .run("echo hi", None, None, "auto", Some("cargoo"), None, false, None, None)
        .await
        .unwrap_err();
    assert!(invalid_hint.to_string().contains("unknown parser"));
}

#[tokio::test]
async fn output_limits_truncate_without_deadlocking_the_child() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus)
        .with_config(ExecutorConfig { max_output_bytes: 64, ..Default::default() });

    let structured = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        executor.run(
            "yes noisy-output | head -n 5000",
            None,
            None,
            "sync",
            None,
            None,
            false,
            None,
            None,
        ),
    )
    .await
    .expect("output truncation must not deadlock")
    .unwrap();
    assert_eq!(structured.status, TaskStatus::Completed);
    let raw = executor.tail(&structured.task_id, 0, "raw").await.unwrap().join("\n");
    assert!(raw.len() <= 128, "captured raw output exceeded the bounded limit plus marker");
    assert!(raw.contains("output truncated"));

    let short = executor
        .run(
            "printf 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-extra'",
            None,
            None,
            "auto",
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(short.short_command);
    assert!(short.raw_output.as_deref().unwrap().contains("output truncated"));
    assert_eq!(store.get_task(&short.task_id).unwrap().unwrap().status, TaskStatus::Completed);
    let saved = executor.tail(&short.task_id, 0, "raw").await.unwrap().join("\n");
    assert!(saved.contains("abcdefghijklmnopqrstuvwxyz"));
    assert!(saved.contains("output truncated"));

    let (large_store, large_parser, large_bus, _large_tmp) = setup();
    let large_executor = Executor::new(large_store.clone(), large_parser, large_bus)
        .with_config(ExecutorConfig { max_output_bytes: 32_768, ..Default::default() });
    let large = large_executor
        .run("printf '%020000d' 0", None, None, "auto", None, None, false, None, None)
        .await
        .unwrap();
    assert!(!large.raw_output.as_deref().unwrap().contains("output truncated"));
    assert!(large_store.get_task(&large.task_id).unwrap().is_some());
    let large_saved = large_executor.tail(&large.task_id, 0, "raw").await.unwrap().join("\n");
    assert_eq!(large_saved.len(), 20_000);
}

#[tokio::test]
async fn spawn_failure_never_leaves_a_running_task() {
    let (store, parser, bus, tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);
    let missing = tmp.path().join("does-not-exist");

    let result = executor
        .run("echo unreachable", missing.to_str(), None, "async", None, None, false, None, None)
        .await
        .unwrap();
    for _ in 0..20 {
        if store.get_task(&result.task_id).unwrap().is_some_and(|task| task.status.is_terminal()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let task = store.get_task(&result.task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.exit_code, Some(-1));
}

#[tokio::test]
async fn concurrent_replays_execute_only_once() {
    let (store, parser, bus, tmp) = setup();
    let executor = Arc::new(Executor::new(store, parser, bus));
    let cwd = tmp.path().to_str().unwrap();

    let first = executor.run(
        "printf x >> count.txt",
        Some(cwd),
        None,
        "sync",
        None,
        None,
        false,
        None,
        Some("same-session:42"),
    );
    let second = executor.run(
        "printf x >> count.txt",
        Some(cwd),
        None,
        "sync",
        None,
        None,
        false,
        None,
        Some("same-session:42"),
    );
    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap();
    let second = second.unwrap();

    assert_eq!(first.task_id, second.task_id);
    assert_eq!(std::fs::read_to_string(tmp.path().join("count.txt")).unwrap(), "x");
}

#[tokio::test]
async fn test_executor_kill_graceful() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result =
        executor.run("sleep 60", None, None, "async", None, None, false, None, None).await.unwrap();
    assert_eq!(result.status, TaskStatus::Running);

    // Wait a bit for the process to start
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Kill it
    executor.kill(&result.task_id).await.unwrap();

    // Wait for kill to take effect
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Check task was updated
    let task = store.get_task(&result.task_id).unwrap().unwrap();
    assert!(task.status == TaskStatus::Killed, "expected Killed, got {:?}", task.status);
}

#[tokio::test]
async fn repeated_cancel_does_not_mark_task_terminal_before_process_exit() {
    let (store, parser, bus, _tmp) = setup();
    let config = ExecutorConfig { kill_graceful_ms: 100, kill_force_ms: 100, ..Default::default() };
    let executor = Executor::new(store.clone(), parser, bus).with_config(config);

    let result = executor
        .run("trap '' INT TERM; exec sleep 60", None, None, "async", None, None, false, None, None)
        .await
        .unwrap();
    executor.kill(&result.task_id).await.unwrap();
    executor.kill(&result.task_id).await.unwrap();

    assert_eq!(store.get_task(&result.task_id).unwrap().unwrap().status, TaskStatus::Running);

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if store.get_task(&result.task_id).unwrap().unwrap().status.is_terminal() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancelled process did not reach a terminal state");
    assert_eq!(store.get_task(&result.task_id).unwrap().unwrap().status, TaskStatus::Killed);
}

#[tokio::test]
async fn cancel_rejects_missing_and_terminal_tasks() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store, parser, bus);
    assert!(executor.kill("missing").await.unwrap_err().to_string().contains("not found"));

    let result =
        executor.run("true", None, None, "sync", None, None, false, None, None).await.unwrap();
    let err = executor.kill(&result.task_id).await.unwrap_err();
    assert!(err.to_string().contains("already"));
}

// ── P11: Auto mode tests ──────────────────────────────────────────────────

fn uses_fast_path(command: &str) -> bool {
    is_short_command(command, false)
}

fn parser_uses_structured_path(command: &str) -> bool {
    is_short_command(command, true)
}

/// Fast-path selection edge cases.
#[test]
fn short_command_empty() {
    assert!(uses_fast_path(""));
    assert!(uses_fast_path("   "));
}

#[test]
fn short_command_under_80_chars() {
    assert!(uses_fast_path("ls -la"));
    assert!(uses_fast_path("echo hello world"));
    assert!(uses_fast_path("git status"));
}

#[test]
fn short_command_over_80_chars() {
    // Inspection tools (echo, cat, etc.) bypass the 80-char limit —
    // their raw text is more useful than structured "log" events.
    let long_inspect = "echo this is a really really really really really really really long command that exceeds eighty characters easily";
    assert!(uses_fast_path(long_inspect));
    // Non-inspection commands over 80 chars are still non-short
    let long_build =
        "cargo build --manifest-path /some/really/really/really/long/path/Cargo.toml --release";
    assert!(!uses_fast_path(long_build));
}

#[test]
fn detected_parser_selects_structured_path() {
    // The registry handles path-style detection. Once a parser matches, no
    // tool-specific command list is needed in decision.rs.
    assert!(!parser_uses_structured_path("./node_modules/.bin/tsc --noEmit greet.ts"));
    assert!(!parser_uses_structured_path("/usr/bin/tsc --noEmit greet.ts"));
    assert!(!parser_uses_structured_path("node_modules/.bin/eslint src/index.ts"));
    assert!(!parser_uses_structured_path("uv run --with pytest pytest -q"));
    assert!(!parser_uses_structured_path("python3 -m pytest -q"));
    assert!(!parser_uses_structured_path("a-new-parser-tool"));
}

#[test]
fn short_command_has_pipe() {
    // Simple pipes are now allowed as short commands
    assert!(uses_fast_path("ls -la | grep foo"));
    assert!(uses_fast_path("cat file.txt | head -5"));
    // Multi-pipe text-processing chains with inspection tools → short
    assert!(uses_fast_path("cat file | sort | uniq | head -n 20"));
    // Non-inspection tools with many pipes → still non-short
    assert!(!parser_uses_structured_path("cargo build | grep error | wc -l"));
    assert!(!parser_uses_structured_path("cat Cargo.toml | cargo metadata"));
    assert!(!uses_fast_path("echo payload | sh"));
}

#[test]
fn short_command_has_redirect() {
    assert!(!uses_fast_path("echo hello >> out.txt"));
}

#[test]
fn short_command_has_chaining() {
    assert!(!uses_fast_path("make build && make test"));
    assert!(!uses_fast_path("cd dir || exit 1"));
    assert!(uses_fast_path("echo one; echo two"));
    assert!(uses_fast_path("sed -n '1,2p' file | head"));
    assert!(!uses_fast_path("sed -i.bak 's/a/b/' file"));
}

#[test]
fn short_command_has_background() {
    assert!(!uses_fast_path("npm run dev &"));
}

#[test]
fn short_command_too_many_words() {
    assert!(!uses_fast_path("one two three four five six"));
}

#[test]
fn short_command_long_flag_detected() {
    assert!(!uses_fast_path("cargo watch --watch src/"));
    assert!(!uses_fast_path("tail -f /var/log/system.log"));
    assert!(!uses_fast_path("python -m http.server 8080"));
}

#[test]
fn short_command_daemon_flag() {
    assert!(!uses_fast_path("nginx daemon off"));
}

#[test]
fn parser_assets_replace_hard_coded_build_tool_prefixes() {
    for command in [
        "cargo test",
        "cargo build",
        "cargo clippy",
        "npm test",
        "npm run build",
        "yarn test",
        "go test ./...",
        "go build",
        "make",
        "make test",
        "pytest",
        "pip install requests",
        "docker build .",
    ] {
        assert!(!parser_uses_structured_path(command), "{command}");
    }

    // Read-only inspection remains raw even if a broad parser (for example
    // git) also recognizes the tool.
    assert!(super::decision::is_short_command_with_route("ls", true, Some("fast")));
    assert!(super::decision::is_short_command_with_route("echo hello", true, Some("fast")));
    assert!(super::decision::is_short_command_with_route("git status", true, Some("fast")));
    assert!(super::decision::is_short_command_with_route("pwd", true, Some("fast")));
    assert!(!is_short_command("git commit -m test", true));
    assert!(!is_short_command("git branch new-feature", true));
    assert!(!is_short_command("git tag v1", true));
    assert!(!is_short_command("git config user.name test", true));
}

#[test]
fn quoted_shell_syntax_is_data_not_lifecycle() {
    assert!(uses_fast_path("echo 'a > b; cargo build'"));
    assert!(uses_fast_path("rg 'curl.*\\| sh' src"));
    assert!(uses_fast_path("echo dev"));
    assert!(!uses_fast_path("echo ok > out.txt"));
}

/// Auto mode with a short command returns raw_output + short_command flag.
#[tokio::test]
async fn auto_short_returns_raw_output() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run("echo fast-path", None, None, "auto", None, None, false, None, None)
        .await
        .unwrap();
    assert!(result.short_command, "short_command should be true");
    assert_eq!(result.status, TaskStatus::Completed);
    assert_eq!(result.exit_code, Some(0));
    assert!(result.raw_output.is_some(), "raw_output should be populated");
    assert!(
        result.raw_output.as_ref().unwrap().contains("fast-path"),
        "raw_output should contain the command's stdout"
    );
    assert!(result.duration_ms.is_some());
}

/// Auto mode with a long non-inspection command now uses smart sync.
/// The command completes quickly (invalid path), so it returns a structured result.
#[tokio::test]
async fn auto_long_uses_smart_sync() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run(
            "cargo build --manifest-path /some/really/really/really/long/path/Cargo.toml --release",
            None,
            None,
            "auto",
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(!result.short_command, "long build command should not be short_command");
    assert_eq!(result.status, TaskStatus::Failed, "invalid path should fail");
    assert!(result.exit_code.is_some(), "should have exit code");
    assert!(
        result.primary_diagnostic.is_some(),
        "smart sync attaches a primary diagnostic or project context for failed builds"
    );
}

/// Inspection tools over 80 chars still take the short path — raw text beats log events.
#[tokio::test]
async fn auto_long_inspect_takes_short_path() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor.run(
        "echo this-command-is-definitely-longer-than-eighty-characters-so-it-should-still-use-short-path",
        None, None, "auto", None, None, false, None, None,
    ).await.unwrap();
    assert!(result.short_command, "long echo should still use short path");
    assert_eq!(result.status, TaskStatus::Completed);
    assert!(result.raw_output.is_some());
}

/// Auto mode with a simple piped command: now takes the short path
/// since pipes are allowed in short commands (≤5 words, ≤80 chars).
#[tokio::test]
async fn auto_piped_short_path() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run("echo hello | cat", None, None, "auto", None, None, false, None, None)
        .await
        .unwrap();
    assert!(result.short_command, "simple piped cmd should use short path");
    assert_eq!(result.status, TaskStatus::Completed);
    assert!(result.raw_output.is_some(), "short path populates raw_output");
    assert_eq!(result.raw_output.as_ref().unwrap().trim(), "hello");
}

/// Explicit sync mode with a short command still uses the full structured path.
#[tokio::test]
async fn sync_with_short_uses_full_path() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run("echo sync-short", None, None, "sync", None, None, false, None, None)
        .await
        .unwrap();
    // Sync mode: should complete and return structure, not short path
    assert!(!result.short_command, "explicit sync should use full structured path");
    assert_eq!(result.status, TaskStatus::Completed);
    assert_eq!(result.exit_code, Some(0));
    assert!(result.raw_output.is_none(), "full path should not set raw_output");
}

/// Auto + short + failure: raw_output still populated, status is Failed.
#[tokio::test]
async fn auto_short_failure_has_raw_output() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run("nonexistent_xyz", None, None, "auto", None, None, false, None, None)
        .await
        .unwrap();
    assert!(result.short_command);
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(result.exit_code.unwrap() != 0);
    assert!(result.raw_output.is_some());
}

// ── S2+S4: parse_hint + mode:auto linkage ────────────────────────────────

/// parse_hint="json" bypasses the short path to ensure structured processing.
#[tokio::test]
async fn parse_hint_json_forces_structured_path() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    // Short command with parse_hint="json" → should NOT take short path
    let result = executor
        .run("echo hello", None, None, "auto", Some("json"), None, false, None, None)
        .await
        .unwrap();
    assert!(!result.short_command, "parse_hint should force structured path");
    assert_eq!(result.status, TaskStatus::Completed);

    let task = store.get_task(&result.task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
}

/// parse_hint="json" with JSON output command produces structured events.
#[tokio::test]
async fn parse_hint_json_with_json_output() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run(
            r#"echo '{"status":"ok","count":1}'"#,
            None,
            None,
            "auto",
            Some("json"),
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(!result.short_command);

    wait_for_task_completion(&store, &result.task_id).await;

    // Verify JSON events were stored
    use crate::ipc::QueryParams;
    let params = QueryParams {
        task_id: Some(result.task_id.clone()),
        event_type: None,
        severity: None,
        code: None,
        file: None,
        limit: 100,
        offset: 0,
        include_logs: true,
    };
    let (events, _total) = store.query_events(&params).unwrap();
    // Should have at least one JSON data event
    let has_json_event = events.iter().any(|e| e.event_type == "data");
    assert!(has_json_event, "expected at least one JSON data event");
}

/// parse_hint="raw" also forces structured path (any hint forces it).
#[tokio::test]
async fn parse_hint_raw_forces_structured_path() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    // Short command with parse_hint="raw" → structured path
    let result = executor
        .run("echo hello", None, None, "auto", Some("raw"), None, false, None, None)
        .await
        .unwrap();
    assert!(!result.short_command, "any parse_hint should force structured path");
    assert_eq!(result.status, TaskStatus::Completed);

    let task = store.get_task(&result.task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
}

/// No parse_hint → short commands take the fast path (existing behavior).
#[tokio::test]
async fn no_parse_hint_keeps_short_path() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result =
        executor.run("echo fast", None, None, "auto", None, None, false, None, None).await.unwrap();
    assert!(result.short_command, "no hint should keep short path for short commands");
    assert!(result.raw_output.is_some());
}

/// RunTaskParams deserializes parse_hint correctly.
#[test]
fn runtaskparams_default_parse_hint() {
    let json = r#"{"command":"ls"}"#;
    let params: crate::ipc::RunTaskParams = serde_json::from_str(json).unwrap();
    assert_eq!(params.command, "ls");
    assert_eq!(params.mode, "auto");
    assert!(params.parse_hint.is_none());
}

#[test]
fn runtaskparams_with_parse_hint() {
    let json = r#"{"command":"gh pr list --json","parse_hint":"json"}"#;
    let params: crate::ipc::RunTaskParams = serde_json::from_str(json).unwrap();
    assert_eq!(params.parse_hint.as_deref(), Some("json"));
}

// ── S6: CLI+Skill adaptation tests ───────────────────────────────────────

/// JSON output via echo → JSON parser auto-detects at completion.
#[tokio::test]
async fn cli_json_output_auto_detected() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run(
            r#"printf '{"name":"test","count":42}\n'"#,
            None,
            None,
            "async",
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

    wait_for_task_completion(&store, &result.task_id).await;

    use crate::ipc::QueryParams;
    let params = QueryParams {
        task_id: Some(result.task_id.clone()),
        event_type: None,
        severity: None,
        code: None,
        file: None,
        limit: 100,
        offset: 0,
        include_logs: true,
    };
    let (events, _total) = store.query_events(&params).unwrap();
    let json_events: Vec<_> = events.iter().filter(|e| e.event_type == "data").collect();
    assert!(!json_events.is_empty(), "JSON output should produce data events");
    assert!(
        json_events.iter().any(|e| e.message.contains("name") && e.message.contains("test")),
        "JSON data event should contain the parsed content"
    );
}

/// parse_hint="json" with a JSON array → produces one event per array element.
#[tokio::test]
async fn parse_hint_json_array_produces_structured_events() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run(
            r#"echo '["item-a","item-b","item-c"]'"#,
            None,
            None,
            "auto",
            Some("json"),
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

    wait_for_task_completion(&store, &result.task_id).await;

    use crate::ipc::QueryParams;
    let params = QueryParams {
        task_id: Some(result.task_id.clone()),
        event_type: None,
        severity: None,
        code: None,
        file: None,
        limit: 100,
        offset: 0,
        include_logs: true,
    };
    let (events, _total) = store.query_events(&params).unwrap();
    let json_events: Vec<_> = events.iter().filter(|e| e.event_type == "data").collect();
    assert_eq!(json_events.len(), 3, "JSON array of 3 items should produce 3 data events");
}

/// CLI without dedicated parser → stderr error recognition catches common error patterns.
#[tokio::test]
async fn stderr_recognizes_generic_errors() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    // Write to stderr with common error patterns
    let result = executor.run(
        r#"sh -c 'echo "error: cannot find module" >&2; echo "warning: using fallback" >&2; exit 0'"#,
        None,
        None,
        "async",
        None,
    None,
    false, None, None,
    ).await.unwrap();

    wait_for_task_completion(&store, &result.task_id).await;

    use crate::ipc::QueryParams;
    let params = QueryParams {
        task_id: Some(result.task_id.clone()),
        event_type: None,
        severity: None,
        code: None,
        file: None,
        limit: 100,
        offset: 0,
        include_logs: true,
    };
    let (events, _total) = store.query_events(&params).unwrap();

    let has_error = events.iter().any(|e| e.severity.as_deref() == Some("error"));
    let has_warning = events.iter().any(|e| e.severity.as_deref() == Some("warning"));
    assert!(has_error, "stderr with 'error:' should produce error severity");
    assert!(has_warning, "stderr with 'warning:' should produce warning severity");
}

/// stderr with "Permission denied" is detected as error.
#[tokio::test]
async fn stderr_permission_denied_is_error() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let result = executor
        .run(
            r#"sh -c 'echo "Permission denied (os error 13)" >&2; exit 1'"#,
            None,
            None,
            "async",
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

    wait_for_task_completion(&store, &result.task_id).await;

    use crate::ipc::QueryParams;
    let params = QueryParams {
        task_id: Some(result.task_id.clone()),
        event_type: None,
        severity: None,
        code: None,
        file: None,
        limit: 100,
        offset: 0,
        include_logs: true,
    };
    let (events, _total) = store.query_events(&params).unwrap();
    assert!(
        events.iter().any(|e| e.severity.as_deref() == Some("error")),
        "Permission denied on stderr should be classified as error"
    );
}

/// Short command + parse_hint → structured path with events in store.
#[tokio::test]
async fn short_command_with_parse_hint_stores_events() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    // Short command with parse_hint → forced structured path
    let result = executor
        .run("echo structured", None, None, "auto", Some("raw"), None, false, None, None)
        .await
        .unwrap();

    assert!(!result.short_command);

    wait_for_task_completion(&store, &result.task_id).await;

    let task = store.get_task(&result.task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
    assert!(task.events_count > 0, "structured path should store events");
}

/// Non-JSON output with no parse_hint → no JSON parsing attempted (graceful fallthrough).
#[tokio::test]
async fn non_json_output_no_false_positive() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    // Plain text output
    let result = executor
        .run(
            "printf 'regular output\nmore output\n'",
            None,
            None,
            "async",
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

    wait_for_task_completion(&store, &result.task_id).await;

    use crate::ipc::QueryParams;
    let params = QueryParams {
        task_id: Some(result.task_id.clone()),
        event_type: None,
        severity: None,
        code: None,
        file: None,
        limit: 100,
        offset: 0,
        include_logs: true,
    };
    let (events, _total) = store.query_events(&params).unwrap();
    // All should be "log" type, no "data" (JSON) events
    assert!(
        events.iter().all(|e| e.event_type != "data"),
        "plain text output should not produce JSON data events"
    );
    assert!(
        events.iter().all(|e| e.event_type == "log"),
        "plain text output should produce log events"
    );
}

#[test]
fn select_primary_diagnostic_first_error_for_normal_stream() {
    let events = Some(vec![
        serde_json::json!({"type": "log", "severity": "info", "message": "noise"}),
        serde_json::json!({"type": "diagnostic", "severity": "error", "message": "cannot find type `X`", "seq": 1}),
        serde_json::json!({"type": "location", "severity": "error", "message": "  --> src/main.rs:42", "seq": 2}),
    ]);
    let rc = select_primary_diagnostic(&events).unwrap();
    assert_eq!(rc["message"], "cannot find type `X`");
}

#[test]
fn select_primary_diagnostic_traceback_picks_exception() {
    // A python traceback: the first error is the banner; the real root
    // cause is the final exception line (last diagnostic error).
    let events = Some(vec![
        serde_json::json!({"type": "diagnostic", "severity": "error", "message": "Traceback (most recent call last):", "seq": 1}),
        serde_json::json!({"type": "location", "severity": "error", "message": "  File \"/tmp/x.py\", line 3, in <module>", "seq": 2}),
        serde_json::json!({"type": "log", "severity": "warning", "message": "    f()", "seq": 3}),
        serde_json::json!({"type": "diagnostic", "severity": "error", "message": "ZeroDivisionError: division by zero", "seq": 4}),
    ]);
    let rc = select_primary_diagnostic(&events).unwrap();
    assert_eq!(rc["message"], "ZeroDivisionError: division by zero");
    assert_eq!(rc["seq"], 4);
}

// ── Replay protection & concurrency limit ─────────────────────────────

/// The same dedup_key (a proxy replay after connection blip) must return the
/// same task instead of executing the command twice.
#[tokio::test]
async fn run_dedup_key_reuses_same_task() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus);

    let r1 = executor
        .run("echo dedup", None, None, "async", None, None, false, None, Some("req-1"))
        .await
        .unwrap();
    let r2 = executor
        .run("echo dedup", None, None, "async", None, None, false, None, Some("req-1"))
        .await
        .unwrap();

    assert_eq!(
        r1.task_id, r2.task_id,
        "a replayed request with the same dedup_key must reuse the original task"
    );

    // Fill the bounded cache. A new replay key still has to be cached after
    // evicting an old result, or concurrent/retried calls can execute twice.
    {
        let mut cache = executor.run_dedup.lock().await;
        for index in 0..RUN_DEDUP_CACHE_MAX {
            cache_run_result(&mut cache, format!("seed-{index}"), r1.clone(), Instant::now());
        }
        assert_eq!(cache.len(), RUN_DEDUP_CACHE_MAX);
    }

    // A different key is a different command invocation.
    let r3 = executor
        .run("echo dedup", None, None, "async", None, None, false, None, Some("req-2"))
        .await
        .unwrap();
    assert_ne!(r1.task_id, r3.task_id);
    let r4 = executor
        .run("echo dedup", None, None, "async", None, None, false, None, Some("req-2"))
        .await
        .unwrap();
    assert_eq!(r3.task_id, r4.task_id, "a full cache must evict old results and retain new ones");

    // Reusing an MCP request id for a different invocation must not replay a
    // stale result from the earlier call.
    let r5 = executor
        .run("echo different", None, None, "async", None, None, false, None, Some("req-1"))
        .await
        .unwrap();
    assert_ne!(r1.task_id, r5.task_id);

    let mut env_forward = HashMap::new();
    env_forward.insert("FIRST".into(), "1".into());
    env_forward.insert("SECOND".into(), "2".into());
    let mut env_reverse = HashMap::new();
    env_reverse.insert("SECOND".into(), "2".into());
    env_reverse.insert("FIRST".into(), "1".into());
    let r6 = executor
        .run(
            "echo env",
            None,
            None,
            "async",
            None,
            Some(&env_forward),
            false,
            None,
            Some("req-env"),
        )
        .await
        .unwrap();
    let r7 = executor
        .run(
            "echo env",
            None,
            None,
            "async",
            None,
            Some(&env_reverse),
            false,
            None,
            Some("req-env"),
        )
        .await
        .unwrap();
    assert_eq!(r6.task_id, r7.task_id, "environment map order must not affect replay identity");
}

/// `max_concurrent_tasks` must actually bound structured tasks (previously
/// it was a config field with no enforcement).
#[tokio::test]
async fn max_concurrent_tasks_rejects_overflow() {
    let (store, parser, bus, _tmp) = setup();
    let executor = Executor::new(store.clone(), parser, bus)
        .with_config(ExecutorConfig { max_concurrent_tasks: 1, ..Default::default() });

    let r1 =
        executor.run("sleep 5", None, None, "async", None, None, false, None, None).await.unwrap();
    assert_eq!(r1.status, TaskStatus::Running);

    let err = executor
        .run("echo overflow", None, None, "async", None, None, false, None, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("too many concurrent tasks"),
        "overflow must be rejected, got: {}",
        err
    );
}
