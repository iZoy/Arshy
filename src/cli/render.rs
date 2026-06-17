//! Pretty terminal renderer for arshy command results.
//!
//! Renders structured build/test output as a visual terminal UI with
//! box-drawing characters, color coding, and source context display.

use std::fmt::Write;

// ── ANSI colors ──────────────────────────────────────────────────────────────

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const WHITE: &str = "\x1b[37m";
const BG_GREEN: &str = "\x1b[42m";

// ── Box drawing ──────────────────────────────────────────────────────────────

const TL: &str = "╭";
const TR: &str = "╮";
const BL: &str = "╰";
const BR: &str = "╯";
const H: &str = "─";
const V: &str = "│";

const BOX_W: usize = 60;
const INNER_W: usize = BOX_W - 4; // subtract "│ " and " │"

// ── Public API ───────────────────────────────────────────────────────────────

/// Render a pretty terminal UI from a JSON response.
/// Returns `true` if the response was rendered, `false` if it fell back to JSON.
pub fn render(result: &serde_json::Value) {
    let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
    let is_short = result.get("short_command").and_then(|v| v.as_bool()).unwrap_or(false);

    // Short commands: show raw output with minimal decoration
    if is_short {
        render_short(result);
        return;
    }

    // Long commands: render structured UI
    match status {
        "completed" => render_completed(result),
        "failed" => render_failed(result),
        "timeout" => render_failed(result),
        "killed" => render_killed(result),
        "running" => render_running(result),
        _ => render_unknown(result),
    }
}

// ── Short command renderer ───────────────────────────────────────────────────

fn render_short(result: &serde_json::Value) {
    let exit_code = result.get("exit_code").and_then(|v| v.as_i64());
    let duration = result.get("duration_ms").and_then(|v| v.as_u64());
    let raw = result.get("raw_output").and_then(|v| v.as_str()).unwrap_or("");

    let success = exit_code == Some(0);
    let icon = if success { "✓" } else { "✗" };
    let color = if success { GREEN } else { RED };

    eprint!("  {}{}{}{}", BOLD, color, icon, RESET);

    if let Some(code) = exit_code {
        if !success {
            eprint!("  {}exit {}{}", DIM, code, RESET);
        }
    }
    if let Some(ms) = duration {
        eprint!("  {}{}ms{}", DIM, ms, RESET);
    }
    eprintln!();

    if !raw.is_empty() {
        for line in raw.lines() {
            eprintln!("  {}{}{}", DIM, line, RESET);
        }
    }
}

// ── Completed (success) ─────────────────────────────────────────────────────

