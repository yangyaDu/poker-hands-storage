use std::fs;
use std::path::Path;

use crate::benchmark::compare::types::BenchmarkCompareReport;
use crate::errors::ToolError;

pub fn write_compare_json(path: &Path, report: &BenchmarkCompareReport) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    fs::write(path, format!("{json}\n"))?;
    Ok(())
}

pub fn write_compare_markdown(
    path: &Path,
    report: &BenchmarkCompareReport,
) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, render_compare_markdown(report))?;
    Ok(())
}

pub fn render_compare_markdown(report: &BenchmarkCompareReport) -> String {
    let mut markdown = String::new();
    markdown.push_str("# Range Strata Binary vs SQLite Benchmark Compare\n\n");
    markdown.push_str(&format!("Generated at: {}\n\n", report.generated_at));
    markdown.push_str("## Summary\n\n");
    markdown.push_str(&format!(
        "- Binary report: `{}`\n",
        report.binary_report_path
    ));
    markdown.push_str(&format!(
        "- SQLite report: `{}`\n",
        report.sqlite_report_path
    ));
    markdown.push_str(&format!(
        "- Compatible workload: {}\n\n",
        report.compatible_workload
    ));

    if !report.compatibility_notes.is_empty() {
        markdown.push_str("## Compatibility Notes\n\n");
        for note in &report.compatibility_notes {
            markdown.push_str(&format!("- {note}\n"));
        }
        markdown.push('\n');
    }

    markdown.push_str("## Case Comparison\n\n");
    markdown.push_str("| case | binary avg | sqlite avg | latency ratio | binary p95 | sqlite p95 | p95 ratio | binary qps | sqlite qps | qps ratio | errors | result match |\n");
    markdown.push_str(
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n",
    );
    for case in &report.cases {
        markdown.push_str(&format!(
            "| {} | {} | {} | {:.3} | {} | {} | {:.3} | {:.2} | {:.2} | {:.3} | {}/{} | {} |\n",
            case.name,
            format_ms(case.binary.avg_ms),
            format_ms(case.sqlite.avg_ms),
            case.binary_to_sqlite_avg_latency_ratio,
            format_ms(case.binary.p95_ms),
            format_ms(case.sqlite.p95_ms),
            case.binary_to_sqlite_p95_latency_ratio,
            case.binary.qps,
            case.sqlite.qps,
            case.binary_to_sqlite_qps_ratio,
            case.binary.error_count,
            case.sqlite.error_count,
            case.result_count_match
        ));
    }
    markdown.push('\n');

    markdown.push_str("## Notes\n\n");
    for note in &report.notes {
        markdown.push_str(&format!("- {note}\n"));
    }
    markdown
}

fn format_ms(value: f64) -> String {
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
