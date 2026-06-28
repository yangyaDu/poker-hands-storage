use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::float32_precision::Float32PrecisionStats;
use crate::errors::ToolError;

const FREQUENCY_TOLERANCE: f64 = 1e-6;
const HAND_EV_TOLERANCE: f64 = 1e-5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VerifyMode {
    Standalone,
    Cross,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerifyLayer {
    FileExistence,
    Manifest,
    Catalog,
    IndexHeader,
    PackHeader,
    IndexPackCross,
    ConcreteIndexConsistency,
    SourceCross,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyFailure {
    pub layer: VerifyLayer,
    pub check: String,
    pub reason: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DimensionVerifyDetail {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub checked: bool,
    pub index_records: u32,
    pub bin_file_size_bytes: u64,
    pub idx_file_size_bytes: u64,
    pub header_failures: usize,
    pub index_pack_cross_failures: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_cross_failures: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_cross_records: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyOptionsSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_failures: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyTolerances {
    pub frequency: f64,
    pub hand_ev: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrecisionPolicy {
    pub numeric_fields: &'static str,
    pub nullable_hand_ev: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyPrecision {
    pub frequency: Float32PrecisionStats,
    pub hand_ev: Float32PrecisionStats,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyTotals {
    pub dimensions: usize,
    pub manifest_ok: bool,
    pub catalog_ok: bool,
    pub index_files_ok: usize,
    pub index_files_failed: usize,
    pub pack_files_ok: usize,
    pub pack_files_failed: usize,
    pub index_pack_cross_failures: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_source_records: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_source_records: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_binary_records: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeStrataVerifyReport {
    pub generated_at: String,
    pub mode: VerifyMode,
    pub directory: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_db_path: Option<String>,
    pub verify_checksums: bool,
    pub tolerances: VerifyTolerances,
    pub precision_policy: PrecisionPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<VerifyPrecision>,
    pub options: VerifyOptionsSummary,
    pub totals: VerifyTotals,
    pub dimensions: Vec<DimensionVerifyDetail>,
    pub failures: Vec<VerifyFailure>,
    pub repair_suggestions: Vec<String>,
}

impl RangeStrataVerifyReport {
    pub fn new(
        mode: VerifyMode,
        directory: String,
        source_db_path: Option<String>,
        verify_checksums: bool,
        options: VerifyOptionsSummary,
        dimensions: Vec<DimensionVerifyDetail>,
        failures: Vec<VerifyFailure>,
    ) -> Self {
        let totals = calculate_totals(&dimensions, &failures);
        let capped_failures = failures.into_iter().take(200).collect::<Vec<_>>();
        let repair_suggestions = repair_suggestions(&capped_failures);
        Self {
            generated_at: utc_now_iso8601(),
            mode,
            directory,
            source_db_path,
            verify_checksums,
            tolerances: VerifyTolerances {
                frequency: FREQUENCY_TOLERANCE,
                hand_ev: HAND_EV_TOLERANCE,
            },
            precision_policy: PrecisionPolicy {
                numeric_fields: "float32-bit-exact",
                nullable_hand_ev: "null-or-float32-bit-exact",
            },
            precision: None,
            options,
            totals,
            dimensions,
            failures: capped_failures,
            repair_suggestions,
        }
    }

    pub fn with_cross_totals(
        mut self,
        checked_source_records: u64,
        failed_source_records: u64,
        extra_binary_records: u64,
        precision: VerifyPrecision,
    ) -> Self {
        self.totals.checked_source_records = Some(checked_source_records);
        self.totals.failed_source_records = Some(failed_source_records);
        self.totals.extra_binary_records = Some(extra_binary_records);
        self.precision = Some(precision);
        self
    }

    pub fn has_failures(&self) -> bool {
        !self.totals.manifest_ok
            || !self.totals.catalog_ok
            || self.totals.index_files_failed > 0
            || self.totals.pack_files_failed > 0
            || self.totals.index_pack_cross_failures > 0
            || self.totals.failed_source_records.unwrap_or_default() > 0
            || self.totals.extra_binary_records.unwrap_or_default() > 0
    }
}

pub fn write_json_report(report: &RangeStrataVerifyReport, path: &Path) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    fs::write(path, format!("{json}\n"))?;
    Ok(())
}

pub fn write_markdown_report(
    report: &RangeStrataVerifyReport,
    path: &Path,
) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, render_markdown(report))?;
    Ok(())
}

pub fn render_markdown(report: &RangeStrataVerifyReport) -> String {
    let mut lines = vec![
        "# Range Strata Binary Integrity Report".to_owned(),
        String::new(),
        format!("Generated: {}", report.generated_at),
        format!("Mode: {}", mode_label(report.mode)),
        format!("Directory: `{}`", report.directory),
    ];
    if let Some(source) = &report.source_db_path {
        lines.push(format!("Source DB: `{source}`"));
    }
    lines.extend([
        String::new(),
        "## Summary".to_owned(),
        markdown_table(
            &["Metric", "Value"],
            vec![
                vec![
                    "Dimensions".to_owned(),
                    report.totals.dimensions.to_string(),
                ],
                vec![
                    "Manifest OK".to_owned(),
                    yes_no(report.totals.manifest_ok).to_owned(),
                ],
                vec![
                    "Catalog OK".to_owned(),
                    yes_no(report.totals.catalog_ok).to_owned(),
                ],
                vec![
                    "Index Files OK".to_owned(),
                    format!(
                        "{} / {}",
                        report.totals.index_files_ok,
                        report.totals.index_files_ok + report.totals.index_files_failed
                    ),
                ],
                vec![
                    "Pack Files OK".to_owned(),
                    format!(
                        "{} / {}",
                        report.totals.pack_files_ok,
                        report.totals.pack_files_ok + report.totals.pack_files_failed
                    ),
                ],
                vec![
                    "Index-Pack Cross Failures".to_owned(),
                    report.totals.index_pack_cross_failures.to_string(),
                ],
            ],
        ),
        String::new(),
        "## Precision Policy".to_owned(),
        markdown_table(
            &["Parameter", "Value"],
            vec![
                vec![
                    "numeric fields".to_owned(),
                    report.precision_policy.numeric_fields.to_owned(),
                ],
                vec![
                    "nullable handEV".to_owned(),
                    report.precision_policy.nullable_hand_ev.to_owned(),
                ],
                vec![
                    "legacy frequency tolerance".to_owned(),
                    report.tolerances.frequency.to_string(),
                ],
                vec![
                    "legacy handEV tolerance".to_owned(),
                    report.tolerances.hand_ev.to_string(),
                ],
            ],
        ),
    ]);

    if !report.dimensions.is_empty() {
        lines.extend([
            String::new(),
            "## Dimensions".to_owned(),
            markdown_table(
                &[
                    "Dimension",
                    "Checked",
                    "Index Records",
                    "Header Failures",
                    "Index-Pack Failures",
                    "Cross Records",
                    "Cross Failures",
                ],
                report
                    .dimensions
                    .iter()
                    .map(|dimension| {
                        vec![
                            format!(
                                "{}:{}max:{}BB",
                                dimension.strategy, dimension.player_count, dimension.depth_bb
                            ),
                            yes_no(dimension.checked).to_owned(),
                            dimension.index_records.to_string(),
                            dimension.header_failures.to_string(),
                            dimension.index_pack_cross_failures.to_string(),
                            dimension
                                .source_cross_records
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "-".to_owned()),
                            dimension
                                .source_cross_failures
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "-".to_owned()),
                        ]
                    })
                    .collect(),
            ),
        ]);
    }

    lines.push(String::new());
    lines.push("## Failures".to_owned());
    if report.failures.is_empty() {
        lines.push("None. All checks passed.".to_owned());
    } else {
        lines.push(markdown_table(
            &["Layer", "Check", "Reason", "Message"],
            report
                .failures
                .iter()
                .take(100)
                .map(|failure| {
                    vec![
                        layer_label(failure.layer).to_owned(),
                        failure.check.clone(),
                        failure.reason.clone(),
                        truncate(&failure.message, 120),
                    ]
                })
                .collect(),
        ));
    }

    lines.push(String::new());
    lines.push("## Repair Suggestions".to_owned());
    for suggestion in &report.repair_suggestions {
        lines.push(format!("- {suggestion}"));
    }

    lines.join("\n")
}

fn calculate_totals(
    dimensions: &[DimensionVerifyDetail],
    failures: &[VerifyFailure],
) -> VerifyTotals {
    let manifest_failed = failures.iter().any(|failure| {
        failure.layer == VerifyLayer::Manifest
            || (failure.layer == VerifyLayer::FileExistence && failure.check == "manifest.json")
    });
    let catalog_failed = failures.iter().any(|failure| {
        failure.layer == VerifyLayer::Catalog
            || (failure.layer == VerifyLayer::FileExistence && failure.check == "meta.db")
    });
    let checked_dimensions = dimensions
        .iter()
        .filter(|dimension| dimension.checked)
        .collect::<Vec<_>>();
    let index_files_ok = checked_dimensions
        .iter()
        .filter(|dimension| {
            dimension.header_failures == 0 && dimension.index_pack_cross_failures == 0
        })
        .count();
    let pack_files_failed = checked_dimensions
        .iter()
        .filter(|dimension| {
            failures.iter().any(|failure| {
                failure.layer == VerifyLayer::PackHeader
                    && failure.check == dimension_check_key(dimension)
            })
        })
        .count();
    VerifyTotals {
        dimensions: dimensions.len(),
        manifest_ok: !manifest_failed,
        catalog_ok: !catalog_failed,
        index_files_ok,
        index_files_failed: checked_dimensions.len().saturating_sub(index_files_ok),
        pack_files_ok: checked_dimensions.len().saturating_sub(pack_files_failed),
        pack_files_failed,
        index_pack_cross_failures: failures
            .iter()
            .filter(|failure| failure.layer == VerifyLayer::IndexPackCross)
            .count(),
        checked_source_records: None,
        failed_source_records: None,
        extra_binary_records: None,
    }
}

fn repair_suggestions(failures: &[VerifyFailure]) -> Vec<String> {
    if failures.is_empty() {
        return vec!["All checks passed - no repairs needed.".to_owned()];
    }

    let mut suggestions = Vec::new();
    if failures
        .iter()
        .any(|failure| failure.reason == "MISSING_FILE")
    {
        suggestions
            .push("Some expected files are missing. Rebuild the affected dimensions.".to_owned());
    }
    if failures
        .iter()
        .any(|failure| failure.layer == VerifyLayer::Manifest)
    {
        suggestions.push(
            "manifest.json is corrupt or incompatible. Rebuild the output directory.".to_owned(),
        );
    }
    if failures
        .iter()
        .any(|failure| failure.layer == VerifyLayer::Catalog)
    {
        suggestions
            .push("meta.db catalog integrity failed. Rebuild the output directory.".to_owned());
    }
    if failures.iter().any(|failure| {
        failure.layer == VerifyLayer::IndexHeader || failure.layer == VerifyLayer::IndexPackCross
    }) {
        suggestions.push(
            "Index or index-pack consistency failed. Regenerate the affected .idx/.bin files."
                .to_owned(),
        );
    }
    if failures
        .iter()
        .any(|failure| failure.layer == VerifyLayer::PackHeader)
    {
        suggestions
            .push("Pack header validation failed. Regenerate the affected .bin files.".to_owned());
    }
    if failures
        .iter()
        .any(|failure| failure.layer == VerifyLayer::SourceCross)
    {
        suggestions.push("Source cross-validation failed. Verify the source SQLite DB matches the binary build input.".to_owned());
    }
    if suggestions.is_empty() {
        suggestions.push("Verify the build was run with compatible software versions.".to_owned());
    }
    suggestions
}

fn markdown_table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
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
    output.trim_end().to_owned()
}

fn dimension_check_key(dimension: &DimensionVerifyDetail) -> String {
    format!(
        "dimension:{}:{}max:{}BB",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    )
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    value.chars().take(max_chars).collect()
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "YES"
    } else {
        "NO"
    }
}

fn mode_label(mode: VerifyMode) -> &'static str {
    match mode {
        VerifyMode::Standalone => "standalone",
        VerifyMode::Cross => "cross",
    }
}

fn layer_label(layer: VerifyLayer) -> &'static str {
    match layer {
        VerifyLayer::FileExistence => "file-existence",
        VerifyLayer::Manifest => "manifest",
        VerifyLayer::Catalog => "catalog",
        VerifyLayer::IndexHeader => "index-header",
        VerifyLayer::PackHeader => "pack-header",
        VerifyLayer::IndexPackCross => "index-pack-cross",
        VerifyLayer::ConcreteIndexConsistency => "concrete-index-consistency",
        VerifyLayer::SourceCross => "source-cross",
    }
}

fn utc_now_iso8601() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}
