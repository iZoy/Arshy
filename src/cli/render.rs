//! Observation renderers for arshy stats, benchmarks, and impact reports.
//!
//! Command execution output is always raw/JSON for agent consumption. These
//! renderers are only used for human-observation tools: stats, benchmark, analyze.

use std::fmt::Write;

// ── ANSI colors ──────────────────────────────────────────────────────────────

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const CYAN: &str = "\x1b[36m";

// ── Box drawing ──────────────────────────────────────────────────────────────

const TL: &str = "╭";
const TR: &str = "╮";
const BL: &str = "╰";
const BR: &str = "╯";
const H: &str = "─";
const V: &str = "│";

const BOX_W: usize = 60;
const INNER_W: usize = BOX_W - 4;

// ── Box drawing helpers ──────────────────────────────────────────────────────

fn box_top(out: &mut String) {
    let _ = writeln!(out, "  {}{}{}", TL, H.repeat(BOX_W - 2), TR);
}

fn box_bottom(out: &mut String) {
    let _ = writeln!(out, "  {}{}{}", BL, H.repeat(BOX_W - 2), BR);
}

fn box_divider(out: &mut String) {
    let _ = writeln!(out, "  ├{}┤", H.repeat(BOX_W - 2));
}

fn box_line(out: &mut String, content: &str) {
    let visible_len = visible_width(content);
    let padding = INNER_W.saturating_sub(visible_len);
    let _ = writeln!(out, "  {} {}{}{}", V, content, " ".repeat(padding), V);
}

// ── Text helpers ─────────────────────────────────────────────────────────────

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

fn truncate_name(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let end = s.char_indices().nth(width.saturating_sub(3)).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

fn render_bar(count: u64, max: u64, max_bar: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let width = ((count as f64 / max as f64) * max_bar as f64).round() as usize;
    "\u{2588}".repeat(width.max(1))
}

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
    (0x4E00..=0x9FFF).contains(&cp)
        || (0x3000..=0x303F).contains(&cp)
        || (0xFF00..=0xFFEF).contains(&cp)
        || (0x1F300..=0x1F9FF).contains(&cp)
        || (0x2E80..=0x2FDF).contains(&cp)
        || (0x3400..=0x4DBF).contains(&cp)
        || (0x20000..=0x2A6DF).contains(&cp)
}

fn pad_right(s: &str, width: usize) -> String {
    let vis = visible_width(s);
    if vis >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - vis))
    }
}

fn pad_left(s: &str, width: usize) -> String {
    let vis = visible_width(s);
    if vis >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - vis), s)
    }
}

fn fmt_diff(diff: Option<f64>, suffix: &str, pct: bool) -> String {
    match diff {
        Some(d) if d > 0.001 => {
            let val = if pct { d * 100.0 } else { d };
            format!("  \x1b[32m(+{:.1}{})\x1b[0m", val, suffix)
        }
        Some(d) if d < -0.001 => {
            let val = if pct { d * 100.0 } else { d };
            format!("  \x1b[31m({:.1}{})\x1b[0m", val, suffix)
        }
        _ => String::new(),
    }
}

fn fmt_diff_int(diff: Option<i64>, suffix: &str) -> String {
    match diff {
        Some(d) if d > 0 => {
            format!("  \x1b[31m(+{}{})\x1b[0m", d, suffix)
        }
        Some(d) if d < 0 => {
            format!("  \x1b[32m({}{})\x1b[0m", d, suffix)
        }
        _ => String::new(),
    }
}

// ── Stats renderer ───────────────────────────────────────────────────────────

