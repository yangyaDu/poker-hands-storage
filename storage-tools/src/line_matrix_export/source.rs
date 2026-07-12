use range_store_core::dimension::{quote_identifier, DimensionSpec};
use range_store_core::sqlite::{Connection, Value};

use crate::errors::ToolError;

use super::ConcreteLineSelector;

#[derive(Debug, Clone)]
pub(crate) struct ResolvedLine {
    pub concrete_line_id: u32,
    pub abstract_line: String,
    pub concrete_line: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceRow {
    pub hole_cards: String,
    pub action_name: String,
    pub action_size: f64,
    pub amount_bb: f64,
    pub frequency: f64,
    pub hand_ev: Option<f64>,
}

pub(crate) fn load_all_lines(
    connection: &Connection,
    dimension: &DimensionSpec,
) -> Result<Vec<ResolvedLine>, ToolError> {
    let table = quote_identifier(&dimension.concrete_table())?;
    let mut statement = connection.prepare(&format!(
        "SELECT id, abstract_line, concrete_line FROM {table} ORDER BY id"
    ))?;
    statement.start(&[])?;

    let mut lines = Vec::new();
    while statement.step_row()? {
        lines.push(ResolvedLine {
            concrete_line_id: statement.column_u32(0)?,
            abstract_line: statement.column_text(1)?,
            concrete_line: statement.column_text(2)?,
        });
    }
    if lines.is_empty() {
        return Err(ToolError::new(
            "LINE_MATRIX_ARCHIVE_EMPTY",
            "The selected dimension has no concrete lines",
        ));
    }
    Ok(lines)
}

pub(crate) fn resolve_line(
    connection: &Connection,
    dimension: &DimensionSpec,
    selector: &ConcreteLineSelector,
) -> Result<ResolvedLine, ToolError> {
    let table = quote_identifier(&dimension.concrete_table())?;
    let (sql, values) = match selector {
        ConcreteLineSelector::Id(concrete_line_id) => (
            format!(
                "SELECT id, abstract_line, concrete_line FROM {table} WHERE id = ?1 ORDER BY id"
            ),
            vec![Value::from(*concrete_line_id)],
        ),
        ConcreteLineSelector::Text {
            concrete_line,
            abstract_line: Some(abstract_line),
        } => (
            format!(
                "SELECT id, abstract_line, concrete_line FROM {table} \
                 WHERE concrete_line = ?1 AND abstract_line = ?2 ORDER BY id"
            ),
            vec![
                Value::from(concrete_line.as_str()),
                Value::from(abstract_line.as_str()),
            ],
        ),
        ConcreteLineSelector::Text {
            concrete_line,
            abstract_line: None,
        } => (
            format!(
                "SELECT id, abstract_line, concrete_line FROM {table} \
                 WHERE concrete_line = ?1 ORDER BY id"
            ),
            vec![Value::from(concrete_line.as_str())],
        ),
    };

    let mut statement = connection.prepare(&sql)?;
    statement.start(&values)?;
    let mut matches = Vec::new();
    while statement.step_row()? {
        matches.push(ResolvedLine {
            concrete_line_id: statement.column_u32(0)?,
            abstract_line: statement.column_text(1)?,
            concrete_line: statement.column_text(2)?,
        });
    }

    match matches.len() {
        0 => Err(ToolError::new(
            "LINE_NOT_FOUND",
            format!("No concrete line matched {selector}"),
        )),
        1 => Ok(matches.remove(0)),
        _ => {
            let ids = matches
                .iter()
                .map(|line| line.concrete_line_id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Err(ToolError::new(
                "AMBIGUOUS_CONCRETE_LINE",
                format!(
                    "{selector} matched multiple concrete lines ({ids}); add --abstract-line or use --concrete-line-id"
                ),
            ))
        }
    }
}

pub(crate) fn load_rows(
    connection: &Connection,
    dimension: &DimensionSpec,
    concrete_line_id: u32,
) -> Result<Vec<SourceRow>, ToolError> {
    load_rows_matching(connection, dimension, concrete_line_id, false)
}

pub(crate) fn load_rows_with_ev(
    connection: &Connection,
    dimension: &DimensionSpec,
    concrete_line_id: u32,
) -> Result<Vec<SourceRow>, ToolError> {
    load_rows_matching(connection, dimension, concrete_line_id, true)
}

fn load_rows_matching(
    connection: &Connection,
    dimension: &DimensionSpec,
    concrete_line_id: u32,
    require_hand_ev: bool,
) -> Result<Vec<SourceRow>, ToolError> {
    let table = quote_identifier(&dimension.range_table())?;
    let hand_ev_filter = if require_hand_ev {
        " AND hand_ev IS NOT NULL"
    } else {
        ""
    };
    let mut statement = connection.prepare(&format!(
        "SELECT hole_cards, action_name, action_size, amount_bb, frequency, hand_ev \
         FROM {table} WHERE concrete_line_id = ?1{hand_ev_filter} \
         ORDER BY hole_cards, action_name, action_size, amount_bb"
    ))?;
    statement.start(&[Value::from(concrete_line_id)])?;

    let mut rows = Vec::new();
    while statement.step_row()? {
        rows.push(SourceRow {
            hole_cards: statement.column_text(0)?,
            action_name: statement.column_text(1)?,
            action_size: statement.column_f64(2),
            amount_bb: statement.column_f64(3),
            frequency: statement.column_f64(4),
            hand_ev: statement.column_optional_f64(5),
        });
    }
    if rows.is_empty() {
        return Err(ToolError::new(
            "LINE_MATRIX_EMPTY",
            format!(
                "Concrete line {concrete_line_id} has no {}range rows",
                if require_hand_ev { "non-NULL EV " } else { "" }
            ),
        ));
    }
    Ok(rows)
}
