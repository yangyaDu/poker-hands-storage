use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::errors::ToolError;

pub(crate) fn write_json_report<T: Serialize>(path: &Path, report: &T) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    fs::write(path, format!("{json}\n"))?;
    Ok(())
}

pub(crate) fn write_markdown_report(
    path: &Path,
    markdown: impl AsRef<str>,
) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, markdown.as_ref())?;
    Ok(())
}

pub(crate) fn format_ms(value: f64) -> String {
    if !value.is_finite() {
        return "unknown".to_owned();
    }
    if value >= 1000.0 {
        format!("{:.2} s", value / 1000.0)
    } else if value >= 10.0 {
        format!("{value:.2} ms")
    } else {
        format!("{value:.3} ms")
    }
}

pub(crate) fn format_binary_bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut unit_index = 0;
    let mut size = value as f64;
    while size >= 1024.0 && unit_index + 1 < UNITS.len() {
        size /= 1024.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{value} {}", UNITS[unit_index])
    } else {
        format!("{size:.2} {}", UNITS[unit_index])
    }
}

pub(crate) fn markdown_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut output = String::new();
    output.push_str("| ");
    output.push_str(&headers.join(" | "));
    output.push_str(" |\n| ");
    output.push_str(&vec!["---"; headers.len()].join(" | "));
    output.push_str(" |\n");
    for row in rows {
        output.push_str("| ");
        output.push_str(&row.join(" | "));
        output.push_str(" |\n");
    }
    output
}

pub fn generated_at_utc() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = duration.as_secs() as i64;
    let millis = duration.subsec_millis();
    let (year, month, day, hour, minute, second) = unix_seconds_to_utc(seconds);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn unix_seconds_to_utc(seconds: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let hour = (seconds_of_day / 3_600) as u32;
    let minute = ((seconds_of_day % 3_600) / 60) as u32;
    let second = (seconds_of_day % 60) as u32;
    let (year, month, day) = civil_from_days(days);
    (year, month, day, hour, minute, second)
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}
