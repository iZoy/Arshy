//! Background watchdog that periodically runs dogfood.sh and stores results.

use crate::store::dogfood::DogfoodRun;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::process::Command;

pub struct DogfoodWatchdog {
    pub interval_minutes: u32,
    store: Arc<crate::store::Store>,
}

impl DogfoodWatchdog {
    pub fn new(interval_minutes: u32, store: Arc<crate::store::Store>) -> Self {
        Self { interval_minutes, store }
    }

    /// Find dogfood.sh by searching known locations relative to the binary.
    fn find_dogfood_script() -> Option<PathBuf> {
        let bin_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))?;

        let candidates = [
            bin_dir.join("../scripts/dogfood.sh"),
            bin_dir.join("../../scripts/dogfood.sh"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/dogfood.sh"),
            dirs::home_dir()
                .unwrap_or_default()
                .join("Documents/arshy/scripts/dogfood.sh"),
        ];

        candidates
            .iter()
            .find(|p| p.exists())
            .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
    }

    /// Spawn the background watchdog loop.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Initial delay — let the daemon finish starting up
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;

            let script = match Self::find_dogfood_script() {
                Some(s) => s,
                None => {
                    tracing::warn!("dogfood: dogfood.sh not found, watchdog disabled");
                    return;
                }
            };
            tracing::info!(
                "dogfood watchdog: running every {} min, script={}",
                self.interval_minutes,
                script.display()
            );

            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                self.interval_minutes as u64 * 60,
            ));
            // First tick fires immediately; skip it since we already waited 60s
            interval.tick().await;

            loop {
                interval.tick().await;
                match Self::run_dogfood(&script).await {
                    Ok(result) => {
                        tracing::info!(
                            "dogfood: {}/{} passed, {} failed",
                            result.pass,
                            result.total,
                            result.fail
                        );
                        if let Err(e) = self.store.insert_dogfood_run(&result) {
                            tracing::warn!("dogfood: failed to store result: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("dogfood: run failed: {}", e);
                    }
                }
            }
        })
    }

    /// Run dogfood.sh and parse the summary line.
    async fn run_dogfood(script: &PathBuf) -> arshy_lib::Result<DogfoodRun> {
        let output = Command::new("bash")
            .arg(script)
            .arg("--json")
            .current_dir(
                script
                    .parent()
                    .and_then(|p| p.parent())
                    .unwrap_or(std::path::Path::new(".")),
            )
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}\n{}", stdout, stderr);

        // Try JSON output first (--json flag)
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
            let pass = json["pass"].as_u64().unwrap_or(0) as u32;
            let fail = json["fail"].as_u64().unwrap_or(0) as u32;
            let total = json["total"].as_u64().unwrap_or(0) as u32;
            return Ok(DogfoodRun {
                timestamp: chrono::Utc::now().to_rfc3339(),
                pass,
                fail,
                total,
                log_excerpt: if fail > 0 {
                    Some(combined.lines().take(50).collect::<Vec<_>>().join("\n"))
                } else {
                    None
                },
            });
        }

        // Fallback: parse "Results: X/Y passed, Z failed" line
        let (pass, fail, total) = parse_results_line(&combined);

        Ok(DogfoodRun {
            timestamp: chrono::Utc::now().to_rfc3339(),
            pass,
            fail,
            total,
            log_excerpt: if fail > 0 {
                Some(combined.lines().take(50).collect::<Vec<_>>().join("\n"))
            } else {
                None
            },
        })
    }
}

/// Parse the summary line from dogfood.sh output.
/// Expected format: "=== Results: X/Y passed, Z failed ==="
fn parse_results_line(output: &str) -> (u32, u32, u32) {
    for line in output.lines() {
        if line.contains("Results:") {
            // Extract "X/Y passed"
            if let Some(passed_part) = line.split("Results:").nth(1) {
                let parts: Vec<&str> = passed_part.split_whitespace().collect();
                if let Some(fraction) = parts.first() {
                    let nums: Vec<&str> = fraction.split('/').collect();
                    if nums.len() == 2 {
                        let pass: u32 = nums[0].parse().unwrap_or(0);
                        let total: u32 = nums[1].parse().unwrap_or(0);
                        let fail = total.saturating_sub(pass);
                        return (pass, fail, total);
                    }
                }
            }
        }
    }
    (0, 0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_results_normal() {
        let output = "some stuff\n=== Results: 21/21 passed, 0 failed ===\nAll good!";
        let (pass, fail, total) = parse_results_line(output);
        assert_eq!(pass, 21);
        assert_eq!(fail, 0);
        assert_eq!(total, 21);
    }

    #[test]
    fn parse_results_with_failures() {
        let output = "=== Results: 18/21 passed, 3 failed ===";
        let (pass, fail, total) = parse_results_line(output);
        assert_eq!(pass, 18);
        assert_eq!(fail, 3);
        assert_eq!(total, 21);
    }

    #[test]
    fn parse_results_empty() {
        let (pass, fail, total) = parse_results_line("no results here");
        assert_eq!(pass, 0);
        assert_eq!(fail, 0);
        assert_eq!(total, 0);
    }
}
