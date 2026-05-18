//! Tests for token-savings tracking and reporting.

mod common;

use std::collections::HashMap;

use chrono::{TimeZone, Utc};
use common::make_chunk;
use semeja::stats::{build_savings_summary, format_savings_report, save_search_stats};
use semeja::types::{CallType, SearchMode, SearchResult};
use tempfile::tempdir;

fn stats_record(ts: f64, call: &str, snippet_chars: i64, file_chars: i64) -> String {
    format!(
        "{{\"ts\": {ts}, \"call\": \"{call}\", \"results\": 3, \
         \"snippet_chars\": {snippet_chars}, \"file_chars\": {file_chars}}}"
    )
}

#[test]
fn save_search_stats_deduplicates_and_silences_errors() {
    let chunk = make_chunk("hello", "src/foo.py");
    let result = SearchResult { chunk, score: 0.9, source: SearchMode::Hybrid };
    let dir = tempdir().expect("temp dir");
    let stats_file = dir.path().join("stats.jsonl");
    let file_sizes: HashMap<String, usize> = [("src/foo.py".to_string(), 42)].into_iter().collect();

    save_search_stats(&[result.clone(), result.clone()], CallType::Search, &file_sizes, &stats_file);
    let written = std::fs::read_to_string(&stats_file).expect("read stats");
    let record: serde_json::Value = serde_json::from_str(written.lines().next().unwrap()).unwrap();
    assert_eq!(record["file_chars"], 42);

    // An unwritable path must not panic.
    save_search_stats(&[result], CallType::Search, &file_sizes, std::path::Path::new("/"));
}

#[test]
fn savings_report_without_file_is_friendly() {
    let dir = tempdir().expect("temp dir");
    let report = format_savings_report(&dir.path().join("nonexistent.jsonl"), false);
    assert!(report.contains("No stats yet"));
}

#[test]
fn savings_report_shows_buckets_and_optional_breakdown() {
    let dir = tempdir().expect("temp dir");
    let stats_file = dir.path().join("stats.jsonl");
    let now = Utc::now().timestamp() as f64;
    std::fs::write(
        &stats_file,
        format!(
            "{}\n{}\n",
            stats_record(now, "search", 1000, 20000),
            stats_record(now, "find_related", 1000, 20000)
        ),
    )
    .expect("write stats");

    let plain = format_savings_report(&stats_file, false);
    assert!(plain.contains("Savings") && plain.contains("Today"));

    let verbose = format_savings_report(&stats_file, true);
    for fragment in ["Savings", "Today", "Usage Breakdown", "search", "find_related"] {
        assert!(verbose.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn savings_report_formats_millions() {
    let dir = tempdir().expect("temp dir");
    let stats_file = dir.path().join("stats.jsonl");
    let now = Utc::now().timestamp() as f64;
    std::fs::write(&stats_file, format!("{}\n", stats_record(now, "search", 0, 4_000_000)))
        .expect("write stats");
    assert!(format_savings_report(&stats_file, false).contains("M tokens"));
}

#[test]
fn savings_do_not_subtract_unknown_baselines() {
    let dir = tempdir().expect("temp dir");
    let stats_file = dir.path().join("stats.jsonl");
    let now = Utc::now().timestamp() as f64;
    std::fs::write(
        &stats_file,
        format!(
            "{}\n{}\n",
            stats_record(now, "search", 100, 500),
            stats_record(now, "search", 1000, 0)
        ),
    )
    .expect("write stats");

    let summary = build_savings_summary(&stats_file);
    assert_eq!(summary.bucket("All time").saved_chars, 400);
    assert!(format_savings_report(&stats_file, false).contains("~100 tokens"));
}

#[test]
fn savings_tolerates_bad_json() {
    let dir = tempdir().expect("temp dir");
    let stats_file = dir.path().join("stats.jsonl");
    std::fs::write(&stats_file, "not valid json\n").expect("write stats");
    assert!(format_savings_report(&stats_file, false).contains("Savings"));
}

#[test]
fn savings_buckets_exclude_old_records() {
    let dir = tempdir().expect("temp dir");
    let stats_file = dir.path().join("stats.jsonl");
    let old = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap().timestamp() as f64;
    let now = Utc::now().timestamp() as f64;
    std::fs::write(
        &stats_file,
        format!("{}\n{}\n", stats_record(old, "search", 1000, 20000), stats_record(now, "search", 1000, 20000)),
    )
    .expect("write stats");

    let summary = build_savings_summary(&stats_file);
    assert_eq!(summary.bucket("All time").calls, 2);
    assert_eq!(summary.bucket("Today").calls, 1);
    assert_eq!(summary.bucket("Last 7 days").calls, 1);
}