pub fn render_stats(result: &serde_json::Value) -> String {
    let mut out = String::new();
    let total_tasks = result.get("total_tasks").and_then(|v| v.as_u64()).unwrap_or(0);
    let total_events = result.get("total_events").and_then(|v| v.as_u64()).unwrap_or(0);
    let total_errors = result.get("total_errors").and_then(|v| v.as_u64()).unwrap_or(0);
    let failure_rate = result.get("failure_rate").and_then(|v| v.as_f64()).map(|r| r * 100.0);
    let avg_duration = result.get("avg_duration_ms").and_then(|v| v.as_f64());
    let db_size = result.get("db_size_bytes").and_then(|v| v.as_u64());

    let by_status = result.get("by_status");
    let running = by_status.and_then(|s| s.get("running")).and_then(|v| v.as_u64()).unwrap_or(0);
    let completed =
        by_status.and_then(|s| s.get("completed")).and_then(|v| v.as_u64()).unwrap_or(0);
    let failed = by_status.and_then(|s| s.get("failed")).and_then(|v| v.as_u64()).unwrap_or(0);
    let killed = by_status.and_then(|s| s.get("killed")).and_then(|v| v.as_u64()).unwrap_or(0);
    let timeout = by_status.and_then(|s| s.get("timeout")).and_then(|v| v.as_u64()).unwrap_or(0);

    let parser_coverage_pct = result.get("parser_coverage_pct").and_then(|v| v.as_f64());
    let context_enriched = result.get("context_enriched").and_then(|v| v.as_u64());
    let dedup_collapsed = result.get("dedup_collapsed").and_then(|v| v.as_u64());
    let correlated_errors = result.get("correlated_errors").and_then(|v| v.as_u64());
    let per_parser_usage = result.get("per_parser_usage").and_then(|v| v.as_array());

    let _ = writeln!(out);
    box_top(&mut out);
    box_line(&mut out, &format!("{}{}ARSHY DAEMON STATS{}", BOLD, CYAN, RESET));
    box_divider(&mut out);

    // Purpose-aware stats: when tasks carry purpose labels (dogfood/sample),
    // split the headline into real development vs test workload. This keeps
    // the published failure metrics honest — deliberate error fixtures no
    // longer count against real-development failure rate.
    let purpose_breakdown = result.get("purpose_breakdown").and_then(|v| v.as_array());
    let (mut real_total, mut test_total) = (0u64, 0u64);
    let (mut real_failed, mut test_failed) = (0u64, 0u64);
    let (mut real_ms, mut test_ms) = (Vec::<f64>::new(), Vec::<f64>::new());
    if let Some(items) = purpose_breakdown {
        for item in items {
            let is_real = item.get("purpose").and_then(|v| v.as_str()) == Some("real");
            let total = item.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
            let failed = item.get("failed").and_then(|v| v.as_u64()).unwrap_or(0);
            if is_real {
                real_total += total;
                real_failed += failed;
            } else {
                test_total += total;
                test_failed += failed;
            }
            if let Some(ms) = item.get("avg_duration_ms").and_then(|v| v.as_f64()) {
                if is_real {
                    real_ms.push(ms);
                } else {
                    test_ms.push(ms);
                }
            }
        }
    }
    let has_purpose_split = real_total > 0 || test_total > 0;

    if has_purpose_split {
        box_line(
            &mut out,
            &format!(
                "Tasks:   {} total ({} real \u{00b7} {} test) \u{2014} {} running, {} done, {} failed, {} killed, {} timeout",
                total_tasks, real_total, test_total, running, completed, failed, killed, timeout
            ),
        );
    } else {
        box_line(
            &mut out,
            &format!(
                "Tasks:   {} total ({} running, {} done, {} failed, {} killed, {} timeout)",
                total_tasks, running, completed, failed, killed, timeout
            ),
        );
    }
    box_line(&mut out, &format!("Events:  {} total ({} errors)", total_events, total_errors));

    if has_purpose_split {
        let rf = if real_total > 0 { real_failed as f64 * 100.0 / real_total as f64 } else { 0.0 };
        let tf = if test_total > 0 { test_failed as f64 * 100.0 / test_total as f64 } else { 0.0 };
        box_line(&mut out, &format!("Failure rate:  {rf:.0}% real \u{00b7} {tf:.0}% test"));
        let ravg = if real_total > 0 && !real_ms.is_empty() {
            real_ms.iter().sum::<f64>() / real_ms.len() as f64
        } else {
            avg_duration.unwrap_or(0.0)
        };
        let tavg = if test_total > 0 && !test_ms.is_empty() {
            test_ms.iter().sum::<f64>() / test_ms.len() as f64
        } else {
            0.0
        };
        box_line(&mut out, &format!("Avg duration:  {ravg:.0}ms real \u{00b7} {tavg:.0}ms test"));
    } else {
        if let Some(rate) = failure_rate {
            box_line(&mut out, &format!("Failure rate:  {:.0}%", rate));
        }
        if let Some(ms) = avg_duration {
            box_line(&mut out, &format!("Avg duration:  {:.0}ms", ms));
        }
    }
    if let Some(bytes) = db_size {
        box_line(&mut out, &format!("DB size:       {} bytes", bytes));
    }

    let has_intelligence = parser_coverage_pct.is_some()
        || context_enriched.is_some()
        || dedup_collapsed.is_some()
        || correlated_errors.is_some()
        || per_parser_usage.is_some();
    if has_intelligence {
        box_divider(&mut out);
        box_line(&mut out, &format!("{}Intelligence{}", BOLD, RESET));
        if let Some(cov) = parser_coverage_pct {
            box_line(
                &mut out,
                &format!("  Parser coverage: {:.0}% events structured (not raw log)", cov),
            );
        }
        if let Some(ctx) = context_enriched {
            box_line(&mut out, &format!("  Context enriched: {} events with source code", ctx));
        }
        if let Some(dedup) = dedup_collapsed {
            if dedup > 0 {
                box_line(
                    &mut out,
                    &format!("  Dedup saved:     {} duplicate lines suppressed", dedup),
                );
            }
        }
        if let Some(corr) = correlated_errors {
            if corr > 0 {
                box_line(
                    &mut out,
                    &format!("  Git correlation:  {} errors linked to recent changes", corr),
                );
            }
        }
        if let Some(parsers) = per_parser_usage {
            if !parsers.is_empty() {
                box_line(&mut out, "");
                box_line(&mut out, "  Top parser hits:");
                let max_count =
                    parsers.iter().filter_map(|p| p.get("count")?.as_u64()).max().unwrap_or(1);
                for p in parsers.iter().take(5) {
                    if let (Some(name), Some(count)) = (
                        p.get("parser").and_then(|v| v.as_str()),
                        p.get("count").and_then(|v| v.as_u64()),
                    ) {
                        let bar = render_bar(count, max_count, 24);
                        let name_trunc = truncate_name(name, 12);
                        box_line(&mut out, &format!("    {:<12} {:>4} {}", name_trunc, count, bar));
                    }
                }
            }
        }
    }

    box_bottom(&mut out);
    let _ = writeln!(out);
    out
}

