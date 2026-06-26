use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::benchmark::benchmark_models::{range_table_name, HandBenchmarkItem};
use crate::domain::dimension::quote_identifier;
use crate::errors::AppError;
use crate::query::QueryService;
use crate::storage::sqlite::{Connection, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResultVerificationSummary {
    pub sample_size: usize,
    pub match_count: u64,
    pub mismatch_count: u64,
    pub error_count: u64,
    pub mismatches: Vec<String>,
    pub errors: Vec<String>,
}

impl ResultVerificationSummary {
    pub fn has_errors(&self) -> bool {
        self.mismatch_count > 0 || self.error_count > 0
    }

    pub fn notes(&self) -> Vec<String> {
        let mut notes = vec![format!(
            "Result verification (sample size={}): {} match, {} mismatch, {} errors.",
            self.sample_size, self.match_count, self.mismatch_count, self.error_count
        )];
        if !self.mismatches.is_empty() {
            notes.push(format!(
                "First {} mismatches: {}",
                self.mismatches.len(),
                self.mismatches.join("; ")
            ));
        }
        if !self.errors.is_empty() {
            notes.push(format!(
                "First {} verification errors: {}",
                self.errors.len(),
                self.errors.join("; ")
            ));
        }
        notes
    }
}

pub fn verify_benchmark_results(
    source_db: &Path,
    service: &QueryService,
    hand_queries: &[HandBenchmarkItem],
) -> Result<ResultVerificationSummary, AppError> {
    let sample_size = hand_queries.len().min(100);
    let connection = match Connection::open(source_db, true) {
        Ok(connection) => connection,
        Err(error) => {
            return Ok(ResultVerificationSummary {
                sample_size,
                match_count: 0,
                mismatch_count: 0,
                error_count: 1,
                mismatches: Vec::new(),
                errors: vec![format!("Could not open source SQLite: {error}")],
            });
        }
    };

    let mut match_count = 0_u64;
    let mut mismatch_count = 0_u64;
    let mut error_count = 0_u64;
    let mut mismatches = Vec::new();
    let mut errors = Vec::new();

    for item in hand_queries.iter().take(sample_size) {
        let context = format!(
            "{}_{}max_{}BB / {} / {}",
            item.strategy, item.player_count, item.depth_bb, item.concrete_line_id, item.hole_cards
        );

        let sqlite_count = match source_action_count(&connection, item) {
            Ok(count) => count,
            Err(error) => {
                error_count += 1;
                push_capped(
                    &mut errors,
                    format!("{context}: source SQLite error: {}", error.message()),
                );
                continue;
            }
        };

        let binary_count =
            match service.query(&item.dimension(), item.concrete_line_id, &item.hole_cards) {
                Ok(result) => result.actions.len(),
                Err(error) => {
                    error_count += 1;
                    push_capped(
                        &mut errors,
                        format!("{context}: binary query error: {}", error.message()),
                    );
                    continue;
                }
            };

        if sqlite_count == binary_count {
            match_count += 1;
        } else {
            mismatch_count += 1;
            push_capped(
                &mut mismatches,
                format!("{context}: SQLite={sqlite_count}, rangeStrata={binary_count}"),
            );
        }
    }

    Ok(ResultVerificationSummary {
        sample_size,
        match_count,
        mismatch_count,
        error_count,
        mismatches,
        errors,
    })
}

fn source_action_count(
    connection: &Connection,
    item: &HandBenchmarkItem,
) -> Result<usize, AppError> {
    let table = quote_identifier(&range_table_name(&item.dimension()))?;
    let sql = format!(
        "SELECT COUNT(*)
         FROM {table}
         WHERE concrete_line_id = ?1 AND hole_cards = ?2"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[
        Value::from(item.concrete_line_id),
        Value::from(item.hole_cards.as_str()),
    ])?;
    if statement.step_row()? {
        usize::try_from(statement.column_i64(0)).map_err(|_| {
            AppError::invalid_format("Source SQLite action count is outside usize range")
        })
    } else {
        Ok(0)
    }
}

fn push_capped(items: &mut Vec<String>, value: String) {
    if items.len() < 10 {
        items.push(value);
    }
}
