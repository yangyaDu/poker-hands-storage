use serde::{Deserialize, Serialize};

use crate::errors::ToolError;

use super::format::{
    ABSTRACT_ACTION_PATHS_DATA_FILE_NAME, ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME,
    DRILL_SCENARIOS_DATA_FILE_NAME, DRILL_SCENARIOS_INDEX_FILE_NAME,
    HAND_STRATEGIES_DATA_FILE_NAME, HAND_STRATEGIES_INDEX_FILE_NAME, HEADER_SIZE,
};

pub const MANIFEST_FILE_NAME: &str = "manifest.json";
pub const ARCHIVE_FORMAT: &str = "POKER_HANDS_PROTO_V3";
pub const ARCHIVE_VERSION: u32 = 3;
pub const PAYLOAD_SCHEMA: &str = "poker.hands.storage.v3";
pub const PREFLOP_HAND_ENCODING: &str = "HAND_ENCODING_PREFLOP";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveManifest {
    pub format: String,
    pub version: u32,
    pub payload_schema: String,
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub hand_encoding: String,
    pub complete: bool,
    pub drill_scenarios: DrillScenariosManifest,
    pub abstract_action_paths: AbstractActionPathsManifest,
    pub hand_strategies: HandStrategiesManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestFile {
    pub file_name: String,
    pub size_bytes: u64,
    pub crc32c: u32,
    pub primary_count: u64,
    pub secondary_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DrillScenariosManifest {
    pub data: ManifestFile,
    pub index: ManifestFile,
    pub page_count: u64,
    pub drill_count: u64,
    pub hash_record_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AbstractActionPathsManifest {
    pub data: ManifestFile,
    pub index: ManifestFile,
    pub page_count: u64,
    pub abstract_path_count: u64,
    pub concrete_path_count: u64,
    pub abstract_hash_record_count: u64,
    pub concrete_hash_record_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandStrategiesManifest {
    pub data: ManifestFile,
    pub index: ManifestFile,
    pub record_count: u64,
}

impl ArchiveManifest {
    pub fn validate(&self) -> Result<(), ToolError> {
        if self.format != ARCHIVE_FORMAT {
            return Err(invalid_manifest(format!(
                "format must be {ARCHIVE_FORMAT}, got {}",
                self.format
            )));
        }
        if self.version != ARCHIVE_VERSION {
            return Err(invalid_manifest(format!(
                "version must be {ARCHIVE_VERSION}, got {}",
                self.version
            )));
        }
        if self.payload_schema != PAYLOAD_SCHEMA {
            return Err(invalid_manifest(format!(
                "payloadSchema must be {PAYLOAD_SCHEMA}, got {}",
                self.payload_schema
            )));
        }
        if self.strategy.trim().is_empty() {
            return Err(invalid_manifest("strategy must not be empty"));
        }
        if self.player_count == 0 || self.depth_bb == 0 {
            return Err(invalid_manifest(
                "playerCount and depthBb must both be positive",
            ));
        }
        if self.hand_encoding != PREFLOP_HAND_ENCODING {
            return Err(invalid_manifest(format!(
                "handEncoding must be {PREFLOP_HAND_ENCODING}"
            )));
        }
        if !self.complete {
            return Err(invalid_manifest("manifest is not marked complete"));
        }

        validate_file(
            &self.drill_scenarios.data,
            DRILL_SCENARIOS_DATA_FILE_NAME,
            self.drill_scenarios.page_count,
            self.drill_scenarios.drill_count,
        )?;
        validate_file(
            &self.drill_scenarios.index,
            DRILL_SCENARIOS_INDEX_FILE_NAME,
            self.drill_scenarios.page_count,
            self.drill_scenarios.hash_record_count,
        )?;
        validate_file(
            &self.abstract_action_paths.data,
            ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
            self.abstract_action_paths.page_count,
            self.abstract_action_paths.abstract_path_count,
        )?;
        validate_file(
            &self.abstract_action_paths.index,
            ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME,
            self.abstract_action_paths.page_count,
            self.abstract_action_paths
                .abstract_hash_record_count
                .checked_add(self.abstract_action_paths.concrete_hash_record_count)
                .ok_or_else(|| invalid_manifest("action path hash record count overflow"))?,
        )?;
        validate_file(
            &self.hand_strategies.data,
            HAND_STRATEGIES_DATA_FILE_NAME,
            self.hand_strategies.record_count,
            0,
        )?;
        validate_file(
            &self.hand_strategies.index,
            HAND_STRATEGIES_INDEX_FILE_NAME,
            self.hand_strategies.record_count,
            0,
        )?;

        if self.drill_scenarios.page_count == 0 || self.drill_scenarios.drill_count == 0 {
            return Err(invalid_manifest(
                "drill scenarios dataset must not be empty",
            ));
        }
        if self.drill_scenarios.hash_record_count != self.drill_scenarios.drill_count {
            return Err(invalid_manifest(
                "drill hash record count must equal drill count",
            ));
        }
        if self.abstract_action_paths.page_count == 0
            || self.abstract_action_paths.abstract_path_count == 0
            || self.abstract_action_paths.concrete_path_count == 0
        {
            return Err(invalid_manifest(
                "abstract/concrete action path dataset must not be empty",
            ));
        }
        if self.abstract_action_paths.abstract_hash_record_count
            != self.abstract_action_paths.abstract_path_count
            || self.abstract_action_paths.concrete_hash_record_count
                != self.abstract_action_paths.concrete_path_count
        {
            return Err(invalid_manifest(
                "action path hash record counts must match path counts",
            ));
        }
        if self.hand_strategies.record_count == 0 {
            return Err(invalid_manifest(
                "hand strategies dataset must not be empty",
            ));
        }
        if self.hand_strategies.record_count != self.abstract_action_paths.concrete_path_count {
            return Err(invalid_manifest(
                "hand strategy record count must equal concrete action path count",
            ));
        }
        Ok(())
    }
}

fn validate_file(
    file: &ManifestFile,
    expected_name: &str,
    expected_primary_count: u64,
    expected_secondary_count: u64,
) -> Result<(), ToolError> {
    if file.file_name != expected_name {
        return Err(invalid_manifest(format!(
            "fileName must be {expected_name}, got {}",
            file.file_name
        )));
    }
    if file.size_bytes < HEADER_SIZE as u64 {
        return Err(invalid_manifest(format!(
            "{} is smaller than the V3 header",
            file.file_name
        )));
    }
    if file.primary_count != expected_primary_count
        || file.secondary_count != expected_secondary_count
    {
        return Err(invalid_manifest(format!(
            "{} counts do not match dataset totals",
            file.file_name
        )));
    }
    Ok(())
}

fn invalid_manifest(message: impl Into<String>) -> ToolError {
    ToolError::new("INVALID_V3_MANIFEST", message)
}