// ── Benchmark renderer ───────────────────────────────────────────────────────

pub fn render_benchmark(result: &serde_json::Value) -> String {
    let mut out = String::new();
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
    let unparsed = result.get("total_unparsed_error_lines").and_then(|v| v.as_u64()).unwrap_or(0);

    let comparison = result.get("comparison");

    let ratio_diff = comparison.and_then(|c| c.get("compression_ratio")).and_then(|v| v.as_f64());
    let accuracy_diff = comparison.and_then(|c| c.get("avg_accuracy")).and_then(|v| v.as_f64());
    let speed_diff =
        comparison.and_then(|c| c.get("error_speed_advantage_pct")).and_then(|v| v.as_f64());
    let unparsed_diff =
        comparison.and_then(|c| c.get("total_unparsed_error_lines")).and_then(|v| v.as_i64());

    let ratio_diff_str = fmt_diff(ratio_diff, "x", false);
    let acc_diff_str = fmt_diff(accuracy_diff, "%", true);
    let speed_diff_str = fmt_diff(speed_diff, "%", false);
    let unparsed_diff_str = fmt_diff_int(unparsed_diff, " lines");

    let _ = writeln!(out);
    let _ = writeln!(out, "╔═══════════════════════════════════════════════════════════════╗");
    let _ = writeln!(
        out,
        "║             {}ARSHY PARSER BENCHMARK RESULTS{}                ║",
        BOLD, RESET
    );
    let _ = writeln!(out, "╚═══════════════════════════════════════════════════════════════╝");
    let _ = writeln!(out);
    let _ = writeln!(out, "  Scope: {} fixtures across 37 builtin parsers", fixtures);
    let _ = writeln!(out, "         {} raw input lines → {} structured events", raw_lines, events);
    let _ = writeln!(out);

    box_top(&mut out);
    box_line(&mut out, &format!("{}INFORMATION DENSITY{}", BOLD, RESET));
    box_divider(&mut out);
    box_line(&mut out, &format!("  Structured output:  {:.1} actionable fields/event", fields_per));
    box_line(&mut out, "                       (type, severity, code,");
    box_line(&mut out, "                        file, line, message)");
    box_line(&mut out, "");
    box_line(&mut out, "  Raw text output:    0 structured fields/line");
    box_line(&mut out, "                       (agent must parse everything)");
    box_line(&mut out, "");
    box_line(&mut out, &format!("  Total fields:       {} across {} events", fields, events));
    box_bottom(&mut out);
    let _ = writeln!(out);

    box_top(&mut out);
    box_line(&mut out, &format!("{}TOKEN EFFICIENCY{}", BOLD, RESET));
    box_divider(&mut out);
    box_line(&mut out, &format!("  Raw text (approx BPE): {} tokens", raw_tokens));
    box_line(&mut out, &format!("  Structured JSON:       {} tokens", struct_tokens));
    box_line(&mut out, &format!("  Ratio:                 {:.1}x{}", ratio, ratio_diff_str));
    box_bottom(&mut out);
    let _ = writeln!(out);

    box_top(&mut out);
    box_line(&mut out, &format!("{}ERROR LOCATION SPEED{}", BOLD, RESET));
    box_divider(&mut out);
    box_line(
        &mut out,
        &format!("  Structured faster:     {:.0}% of fixtures{}", speed, speed_diff_str),
    );
    box_bottom(&mut out);
    let _ = writeln!(out);

    box_top(&mut out);
    box_line(&mut out, &format!("{}PARSER QUALITY & ACCURACY{}", BOLD, RESET));
    box_divider(&mut out);
    box_line(&mut out, &format!("  Average accuracy:      {:.0}%{}", accuracy, acc_diff_str));
    box_line(&mut out, &format!("  Unparsed error lines:  {}{}", unparsed, unparsed_diff_str));
    box_bottom(&mut out);
    let _ = writeln!(out);

    if let Some(details) = result.get("details").and_then(|v| v.as_array()) {
        let _ = writeln!(out, "  Per-parser breakdown:");
        let _ = writeln!(
            out,
            "  ┌──────────────┬───────┬────────┬────────┬──────────┬──────────┬──────────┐"
        );
        let _ = writeln!(
            out,
            "  │ Parser       │ Lines │ Events │ Fields │ Compress │ Accuracy │ Unparsed │"
        );
        let _ = writeln!(
            out,
            "  ├──────────────┼───────┼────────┼────────┼──────────┼──────────┼──────────┤"
        );

        for d in details {
            let parser = d.get("parser").and_then(|v| v.as_str()).unwrap_or("");
            let lines = d.get("raw_lines").and_then(|v| v.as_u64()).unwrap_or(0);
            let evts = d.get("events").and_then(|v| v.as_u64()).unwrap_or(0);
            let flds = d.get("structured_fields").and_then(|v| v.as_u64()).unwrap_or(0);
            let comp = d.get("compression_ratio").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let acc = d.get("accuracy").and_then(|v| v.as_f64()).unwrap_or(0.0) * 100.0;
            let unparsed_fixture =
                d.get("unparsed_error_lines").and_then(|v| v.as_u64()).unwrap_or(0);

            let acc_str = if let Some(diff) = d.get("accuracy_diff").and_then(|v| v.as_f64()) {
                if diff > 0.001 {
                    format!("{:.0}% \x1b[32m(+{:.0})\x1b[0m", acc, diff * 100.0)
                } else if diff < -0.001 {
                    format!("{:.0}% \x1b[31m({:.0})\x1b[0m", acc, diff * 100.0)
                } else {
                    format!("{:.0}%", acc)
                }
            } else {
                format!("{:.0}%", acc)
            };

            let unparsed_str = if let Some(diff) = d.get("unparsed_diff").and_then(|v| v.as_i64()) {
                if diff > 0 {
                    format!("{} \x1b[31m(+{})\x1b[0m", unparsed_fixture, diff)
                } else if diff < 0 {
                    format!("{} \x1b[32m({})\x1b[0m", unparsed_fixture, diff)
                } else {
                    format!("{}", unparsed_fixture)
                }
            } else {
                format!("{}", unparsed_fixture)
            };

            let p_name = if parser.len() > 12 {
                let end = parser.char_indices().nth(12).map(|(i, _)| i).unwrap_or(parser.len());
                &parser[..end]
            } else {
                parser
            };

            let name_padded = pad_right(p_name, 12);
            let lines_padded = pad_left(&lines.to_string(), 5);
            let evts_padded = pad_left(&evts.to_string(), 6);
            let flds_padded = pad_left(&flds.to_string(), 6);
            let comp_padded = pad_left(&format!("{:.1}x", comp), 8);
            let acc_padded = pad_left(&acc_str, 8);
            let unparsed_padded = pad_left(&unparsed_str, 8);

            let _ = writeln!(
                out,
                "  │ {} │ {} │ {} │ {} │ {} │ {} │ {} │",
                name_padded,
                lines_padded,
                evts_padded,
                flds_padded,
                comp_padded,
                acc_padded,
                unparsed_padded
            );
        }

        let _ = writeln!(
            out,
            "  └──────────────┴───────┴────────┴────────┴──────────┴──────────┴──────────┘"
        );
    }
    let _ = writeln!(out);
    out
}

