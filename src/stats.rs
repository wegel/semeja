//! Token-savings tracking: append-only stats file and a formatted report.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use crate::types::{CallType, SearchResult};

/// Standard characters-per-token approximation.
const CHARS_PER_TOKEN: i64 = 4;
/// Width of the savings bar in the report.
const BAR_WIDTH: usize = 16;

/// The default stats file, `~/.semeja/savings.jsonl`.
pub fn default_stats_file() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".semeja").join("savings.jsonl")
}

// --- Types ---

/// Aggregated savings for one time bucket.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BucketStats {
    /// Number of recorded calls.
    pub calls: i64,
    /// Total characters returned in snippets.
    pub snippet_chars: i64,
    /// Total characters of the underlying full files.
    pub file_chars: i64,
    /// Characters saved versus reading full files.
    pub saved_chars: i64,
}

impl BucketStats {
    /// Update the bucket with one call and its character counts.
    fn add(&mut self, snippet_chars: i64, file_chars: i64) {
        self.calls += 1;
        self.snippet_chars += snippet_chars;
        self.file_chars += file_chars;
        self.saved_chars += (file_chars - snippet_chars).max(0);
    }
}

/// A full savings summary across time buckets and call types.
pub struct SavingsSummary {
    /// `(label, stats)` pairs, ordered Today / Last 7 days / All time.
    pub buckets: Vec<(String, BucketStats)>,
    /// Count of calls per call-type label.
    pub call_type_counts: Vec<(String, i64)>,
}

impl SavingsSummary {
    /// Look up a bucket by its label.
    pub fn bucket(&self, label: &str) -> &BucketStats {
        &self.buckets.iter().find(|(name, _)| name == label).expect("known bucket label").1
    }
}

#[derive(Deserialize)]
struct StatsRecord {
    ts: f64,
    call: String,
    snippet_chars: i64,
    file_chars: i64,
}

// --- Public API ---

/// Append stats about a search or `find_related` call to the stats file.
pub fn save_search_stats(
    results: &[SearchResult],
    call_type: CallType,
    file_sizes: &HashMap<String, usize>,
    stats_file: &Path,
) {
    let snippet_chars: i64 = results.iter().map(|r| r.chunk.content.chars().count() as i64).sum();

    let mut seen: Vec<&str> = Vec::new();
    let mut file_chars: i64 = 0;
    for result in results {
        let path = result.chunk.file_path.as_str();
        if seen.contains(&path) {
            continue;
        }
        seen.push(path);
        if let Some(size) = file_sizes.get(path) {
            file_chars += *size as i64;
        }
    }

    let record = serde_json::json!({
        "ts": Utc::now().timestamp_micros() as f64 / 1_000_000.0,
        "call": call_type.as_str(),
        "results": results.len(),
        "snippet_chars": snippet_chars,
        "file_chars": file_chars,
    });

    if let Some(parent) = stats_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(stats_file) {
        let _ = writeln!(file, "{record}");
    }
}

/// Read the stats file and aggregate it into a [`SavingsSummary`].
pub fn build_savings_summary(path: &Path) -> SavingsSummary {
    let now = Utc::now();
    let today = now.date_naive();
    let seven_days_ago = (now - Duration::days(7)).date_naive();

    let mut today_bucket = BucketStats::default();
    let mut last_7 = BucketStats::default();
    let mut all_time = BucketStats::default();
    let mut call_counts: Vec<(String, i64)> = Vec::new();

    let content = fs::read_to_string(path).unwrap_or_default();
    for line in content.lines() {
        let record: StatsRecord = match serde_json::from_str(line) {
            Ok(record) => record,
            Err(_) => {
                eprintln!("Skipping malformed JSON line in stats file");
                continue;
            }
        };
        bump(&mut call_counts, &record.call);
        all_time.add(record.snippet_chars, record.file_chars);

        let date = DateTime::<Utc>::from_timestamp(record.ts as i64, 0)
            .unwrap_or(now)
            .date_naive();
        if date > seven_days_ago {
            last_7.add(record.snippet_chars, record.file_chars);
        }
        if date == today {
            today_bucket.add(record.snippet_chars, record.file_chars);
        }
    }

    SavingsSummary {
        buckets: vec![
            ("Today".to_string(), today_bucket),
            ("Last 7 days".to_string(), last_7),
            ("All time".to_string(), all_time),
        ],
        call_type_counts: call_counts,
    }
}

/// Return a formatted token-savings report.
pub fn format_savings_report(path: &Path, verbose: bool) -> String {
    if !path.exists() {
        return "No stats yet. Run a search first.".to_string();
    }

    let summary = build_savings_summary(path);
    let heavy_line = format!("  {}", "═".repeat(64));
    let light_line = format!("  {}", "─".repeat(64));

    let mut lines: Vec<String> = vec![
        String::new(),
        "  Semeja Token Savings".to_string(),
        heavy_line.clone(),
        format!("  {:<12}  {:<6}  Savings", "Period", "Calls"),
        light_line.clone(),
    ];
    for (label, bucket) in &summary.buckets {
        lines.push(format_bucket_line(label, bucket));
    }
    if verbose && !summary.call_type_counts.is_empty() {
        lines.push(String::new());
        lines.push("  Usage Breakdown".to_string());
        lines.push(light_line);
        lines.push(format!("  {:<16}  Calls", "Call type"));
        let mut counts = summary.call_type_counts.clone();
        counts.sort();
        for (call_type, count) in counts {
            lines.push(format!("  {:<16}  {}", call_type, count_str(count)));
        }
        lines.push(heavy_line);
    }
    lines.push(String::new());
    lines.join("\n")
}

// --- Private helpers ---

/// Render one period bucket as a savings bar line.
fn format_bucket_line(label: &str, bucket: &BucketStats) -> String {
    let saved_tokens = bucket.saved_chars / CHARS_PER_TOKEN;
    let saved_str = if saved_tokens >= 1_000_000 {
        format!("~{:.1}M", saved_tokens as f64 / 1_000_000.0)
    } else if saved_tokens >= 1000 {
        format!("~{:.1}k", saved_tokens as f64 / 1000.0)
    } else {
        format!("~{saved_tokens}")
    };
    let calls = count_str(bucket.calls);

    if bucket.file_chars > 0 {
        let ratio = bucket.saved_chars as f64 / bucket.file_chars as f64;
        let filled = (ratio * BAR_WIDTH as f64).round() as usize;
        let bar = format!("{}{}", "█".repeat(filled.min(BAR_WIDTH)), "░".repeat(BAR_WIDTH.saturating_sub(filled)));
        let pct = (ratio * 100.0).round() as i64;
        format!("  {label:<12}  {calls:<6}  [{bar}]  {saved_str} tokens ({pct}%)")
    } else {
        let bar = "░".repeat(BAR_WIDTH);
        format!("  {label:<12}  {calls:<6}  [{bar}]  {saved_str} tokens")
    }
}

/// Format a call count with `k` suffix for large values.
fn count_str(count: i64) -> String {
    if count >= 1000 {
        format!("{:.1}k", count as f64 / 1000.0)
    } else {
        count.to_string()
    }
}

/// Increment the count for a call-type label, preserving first-seen order.
fn bump(counts: &mut Vec<(String, i64)>, call: &str) {
    if let Some(entry) = counts.iter_mut().find(|(name, _)| name == call) {
        entry.1 += 1;
    } else {
        counts.push((call.to_string(), 1));
    }
}
