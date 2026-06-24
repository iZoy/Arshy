//! Pretty terminal renderer for arshy command results.
//!
//! Renders structured build/test output as a visual terminal UI with
//! box-drawing characters, color coding, and source context display.

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
    let error_count = result.get("error_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let warning_count = result.get("warning_count").and_then(|v| v.as_u64()).unwrap_or(0);

    let success = exit_code == 0 && error_count == 0;

    eprintln!();
    box_top();

    // Status line — always show error/warning counts and duration
    if success {
        box_line(&format!(
            "{}{}{}{} Completed{}   {} error{}, {} warning{}, {}",
            BOLD,
            GREEN,
            BG_GREEN,
            "\u{2713}", // checkmark
            RESET,
            error_count,
            if error_count == 1 { "" } else { "s" },
            warning_count,
            if warning_count == 1 { "" } else { "s" },
            duration_str(duration)
        ));
    } else {
        box_line(&format!(
            "{}{}✗ Failed{}   {} error{}, {} warning{}, {}",
            BOLD,
            RED,
            RESET,
            error_count,
            if error_count == 1 { "" } else { "s" },
            warning_count,
            if warning_count == 1 { "" } else { "s" },
            duration_str(duration)
        ));
    }

    box_divider();

    if let Some(project_context) = result.get("project_context") {
        if let Some(git_diff) = project_context.get("git_diff_stat").and_then(|v| v.as_str()) {
            if !git_diff.is_empty() {
                box_divider();
                box_line(&format!("{}Recent changes{}", DIM, RESET));
                box_line(&truncate(git_diff, INNER_W - 2));
            }
        }

        if let Some(correlated) =
            project_context.get("correlated_errors").and_then(|v| v.as_array())
        {
            if !correlated.is_empty() {
                box_divider();
                for item in correlated.iter().take(5) {
                    let file = item.get("file").and_then(|v| v.as_str()).unwrap_or("");
                    let recently =
                        item.get("recently_changed").and_then(|v| v.as_bool()).unwrap_or(false);
                    let marker = if recently { "!" } else { " " };
                    box_line(&format!(
                        "  {}{}{} {}",
                        marker,
                        file,
                        RESET,
                        if recently { "(recently changed)" } else { "" }
                    ));
                }
            }
        }
    }

    if let Some(root_cause) = result.get("root_cause") {
        box_divider();
        let msg = root_cause.get("message").and_then(|v| v.as_str()).unwrap_or("");
        if !msg.is_empty() {
            box_line(&format!("{}{}{}", DIM, msg, RESET));
        }
    }

    if success {
        box_line(&format!("{}Command completed successfully{}", DIM, RESET));
    }

    box_bottom();
    eprintln!();
}

// ── Failed ───────────────────────────────────────────────────────────────────