// ── Analyze renderer ─────────────────────────────────────────────────────────

pub fn render_analyze(result: &serde_json::Value) -> String {
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

    let patterns = result.get("command_patterns");
    let short_pct =
        patterns.and_then(|p| p.get("short_cmd_pct")).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let long_pct =
        patterns.and_then(|p| p.get("long_cmd_pct")).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let total_retries =
        patterns.and_then(|p| p.get("total_retry_runs")).and_then(|v| v.as_u64()).unwrap_or(0);
    let top_retried = patterns.and_then(|p| p.get("top_retried")).and_then(|v| v.as_array());

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

    let mut out = String::new();

    let _ = writeln!(out);
    let _ = writeln!(out, "╔═══════════════════════════════════════════════════════════════╗");
    let _ = writeln!(
        out,
        "║               {}{}ARSHY IMPACT REPORT{}                 ║",
        BOLD, CYAN, RESET
    );
    let _ = writeln!(out, "╚═══════════════════════════════════════════════════════════════╝");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  Summary: {} tasks over {} days ({:.1} tasks/day)",
        fmt_commas(total_tasks),
        date_range_days,
        avg_tasks_per_day
    );
    let _ = writeln!(
        out,
        "           {} events, {} errors",
        fmt_commas(total_events),
        fmt_commas(total_errors)
    );
    let _ = writeln!(out);

    box_top(&mut out);
    box_line(&mut out, &format!("{}TOKEN EFFICIENCY{}", BOLD, RESET));
    box_divider(&mut out);
    box_line(
        &mut out,
        &format!("  Agent-visible events:  {} ({:.1}%)", fmt_commas(agent_visible), visible_pct),
    );
    box_line(
        &mut out,
        &format!(
            "  Agent-skipped events:  {} ({:.1}%)   \u{2190} noise filtered",
            fmt_commas(agent_skipped),
            skipped_pct
        ),
    );
    box_line(&mut out, &format!("  Estimated savings:     ~{:.0}% tokens", savings_pct));
    box_bottom(&mut out);
    let _ = writeln!(out);

    box_top(&mut out);
    box_line(&mut out, &format!("{}INFORMATION DENSITY{}", BOLD, RESET));
    box_divider(&mut out);
    box_line(&mut out, &format!("  Avg fields/event:      {:.2}", avg_fields));
    box_line(
        &mut out,
        &format!(
            "  Events with location:  {} / {}",
            fmt_commas(with_location),
            fmt_commas(total_events)
        ),
    );
    box_line(
        &mut out,
        &format!(
            "  Events with code:      {} / {}",
            fmt_commas(with_code),
            fmt_commas(total_events)
        ),
    );
    box_line(
        &mut out,
        &format!(
            "  Events with context:   {} / {}",
            fmt_commas(with_context),
            fmt_commas(total_events)
        ),
    );
    box_bottom(&mut out);
    let _ = writeln!(out);

    let repair = result.get("repair_loop");
    let fix_loops = repair.and_then(|r| r.get("fix_loops")).and_then(|v| v.as_u64()).unwrap_or(0);
    let avg_retries =
        repair.and_then(|r| r.get("avg_retries_to_fix")).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let avg_fix_ms =
        repair.and_then(|r| r.get("avg_fix_duration_ms")).and_then(|v| v.as_u64()).unwrap_or(0);
    let fastest_fix_ms =
        repair.and_then(|r| r.get("fastest_fix_ms")).and_then(|v| v.as_u64()).unwrap_or(0);

    box_top(&mut out);
    box_line(&mut out, &format!("{}COMMAND PATTERNS{}", BOLD, RESET));
    box_divider(&mut out);
    box_line(&mut out, &format!("  Short commands:        {:.1}%", short_pct));
    box_line(&mut out, &format!("  Long commands:         {:.1}%", long_pct));
    box_line(&mut out, &format!("  Total retries:         {}", fmt_commas(total_retries)));
    box_line(&mut out, "");
    let has_retried = top_retried.is_some_and(|arr| !arr.is_empty());
    if has_retried {
        box_line(&mut out, "  Top retried:");
        for entry in top_retried.unwrap() {
            let cmd = entry.get(0).and_then(|v| v.as_str()).unwrap_or("");
            let count = entry.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
            let name = truncate_name(cmd, 18);
            box_line(&mut out, &format!("    {:<18} {}x", name, count));
        }
    }
    box_bottom(&mut out);
    let _ = writeln!(out);

    if fix_loops > 0 {
        box_top(&mut out);
        box_line(&mut out, &format!("{}REPAIR LOOP{}", BOLD, RESET));
        box_divider(&mut out);
        box_line(&mut out, &format!("  Fix loops:             {}", fmt_commas(fix_loops)));
        box_line(&mut out, &format!("  Avg retries to fix:    {:.1}", avg_retries));
        box_line(
            &mut out,
            &format!("  Avg fix duration:      {}", format_duration_text(avg_fix_ms)),
        );
        box_line(
            &mut out,
            &format!("  Fastest fix:           {}", format_duration_text(fastest_fix_ms)),
        );
        box_bottom(&mut out);
        let _ = writeln!(out);
    }

    if let Some(parsers) = top_retried {
        if !parsers.is_empty() {
            let max_count =
                parsers.iter().filter_map(|p| p.get(1).and_then(|v| v.as_u64())).max().unwrap_or(1);

            box_top(&mut out);
            box_line(&mut out, &format!("{}TOP RETRIED{}", BOLD, RESET));
            box_divider(&mut out);

            for entry in parsers.iter().take(5) {
                let cmd = entry.get(0).and_then(|v| v.as_str()).unwrap_or("");
                let count = entry.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
                let bar = render_bar(count, max_count, 26);
                let name = truncate_name(cmd, 12);
                box_line(&mut out, &format!("  {:<12} {:>4} {}", name, count, bar));
            }

            box_bottom(&mut out);
            let _ = writeln!(out);
        }
    }
    out
}