fn render_completed(result: &serde_json::Value) {
    let duration = result.get("duration_ms").and_then(|v| v.as_u64());
    let exit_code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
    let events = result.get("events").and_then(|v| v.as_array());

    let summary = result.get("summary");
    let error_count = summary
        .and_then(|s| s.get("by_severity"))
        .and_then(|s| s.get("error"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let warning_count = summary
        .and_then(|s| s.get("by_severity"))
        .and_then(|s| s.get("warning"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let success = exit_code == 0 && error_count == 0;

    eprintln!();
    box_top();

    // Status line
    if success {
        box_line(&format!(
            "{}{}{}✓ Passed{}   {}",
            BOLD,
            GREEN,
            BG_GREEN,
            RESET,
            duration_str(duration)
        ));
    } else {
        box_line(&format!(
            "{}{}✗ Failed{}   {} errors, {} warnings   {}",
            BOLD,
            RED,
            RESET,
            error_count,
            warning_count,
            duration_str(duration)
        ));
    }

    box_divider();

    // Show events
    if let Some(evts) = events {
        let diagnostic_events: Vec<_> = evts
            .iter()
            .filter(|e| {
                let t = e.get("type").and_then(|v| v.as_str()).unwrap_or("");
                t == "diagnostic" || t == "test_result"
            })
            .collect();

        if diagnostic_events.is_empty() {
            box_line(&format!("{}No diagnostic events{}", DIM, RESET));
        } else {
            for event in &diagnostic_events {
                render_event_in_box(event);
            }
        }

        // Show summary events
        let summary_events: Vec<_> = evts
            .iter()
            .filter(|e| e.get("type").and_then(|v| v.as_str()) == Some("summary"))
            .collect();

        if !summary_events.is_empty() {
            box_divider();
            for event in &summary_events {
                let msg = event.get("message").and_then(|v| v.as_str()).unwrap_or("");
                if !msg.is_empty() {
                    box_line(&format!("{}{}{}", DIM, msg, RESET));
                }
            }
        }
    } else {
        box_line(&format!("{}Command completed successfully{}", DIM, RESET));
    }

    box_bottom();
    eprintln!();
}

// ── Failed ───────────────────────────────────────────────────────────────────

fn render_failed(result: &serde_json::Value) {
    let duration = result.get("duration_ms").and_then(|v| v.as_u64());
    let exit_code = result.get("exit_code").and_then(|v| v.as_i64());
    let events = result.get("events").and_then(|v| v.as_array());

    let summary = result.get("summary");
    let error_count = summary
        .and_then(|s| s.get("by_severity"))
        .and_then(|s| s.get("error"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let warning_count = summary
        .and_then(|s| s.get("by_severity"))
        .and_then(|s| s.get("warning"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    eprintln!();
    box_top();

    // Status line
    let exit_str = exit_code.map(|c| format!("  exit {}", c)).unwrap_or_default();
    box_line(&format!(
        "{}{}✗ Failed{}   {} errors, {} warnings{}   {}",
        BOLD,
        RED,
        RESET,
        error_count,
        warning_count,
        exit_str,
        duration_str(duration)
    ));

    // Root cause
    if let Some(root_cause) = result.get("root_cause") {
        box_divider();
        let msg = root_cause.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let sev = root_cause.get("severity").and_then(|v| v.as_str()).unwrap_or("error");
        let sev_color = severity_color(sev);
        box_line(&format!(
            "{}Root cause: {}{}{}",
            DIM,
            sev_color,
            truncate(msg, INNER_W - 13),
            RESET
        ));
    }

    box_divider();

    // Show error/warning events
    if let Some(evts) = events {
        let diagnostic_events: Vec<_> = evts
            .iter()
            .filter(|e| {
                let sev = e.get("severity").and_then(|v| v.as_str()).unwrap_or("");
                sev == "error" || sev == "warning"
            })
            .collect();

        if diagnostic_events.is_empty() {
            box_line(&format!("{}No diagnostic events{}", DIM, RESET));
        } else {
            for event in &diagnostic_events {
                render_event_in_box(event);
            }
        }
    } else {
        box_line(&format!("{}No structured events available{}", DIM, RESET));
    }

    box_bottom();
    eprintln!();
}

// ── Killed / Running / Unknown ───────────────────────────────────────────────

fn render_killed(result: &serde_json::Value) {
    let duration = result.get("duration_ms").and_then(|v| v.as_u64());
    eprintln!();
    box_top();
    box_line(&format!("{}{}⚠ Killed{}   {}", BOLD, YELLOW, RESET, duration_str(duration)));
    box_line(&format!("{}Task was terminated by signal{}", DIM, RESET));
    box_bottom();
    eprintln!();
}

fn render_running(result: &serde_json::Value) {
    let task_id = result.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
    eprintln!();
    box_top();
    box_line(&format!("{}{}◌ Running{}   {}", BOLD, CYAN, RESET, task_id));
    box_line(&format!("{}Use arshy tail --task-id {} to see output{}", DIM, task_id, RESET));
    box_bottom();
    eprintln!();
}

fn render_unknown(result: &serde_json::Value) {
    eprintln!("{}", serde_json::to_string_pretty(result).unwrap_or_default());
}

// ── Event rendering ──────────────────────────────────────────────────────────

fn render_event_in_box(event: &serde_json::Value) {
    let sev = event.get("severity").and_then(|v| v.as_str()).unwrap_or("info");
    let msg = event.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let code = event.get("code").and_then(|v| v.as_str());
    let location = event.get("location");
    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

    let sev_color = severity_color(sev);
    let sev_icon = severity_icon(sev);
    let type_label = event_type_label(event_type);

    // Line 1: icon + code + type label
    let mut header = format!("{}{} {}{}", sev_color, sev_icon, type_label, RESET);
    if let Some(c) = code {
        let _ = write!(header, " {}{}{}", DIM, c, RESET);
    }
    box_line(&header);

    // Line 2: message (wrapped if needed)
    if !msg.is_empty() {
        let wrapped = wrap_text(msg, INNER_W - 2);
        for line in wrapped {
            box_line(&format!("  {}{}{}", WHITE, line, RESET));
        }
    }

    // Line 3: file location
    if let Some(loc) = location {
        let file = loc.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let line_no = loc.get("line").and_then(|v| v.as_u64());
        let col = loc.get("column").and_then(|v| v.as_u64());

        if !file.is_empty() {
            let loc_str = match (line_no, col) {
                (Some(l), Some(c)) => format!("{}:{}:{}", file, l, c),
                (Some(l), None) => format!("{}:{}", file, l),
                _ => file.to_string(),
            };
            box_line(&format!("  {}{}{}{}", CYAN, DIM, loc_str, RESET));
        }
    }

    // Blank separator
    box_line("");
}

// ── Box drawing helpers ──────────────────────────────────────────────────────

fn box_top() {
    eprintln!("  {}{}{}", TL, H.repeat(BOX_W - 2), TR);
}

fn box_bottom() {
    eprintln!("  {}{}{}", BL, H.repeat(BOX_W - 2), BR);
}

fn box_divider() {
    eprintln!("  ├{}┤", H.repeat(BOX_W - 2));
}

fn box_line(content: &str) {
    // Strip ANSI codes to measure visible width
    let visible_len = visible_width(content);
    let padding = INNER_W.saturating_sub(visible_len);
    eprintln!("  {} {}{}{}", V, content, " ".repeat(padding), V);
}

// ── Text helpers ─────────────────────────────────────────────────────────────

fn duration_str(ms: Option<u64>) -> String {
    match ms {
        Some(m) if m < 1000 => format!("{}ms", m),
        Some(m) => format!("{:.1}s", m as f64 / 1000.0),
        None => String::new(),
    }
}

fn severity_color(sev: &str) -> &'static str {
    match sev {
        "error" => RED,
        "warning" => YELLOW,
        "info" => CYAN,
        _ => WHITE,
    }
}

fn severity_icon(sev: &str) -> &'static str {
    match sev {
        "error" => "✗",
        "warning" => "⚠",
        "info" => "●",
        _ => "·",
    }
}

fn event_type_label(event_type: &str) -> &'static str {
    match event_type {
        "diagnostic" => "Diagnostic",
        "test_result" => "Test",
        "location" => "Location",
        "summary" => "Summary",
        "crash" => "Crash",
        "log" => "Log",
        _ => "Event",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let end = s.char_indices().nth(max.saturating_sub(3)).map(|(i, _)| i).unwrap_or(s.len());
    format!("{}...", &s[..end])
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if text.len() <= width {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        if current.is_empty() {
            current = word.to_string();
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Calculate visible width of a string, ignoring ANSI escape sequences.
/// CJK/fullwidth characters count as 2 columns.
fn visible_width(s: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false;
    for ch in s.chars() {
        if ch == '\x1b' {
            in_escape = true;
            continue;
        }
        if in_escape {
            if ch == 'm' {
                in_escape = false;
            }
            continue;
        }
        width += if unicode_width(ch) { 2 } else { 1 };
    }
    width
}

fn unicode_width(c: char) -> bool {
    let cp = c as u32;
    // CJK Unified Ideographs + common fullwidth ranges
    (0x4E00..=0x9FFF).contains(&cp)   // CJK
        || (0x3000..=0x303F).contains(&cp)  // CJK symbols
        || (0xFF00..=0xFFEF).contains(&cp)  // fullwidth
        || (0x1F300..=0x1F9FF).contains(&cp) // emoji
        || (0x2E80..=0x2FDF).contains(&cp)  // CJK radicals
        || (0x3400..=0x4DBF).contains(&cp)  // CJK Extension A
        || (0x20000..=0x2A6DF).contains(&cp) // CJK Extension B
}

// ── Benchmark renderer ─────────────────────────────────────────────────────

/// Render stats as a formatted terminal box.
pub fn render_stats(result: &serde_json::Value) {
    let total_tasks = result.get("total_tasks").and_then(|v| v.as_u64()).unwrap_or(0);
    let total_events = result.get("total_events").and_then(|v| v.as_u64()).unwrap_or(0);
    let total_errors = result.get("total_errors").and_then(|v| v.as_u64()).unwrap_or(0);
    let failure_rate = result.get("failure_rate").and_then(|v| v.as_f64()).map(|r| r * 100.0);
    let avg_duration = result.get("avg_duration_ms").and_then(|v| v.as_f64());
    let db_size = result.get("db_size_bytes").and_then(|v| v.as_u64());

    // by_status
    let by_status = result.get("by_status");
    let running = by_status.and_then(|s| s.get("running")).and_then(|v| v.as_u64()).unwrap_or(0);
    let completed =
        by_status.and_then(|s| s.get("completed")).and_then(|v| v.as_u64()).unwrap_or(0);
    let failed = by_status.and_then(|s| s.get("failed")).and_then(|v| v.as_u64()).unwrap_or(0);
    let killed = by_status.and_then(|s| s.get("killed")).and_then(|v| v.as_u64()).unwrap_or(0);
    let timeout = by_status.and_then(|s| s.get("timeout")).and_then(|v| v.as_u64()).unwrap_or(0);

    // Intelligence metrics
    let parser_coverage_pct = result.get("parser_coverage_pct").and_then(|v| v.as_f64());
    let hints_attached = result.get("hints_attached").and_then(|v| v.as_u64());
    let context_enriched = result.get("context_enriched").and_then(|v| v.as_u64());
    let dedup_collapsed = result.get("dedup_collapsed").and_then(|v| v.as_u64());
    let correlated_errors = result.get("correlated_errors").and_then(|v| v.as_u64());
    let per_parser_usage = result.get("per_parser_usage").and_then(|v| v.as_array());

    eprintln!();
    box_top();
    box_line(&format!("{}{}ARSHY DAEMON STATS{}", BOLD, CYAN, RESET));
    box_divider();

    box_line(&format!(
        "Tasks:   {} total ({} running, {} done, {} failed, {} killed, {} timeout)",
        total_tasks, running, completed, failed, killed, timeout
    ));
    box_line(&format!("Events:  {} total ({} errors)", total_events, total_errors));

    if let Some(rate) = failure_rate {
        box_line(&format!("Failure rate:  {:.0}%", rate));
    }
    if let Some(ms) = avg_duration {
        box_line(&format!("Avg duration:  {:.0}ms", ms));
    }
    if let Some(bytes) = db_size {
        box_line(&format!("DB size:       {} bytes", bytes));
    }

    // Intelligence section
    let has_intelligence = parser_coverage_pct.is_some()
        || hints_attached.is_some()
        || context_enriched.is_some()
        || dedup_collapsed.is_some()
        || correlated_errors.is_some()
        || per_parser_usage.is_some();
    if has_intelligence {
        box_divider();
        box_line(&format!("{}Intelligence{}", BOLD, RESET));
        if let Some(cov) = parser_coverage_pct {
            box_line(&format!("  Parser coverage: {:.0}% events structured (not raw log)", cov));
        }
        if let Some(hints) = hints_attached {
            box_line(&format!("  Hints attached:  {} error events with fix suggestions", hints));
        }
        if let Some(ctx) = context_enriched {
            box_line(&format!("  Context enriched: {} events with source code", ctx));
        }
        if let Some(dedup) = dedup_collapsed {
            if dedup > 0 {
                box_line(&format!("  Dedup saved:     {} duplicate lines suppressed", dedup));
            }
        }
        if let Some(corr) = correlated_errors {
            if corr > 0 {
                box_line(&format!("  Git correlation:  {} errors linked to recent changes", corr));
            }
        }
        if let Some(parsers) = per_parser_usage {
            if !parsers.is_empty() {
                let top: Vec<String> = parsers
                    .iter()
                    .take(5)
                    .filter_map(|p| {
                        let name = p.get("parser")?.as_str()?;
                        let count = p.get("count")?.as_u64()?;
                        Some(format!("{}({})", name, count))
                    })
                    .collect();
                if !top.is_empty() {
                    box_line(&format!("  Top parsers:     {}", top.join(", ")));
                }
            }
        }
    }

    box_bottom();
    eprintln!();
}

/// Render benchmark results as a formatted terminal table.
pub fn render_benchmark(result: &serde_json::Value) {
    let fixtures = result.get("total_fixtures").and_then(|v| v.as_u64()).unwrap_or(0);
    let raw_lines = result.get("total_raw_lines").and_then(|v| v.as_u64()).unwrap_or(0);
    let events = result.get("total_events").and_then(|v| v.as_u64()).unwrap_or(0);
    let raw_tokens = result.get("total_raw_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let struct_tokens = result.get("total_structured_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let ratio = result.get("compression_ratio").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let fields = result.get("total_structured_fields").and_then(|v| v.as_u64()).unwrap_or(0);
    let fields_per = result.get("avg_fields_per_event").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let speed = result.get("error_speed_advantage_pct").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let accuracy = result.get("avg_accuracy").and_then(|v| v.as_f64()).unwrap_or(0.0) * 100.0;

    eprintln!();
    eprintln!("╔═══════════════════════════════════════════════════════════════╗");
    eprintln!("║               ARSHY PARSER BENCHMARK RESULTS                 ║");
    eprintln!("╚═══════════════════════════════════════════════════════════════╝");
    eprintln!();
    eprintln!("  Scope: {} fixtures across 37 builtin parsers", fixtures);
    eprintln!("         {} raw input lines → {} structured events", raw_lines, events);
    eprintln!();

    // Information density
    eprintln!("  ┌─────────────────────────────────────────────────────────┐");
    eprintln!("  │ INFORMATION DENSITY                                     │");
    eprintln!("  ├─────────────────────────────────────────────────────────┤");
    eprintln!("  │                                                         │");
    eprintln!("  │  Structured output:  {:.1} actionable fields/event       │", fields_per);
    eprintln!("  │                       (type, severity, code,            │");
    eprintln!("  │                        file, line, message)             │");
    eprintln!("  │                                                         │");
    eprintln!("  │  Raw text output:    0 structured fields/line           │");
    eprintln!("  │                       (agent must parse everything)     │");
    eprintln!("  │                                                         │");
    eprintln!("  │  Total fields:       {} across {} events               │", fields, events);
    eprintln!("  │                                                         │");
    eprintln!("  └─────────────────────────────────────────────────────────┘");
    eprintln!();

    // Token efficiency
    eprintln!("  ┌─────────────────────────────────────────────────────────┐");
    eprintln!("  │ TOKEN EFFICIENCY                                        │");
    eprintln!("  ├─────────────────────────────────────────────────────────┤");
    eprintln!("  │  Raw text:           {} words                          │", raw_tokens);
    eprintln!("  │  Structured JSON:    {} words                          │", struct_tokens);
    eprintln!("  │  Ratio:              {:.1}x                             │", ratio);
    eprintln!("  └─────────────────────────────────────────────────────────┘");
    eprintln!();

    // Error location speed
    eprintln!("  ┌─────────────────────────────────────────────────────────┐");
    eprintln!("  │ ERROR LOCATION SPEED                                    │");
    eprintln!("  ├─────────────────────────────────────────────────────────┤");
    eprintln!("  │  Structured faster:  {:.0}% of fixtures                  │", speed);
    eprintln!("  └─────────────────────────────────────────────────────────┘");
    eprintln!();

    // Parser accuracy
    eprintln!("  ┌─────────────────────────────────────────────────────────┐");
    eprintln!("  │ PARSER ACCURACY                                         │");
    eprintln!("  ├─────────────────────────────────────────────────────────┤");
    eprintln!("  │  Average accuracy:   {:.0}%                             │", accuracy);
    eprintln!("  └─────────────────────────────────────────────────────────┘");
    eprintln!();

    // Per-parser table
    if let Some(details) = result.get("details").and_then(|v| v.as_array()) {
        eprintln!("  Per-parser breakdown:");
        eprintln!("  ┌──────────────┬───────┬────────┬────────┬──────────┬───────────┐");
        eprintln!("  │ Parser       │ Lines │ Events │ Fields │ Compress │ Accuracy  │");
        eprintln!("  ├──────────────┼───────┼────────┼────────┼──────────┼───────────┤");

        for d in details {
            let parser = d.get("parser").and_then(|v| v.as_str()).unwrap_or("");
            let lines = d.get("raw_lines").and_then(|v| v.as_u64()).unwrap_or(0);
            let evts = d.get("events").and_then(|v| v.as_u64()).unwrap_or(0);
            let flds = d.get("structured_fields").and_then(|v| v.as_u64()).unwrap_or(0);
            let comp = d.get("compression_ratio").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let acc = d.get("accuracy").and_then(|v| v.as_f64()).unwrap_or(0.0) * 100.0;
            let p_name = if parser.len() > 12 {
                let end = parser.char_indices().nth(12).map(|(i, _)| i).unwrap_or(parser.len());
                &parser[..end]
            } else {
                parser
            };
            eprintln!(
                "  │ {:<12} │ {:>5} │ {:>6} │ {:>6} │ {:>6.1}x  │ {:>6.0}%   │",
                p_name, lines, evts, flds, comp, acc
            );
        }

        eprintln!("  └──────────────┴───────┴────────┴────────┴──────────┴───────────┘");
    }
    eprintln!();
}