fn render_failed(result: &serde_json::Value) {
    let duration = result.get("duration_ms").and_then(|v| v.as_u64());
    let exit_code = result.get("exit_code").and_then(|v| v.as_i64());
    let error_count = result.get("error_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let warning_count = result.get("warning_count").and_then(|v| v.as_u64()).unwrap_or(0);

    eprintln!();
    box_top();

    // Status line — show error/warning counts, exit code, and duration
    let exit_str = exit_code.map(|c| format!(" (exit {})", c)).unwrap_or_default();
    box_line(&format!(
        "{}{}✗ Failed{}   {} error{}, {} warning{}   {}{}",
        BOLD,
        RED,
        RESET,
        error_count,
        if error_count == 1 { "" } else { "s" },
        warning_count,
        if warning_count == 1 { "" } else { "s" },
        duration_str(duration),
        exit_str,
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

        // File location (if available from root cause)
        if let Some(file) = root_cause.get("file").and_then(|v| v.as_str()) {
            let line = root_cause.get("line").and_then(|v| v.as_u64());
            let loc = match line {
                Some(l) => format!("{}:{}", file, l),
                None => file.to_string(),
            };
            box_line(&format!("  {}Location: {}{}", DIM, loc, RESET));
        }
    }

    // Project context
    if let Some(project_context) = result.get("project_context") {
        if let Some(git_diff) = project_context.get("git_diff_stat").and_then(|v| v.as_str()) {
            if !git_diff.is_empty() {
                box_divider();
                box_line(&format!("{}Recent changes{}", DIM, RESET));
                box_line(&truncate(git_diff, INNER_W - 2));
            }
        }

        if let Some(correlated) =
            project_context.get("correlated_errors").and_then(|v| v.as_array())
        {
            if !correlated.is_empty() {
                box_divider();
                let changed: Vec<_> = correlated
                    .iter()
                    .filter_map(|item| {
                        let file = item.get("file").and_then(|v| v.as_str())?;
                        let recently = item.get("recently_changed").and_then(|v| v.as_bool())?;
                        Some((file, recently))
                    })
                    .collect();

                if !changed.is_empty() {
                    box_line(&format!("{}Error correlation{}", DIM, RESET));
                    for (file, recently_changed) in changed.iter().take(5) {
                        let marker = if *recently_changed { "!" } else { " " };
                        box_line(&format!(
                            "  {}{}{} {}",
                            marker,
                            file,
                            RESET,
                            if *recently_changed { "(recently changed)" } else { "" }
                        ));
                    }
                }
            }
        }
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

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let end = s.char_indices().nth(max.saturating_sub(3)).map(|(i, _)| i).unwrap_or(s.len());
    format!("{}...", &s[..end])
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

// ── Analyze renderer ──────────────────────────────────────────────────────

/// Format a number with commas (e.g. 12276 -> "12,276").
fn fmt_commas(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

/// Truncate a command name to `width` chars, adding "..." if needed.
fn truncate_name(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let end = s.char_indices().nth(width.saturating_sub(3)).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

/// Render a horizontal bar, scaled so that `max` fills `max_bar` columns.
fn render_bar(count: u64, max: u64, max_bar: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let width = ((count as f64 / max as f64) * max_bar as f64).round() as usize;
    "\u{2588}".repeat(width.max(1))
}

/// Render an ImpactReport as a formatted terminal table.
pub fn render_analyze(result: &serde_json::Value) {
    // ── Extract fields ──────────────────────────────────────────────────
    let summary = result.get("summary");
    let total_tasks =
        summary.and_then(|s| s.get("total_tasks")).and_then(|v| v.as_u64()).unwrap_or(0);
    let total_events =
        summary.and_then(|s| s.get("total_events")).and_then(|v| v.as_u64()).unwrap_or(0);
    let total_errors =
        summary.and_then(|s| s.get("total_errors")).and_then(|v| v.as_u64()).unwrap_or(0);
    let date_range_days =
        summary.and_then(|s| s.get("date_range_days")).and_then(|v| v.as_u64()).unwrap_or(1);
    let avg_tasks_per_day =
        summary.and_then(|s| s.get("avg_tasks_per_day")).and_then(|v| v.as_f64()).unwrap_or(0.0);

    let token_eff = result.get("token_efficiency");
    let agent_visible =
        token_eff.and_then(|t| t.get("agent_visible_events")).and_then(|v| v.as_u64()).unwrap_or(0);
    let agent_skipped =
        token_eff.and_then(|t| t.get("agent_skipped_events")).and_then(|v| v.as_u64()).unwrap_or(0);
    let savings_pct = token_eff
        .and_then(|t| t.get("estimated_token_savings_pct"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let info = result.get("information_density");
    let avg_fields =
        info.and_then(|i| i.get("avg_fields_per_event")).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let with_location =
        info.and_then(|i| i.get("events_with_location")).and_then(|v| v.as_u64()).unwrap_or(0);
    let with_code =
        info.and_then(|i| i.get("events_with_code")).and_then(|v| v.as_u64()).unwrap_or(0);
    let with_context =
        info.and_then(|i| i.get("events_with_context")).and_then(|v| v.as_u64()).unwrap_or(0);
    let with_hint =
        info.and_then(|i| i.get("events_with_hint")).and_then(|v| v.as_u64()).unwrap_or(0);

    let patterns = result.get("command_patterns");
    let short_pct =
        patterns.and_then(|p| p.get("short_cmd_pct")).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let long_pct =
        patterns.and_then(|p| p.get("long_cmd_pct")).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let total_retries =
        patterns.and_then(|p| p.get("total_retry_runs")).and_then(|v| v.as_u64()).unwrap_or(0);
    let top_retried = patterns.and_then(|p| p.get("top_retried")).and_then(|v| v.as_array());

    // Derived percentages for token efficiency
    let total_data_events = agent_visible + agent_skipped;
    let visible_pct = if total_data_events > 0 {
        agent_visible as f64 / total_data_events as f64 * 100.0
    } else {
        0.0
    };
    let skipped_pct = if total_data_events > 0 {
        agent_skipped as f64 / total_data_events as f64 * 100.0
    } else {
        0.0
    };

    // ── Title banner (hardcoded widths to match ANSI-invisible content) ──
    eprintln!();
    eprintln!("╔═══════════════════════════════════════════════════════════════╗");
    eprintln!("║               {}{}ARSHY IMPACT REPORT{}                 ║", BOLD, CYAN, RESET);
    eprintln!("╚═══════════════════════════════════════════════════════════════╝");
    eprintln!();
    eprintln!(
        "  Summary: {} tasks over {} days ({:.1} tasks/day)",
        fmt_commas(total_tasks),
        date_range_days,
        avg_tasks_per_day
    );
    eprintln!(
        "           {} events, {} errors",
        fmt_commas(total_events),
        fmt_commas(total_errors)
    );
    eprintln!();

    // ── Token Efficiency box ────────────────────────────────────────────
    box_top();
    box_line(&format!("{}TOKEN EFFICIENCY{}", BOLD, RESET));
    box_divider();
    box_line(&format!(
        "  Agent-visible events:  {} ({:.1}%)",
        fmt_commas(agent_visible),
        visible_pct
    ));
    box_line(&format!(
        "  Agent-skipped events:  {} ({:.1}%)   \u{2190} noise filtered",
        fmt_commas(agent_skipped),
        skipped_pct
    ));
    box_line(&format!("  Estimated savings:     ~{:.0}% tokens", savings_pct));
    box_bottom();
    eprintln!();

    // ── Information Density box ─────────────────────────────────────────
    box_top();
    box_line(&format!("{}INFORMATION DENSITY{}", BOLD, RESET));
    box_divider();
    box_line(&format!("  Avg fields/event:      {:.2}", avg_fields));
    box_line(&format!(
        "  Events with location:  {} / {}",
        fmt_commas(with_location),
        fmt_commas(total_events)
    ));
    box_line(&format!(
        "  Events with code:      {} / {}",
        fmt_commas(with_code),
        fmt_commas(total_events)
    ));
    box_line(&format!(
        "  Events with context:   {} / {}",
        fmt_commas(with_context),
        fmt_commas(total_events)
    ));
    box_line(&format!(
        "  Events with hint:      {} / {}",
        fmt_commas(with_hint),
        fmt_commas(total_events)
    ));
    box_bottom();
    eprintln!();

    // ── Command Patterns box ────────────────────────────────────────────
    box_top();
    box_line(&format!("{}COMMAND PATTERNS{}", BOLD, RESET));
    box_divider();
    box_line(&format!("  Short commands:        {:.1}%", short_pct));
    box_line(&format!("  Long commands:         {:.1}%", long_pct));
    box_line(&format!("  Total retries:         {}", fmt_commas(total_retries)));
    box_line("");
    let has_retried = top_retried.is_some_and(|arr| !arr.is_empty());
    if has_retried {
        box_line("  Top retried:");
        for entry in top_retried.unwrap() {
            let cmd = entry.get(0).and_then(|v| v.as_str()).unwrap_or("");
            let count = entry.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
            let name = truncate_name(cmd, 18);
            box_line(&format!("    {:<18} {}x", name, count));
        }
    }
    box_bottom();
    eprintln!();

    // ── Top Retried box (bar chart from top_retried) ────────────────────
    if let Some(parsers) = top_retried {
        if !parsers.is_empty() {
            let max_count =
                parsers.iter().filter_map(|p| p.get(1).and_then(|v| v.as_u64())).max().unwrap_or(1);

            box_top();
            box_line(&format!("{}TOP RETRIED{}", BOLD, RESET));
            box_divider();

            for entry in parsers.iter().take(5) {
                let cmd = entry.get(0).and_then(|v| v.as_str()).unwrap_or("");
                let count = entry.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
                let bar = render_bar(count, max_count, 26);
                let name = truncate_name(cmd, 12);
                box_line(&format!("  {:<12} {:>4} {}", name, count, bar));
            }

            box_bottom();
            eprintln!();
        }
    }
}
