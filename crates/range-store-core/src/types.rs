//! Binary format type definitions.
//!
//! All integers are little-endian as stored on disk.

// ── .idx file constants ──

pub const IDX_MAGIC: &[u8; 4] = b"PFXI";
pub const IDX_HEADER_SIZE: usize = 16;
pub const IDX_RECORD_SIZE: usize = 22;

/// Parsed .idx file header.
#[derive(Debug, Clone)]
pub struct IdxHeader {
    pub record_count: u32,
}

/// A single .idx record that maps a concreteLineId to its .bin payload location.
#[derive(Debug, Clone)]
pub struct IdxRecord {
    pub concrete_line_id: u32,
    pub action_schema_id: u32,
    pub hand_count: u16,
    pub offset: u32,
    pub byte_length: u32,
    pub checksum: u32,
}

// ── .bin file constants ──

pub const PFSP_MAGIC: &[u8; 4] = b"PFSP";
pub const PFSP_HEADER_SIZE: usize = 16;

/// Parsed .bin file header.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PfspHeader {
    pub version: u16,
    pub endian: u8,
    pub float_type: u8,
    pub layout: u8,
    pub compression: u8,
    pub header_size: u16,
}

// -- Public query result types --

/// A decoded cell result for a single action.
#[derive(Debug, Clone)]
pub struct DecodedCellResult {
    pub action_id: u32,
    pub frequency: f64,
    /// handEv is null (None) when the value is NaN or no EV data.
    pub hand_ev: Option<f64>,
}

/// Result of querying a pack for a specific hand.
#[derive(Debug, Clone)]
pub struct PackDecodeResult {
    pub action_schema_id: u32,
    pub cells: Vec<DecodedCellResult>,
}

/// A single request in a batch query.
#[derive(Debug, Clone)]
pub struct BatchQueryRequest {
    pub concrete_line_id: u32,
    pub hand_id: u32,
}
