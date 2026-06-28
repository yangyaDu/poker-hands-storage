use crate::errors::ToolError;
use range_store_core::dimension::DimensionRef;

pub fn next_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str, ToolError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| ToolError::invalid_argument("Missing option value"))
}

pub fn parse_usize(name: &str, value: &str) -> Result<usize, ToolError> {
    value
        .parse()
        .map_err(|_| ToolError::invalid_argument(format!("{name} must be an integer")))
}

pub fn parse_u32(name: &str, value: &str) -> Result<u32, ToolError> {
    value
        .parse()
        .map_err(|_| ToolError::invalid_argument(format!("{name} must be an integer")))
}

pub fn parse_u64(name: &str, value: &str) -> Result<u64, ToolError> {
    value
        .parse()
        .map_err(|_| ToolError::invalid_argument(format!("{name} must be an integer")))
}

pub fn parse_usize_list(name: &str, value: &str) -> Result<Vec<usize>, ToolError> {
    let mut parsed = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        parsed.push(parse_usize(name, part)?.max(1));
    }
    if parsed.is_empty() {
        return Err(ToolError::invalid_argument(format!(
            "{name} must contain at least one integer"
        )));
    }
    Ok(parsed)
}

pub fn parse_requested_dimension(value: &str) -> Result<DimensionRef, ToolError> {
    if let Some(dimension) = parse_colon_dimension(value) {
        return Ok(dimension);
    }
    if let Some(dimension) = parse_table_dimension(value) {
        return Ok(dimension);
    }
    Err(ToolError::invalid_argument(format!(
        "Invalid --dimension value: {value}. Use default:6:100 or default_6max_100BB."
    )))
}

fn parse_colon_dimension(value: &str) -> Option<DimensionRef> {
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    let player_count = parts[1].strip_suffix("max").unwrap_or(parts[1]);
    let depth_bb = parts[2].strip_suffix("BB").unwrap_or(parts[2]);
    Some(DimensionRef::new(
        parts[0],
        player_count.parse().ok()?,
        depth_bb.parse().ok()?,
    ))
}

fn parse_table_dimension(value: &str) -> Option<DimensionRef> {
    let value = value.strip_suffix("BB")?;
    let (left, depth_bb) = value.rsplit_once('_')?;
    let (strategy, player_count) = left.rsplit_once('_')?;
    let player_count = player_count.strip_suffix("max")?;
    Some(DimensionRef::new(
        strategy,
        player_count.parse().ok()?,
        depth_bb.parse().ok()?,
    ))
}