// ── Run-result text renderer (bash-proxy / agent-facing plain text) ─────────

/// Status icon for a run result's `status` field.
fn status_icon(status: &str) -> &'static str {
    match status {
        "completed" => "✓",
        "failed" | "timeout" => "✗",
        "killed" => "⊘",
        "running" => "⟳",
        _ => "?",
    }
}

/// Compact duration string: `123ms` / `2.5s`.
fn format_duration_text(ms: u64) -> String {
    if ms >= 1000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}ms", ms)
    }
}

/// Render a `task/run` response as concise plain text — the "bash replacement"
/// presentation used by the transparent shell proxy.
///
/// - Short commands: raw output, byte-for-byte (like a native shell).
/// - Long commands: one-line structured summary (`✓ 2 errors, 10.5s`) plus the
///   root cause / changed files on failure, then the top error events as
///   `file:line: message` lines.
pub fn render_run_text(result: &serde_json::Value) -> String {
    let is_short = result["short_command"].as_bool().unwrap_or(false);
    let status = result["status"].as_str().unwrap_or("");

    if is_short {
        let raw = result["raw_output"].as_str().unwrap_or("");
        if raw.is_empty() && (status == "failed" || status == "timeout") {
            let label = if status == "timeout" { "timed out" } else { "failed" };
            return format!(
                "[command {}: exit code {}]",
                label,
                result["exit_code"].as_i64().unwrap_or(-1)
            );
        }
        return raw.to_string();
    }

    let mut out = one_line_summary(result);
    let events = result.get("events").cloned().unwrap_or_default();
    let event_text = render_top_events(&events, 8);
    if !event_text.is_empty() {
        out.push('\n');
        out.push_str(&event_text);
    }
    // Zero structured events → the agent cannot see the command output; point
    // it at the raw-output channel (token restraint: hint only).
    let event_count = result.get("event_count").and_then(|v| v.as_u64()).unwrap_or(0);
    if event_count == 0 {
        if let Some(tid) = result.get("task_id").and_then(|v| v.as_str()) {
            if !tid.is_empty() {
                out.push_str(&format!(
                    "\n(0 structured events — fetch original output: arshy_exec(action:\"raw\", task_id:\"{}\"))",
                    tid
                ));
            }
        }
    }
    out.trim_end_matches('\n').to_string()
}

/// One-line structured summary + root cause + changed files for a run result.
fn one_line_summary(result: &serde_json::Value) -> String {
    let status = result["status"].as_str().unwrap_or("");
    let error_count = result.get("error_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let exit_code = result.get("exit_code").and_then(|v| v.as_i64());
    let duration_ms = result.get("duration_ms").and_then(|v| v.as_u64());

    let mut text = format!(
        "{} {} error{}",
        status_icon(status),
        error_count,
        if error_count == 1 { "" } else { "s" }
    );
    if let Some(ms) = duration_ms {
        text.push_str(", ");
        text.push_str(&format_duration_text(ms));
    }
    if let Some(code) = exit_code {
        if code != 0 {
            text.push_str(&format!(" (exit {})", code));
        }
    }

    if let Some(rc) = result.get("root_cause") {
        if let Some(msg) = rc.get("message").and_then(|v| v.as_str()) {
            if !msg.is_empty() {
                text.push_str(&format!("\nRoot cause: {}", msg));
            }
        }
    }

    let has_errors = error_count > 0;
    let exit_nonzero = exit_code.is_some_and(|c| c != 0);
    if has_errors || exit_nonzero {
        if let Some(pc) = result.get("project_context") {
            if let Some(stat) = pc.get("git_diff_stat").and_then(|v| v.as_str()) {
                if !stat.is_empty() {
                    text.push_str(&format!("\nChanged files:\n{}", stat));
                }
            }
        }
    }
    text
}

/// Render up to `limit` error/warning events as compact `file:line: message` lines.
pub fn render_top_events(events: &serde_json::Value, limit: usize) -> String {
    let Some(arr) = events.as_array() else {
        return String::new();
    };
    let mut out = String::new();
    let mut shown = 0usize;
    for ev in arr.iter() {
        if shown >= limit {
            break;
        }
        let severity = ev["severity"].as_str().unwrap_or("");
        if severity != "error" && severity != "warning" {
            continue;
        }
        let msg = ev["message"].as_str().unwrap_or("");
        let loc = ev.get("location");
        let file = loc.and_then(|l| l.get("file")).and_then(|v| v.as_str()).unwrap_or("");
        let line = loc.and_then(|l| l.get("line")).and_then(|v| v.as_u64());
        match (file, line) {
            (f, Some(l)) if !f.is_empty() => {
                let _ = writeln!(out, "{}:{}: {}", f, l, msg);
            }
            _ => {
                let _ = writeln!(out, "{}", msg);
            }
        }
        shown += 1;
    }
    out
}
// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
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
            out.push(ch);
        }
        out
    }

    #[test]
    fn render_stats_reports_task_counts() {
        let s = strip_ansi(&render_stats(&json!({
            "total_tasks": 10,
            "by_status": {"running": 1, "completed": 8, "failed": 1}
        })));
        assert!(s.contains("ARSHY DAEMON STATS"));
        assert!(s.contains("10 total"));
    }

    #[test]
    fn render_benchmark_reports_scope() {
        let s = strip_ansi(&render_benchmark(&json!({
            "total_fixtures": 37,
            "compression_ratio": 5.0
        })));
        assert!(s.contains("ARSHY PARSER BENCHMARK RESULTS"));
        assert!(s.contains("37 fixtures"));
        assert!(s.contains("TOKEN EFFICIENCY"));
    }

    #[test]
    fn render_analyze_reports_sections() {
        let s = strip_ansi(&render_analyze(&json!({
            "summary": {"total_tasks": 5},
            "command_patterns": {"top_retried": []}
        })));
        assert!(s.contains("ARSHY IMPACT REPORT"));
        assert!(s.contains("TOKEN EFFICIENCY"));
        assert!(s.contains("INFORMATION DENSITY"));
    }

    #[test]
    fn render_analyze_shows_repair_loop_section_when_loops_exist() {
        let s = strip_ansi(&render_analyze(&json!({
            "summary": {"total_tasks": 5},
            "command_patterns": {"top_retried": []},
            "repair_loop": {
                "fix_loops": 3,
                "avg_retries_to_fix": 2.0,
                "avg_fix_duration_ms": 15000,
                "fastest_fix_ms": 4000
            }
        })));
        assert!(s.contains("REPAIR LOOP"));
        assert!(s.contains("Fix loops:             3"));
        assert!(s.contains("Avg retries to fix:    2.0"));
        assert!(s.contains("Avg fix duration:      15.0s"));
        assert!(s.contains("Fastest fix:           4.0s"));

        // No loops → no section, but rendering stays robust.
        let empty = strip_ansi(&render_analyze(&json!({
            "summary": {"total_tasks": 5},
            "command_patterns": {"top_retried": []},
            "repair_loop": {"fix_loops": 0, "avg_retries_to_fix": 0.0,
                            "avg_fix_duration_ms": 0, "fastest_fix_ms": 0}
        })));
        assert!(!empty.contains("REPAIR LOOP"));
    }

    #[test]
    fn renderers_are_robust_to_missing_fields() {
        assert!(render_stats(&json!({})).ends_with('\n'));
        assert!(render_benchmark(&json!({})).ends_with('\n'));
        assert!(render_analyze(&json!({})).ends_with('\n'));
    }
    #[test]
    fn render_run_text_short_raw() {
        let r = render_run_text(&json!({
            "short_command": true,
            "status": "completed",
            "raw_output": "hello world\n",
            "exit_code": 0
        }));
        assert_eq!(r, "hello world\n");
    }

    #[test]
    fn render_run_text_short_timeout() {
        let r = render_run_text(&json!({
            "short_command": true,
            "status": "timeout",
            "raw_output": "",
            "exit_code": 124
        }));
        assert!(r.contains("timed out"));
    }

    #[test]
    fn render_run_text_long_summary_and_events() {
        let r = render_run_text(&json!({
            "short_command": false,
            "status": "failed",
            "exit_code": 1,
            "error_count": 2,
            "warning_count": 1,
            "duration_ms": 10500,
            "root_cause": {"message": "cannot find type `X`"},
            "project_context": {"git_diff_stat": " src/main.rs | 2 +-"},
            "events": [
                {"type": "diagnostic", "severity": "error", "message": "cannot find type `X`",
                 "location": {"file": "src/main.rs", "line": 42, "column": 9}},
                {"type": "log", "severity": "info", "message": "noise"},
                {"type": "diagnostic", "severity": "error", "message": "unused import",
                 "location": {"file": "src/lib.rs", "line": 7}}
            ]
        }));
        assert!(r.contains("✗ 2 errors, 10.5s (exit 1)"), "{r}");
        assert!(r.contains("Root cause: cannot find type `X`"), "{r}");
        assert!(r.contains("src/main.rs:42: cannot find type `X`"), "{r}");
        assert!(r.contains("src/lib.rs:7: unused import"), "{r}");
        assert!(!r.contains("noise"), "info events must be filtered: {r}");
        assert!(r.contains("src/main.rs | 2 +-"), "{r}");
    }

    #[test]
    fn render_top_events_filters_and_limits() {
        let evs = json!([
            {"type": "diagnostic", "severity": "error", "message": "a", "location": {"file": "f.rs", "line": 1}},
            {"type": "diagnostic", "severity": "warning", "message": "b"},
            {"type": "diagnostic", "severity": "error", "message": "c"},
            {"type": "log", "severity": "error", "message": "d"}
        ]);
        let out = render_top_events(&evs, 2);
        assert!(out.contains("f.rs:1: a"));
        assert!(out.contains("b"));
        assert!(!out.contains("c"), "limit not respected: {out}");
    }
}
