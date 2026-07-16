use crate::errors::ToolError;

pub const FORMAT_VERSION: u16 = 3;
pub const HEADER_SIZE: usize = 32;
pub const SECTION_DESCRIPTOR_SIZE: usize = 32;
pub const PAYLOAD_LOCATOR_SIZE: usize = 16;
pub const HASH_LOCATOR_SIZE: usize = 24;
pub const NO_VALUE_INDEX: u32 = u32::MAX;

pub const DRILL_SCENARIOS_DATA_FILE_NAME: &str = "drill-scenarios.pb";
pub const DRILL_SCENARIOS_INDEX_FILE_NAME: &str = "drill-scenarios.idx";
pub const ABSTRACT_ACTION_PATHS_DATA_FILE_NAME: &str = "abstract-action-paths.pb";
pub const ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME: &str = "abstract-action-paths.idx";
pub const HAND_STRATEGIES_DATA_FILE_NAME: &str = "hand-strategies.pb";
pub const HAND_STRATEGIES_INDEX_FILE_NAME: &str = "hand-strategies.idx";

const DRILL_SCENARIOS_DATA_MAGIC: [u8; 4] = *b"V3DD";
const DRILL_SCENARIOS_INDEX_MAGIC: [u8; 4] = *b"V3DI";
const ABSTRACT_ACTION_PATHS_DATA_MAGIC: [u8; 4] = *b"V3AD";
const ABSTRACT_ACTION_PATHS_INDEX_MAGIC: [u8; 4] = *b"V3AI";
const HAND_STRATEGIES_DATA_MAGIC: [u8; 4] = *b"V3HD";
const HAND_STRATEGIES_INDEX_MAGIC: [u8; 4] = *b"V3HI";

const FNV1A_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV1A_PRIME: u64 = 0x00000100000001b3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    DrillScenariosData,
    DrillScenariosIndex,
    AbstractActionPathsData,
    AbstractActionPathsIndex,
    HandStrategiesData,
    HandStrategiesIndex,
}

impl FileKind {
    pub const ALL: [Self; 6] = [
        Self::DrillScenariosData,
        Self::DrillScenariosIndex,
        Self::AbstractActionPathsData,
        Self::AbstractActionPathsIndex,
        Self::HandStrategiesData,
        Self::HandStrategiesIndex,
    ];

    pub const fn magic(self) -> [u8; 4] {
        match self {
            Self::DrillScenariosData => DRILL_SCENARIOS_DATA_MAGIC,
            Self::DrillScenariosIndex => DRILL_SCENARIOS_INDEX_MAGIC,
            Self::AbstractActionPathsData => ABSTRACT_ACTION_PATHS_DATA_MAGIC,
            Self::AbstractActionPathsIndex => ABSTRACT_ACTION_PATHS_INDEX_MAGIC,
            Self::HandStrategiesData => HAND_STRATEGIES_DATA_MAGIC,
            Self::HandStrategiesIndex => HAND_STRATEGIES_INDEX_MAGIC,
        }
    }

    pub const fn file_name(self) -> &'static str {
        match self {
            Self::DrillScenariosData => DRILL_SCENARIOS_DATA_FILE_NAME,
            Self::DrillScenariosIndex => DRILL_SCENARIOS_INDEX_FILE_NAME,
            Self::AbstractActionPathsData => ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
            Self::AbstractActionPathsIndex => ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME,
            Self::HandStrategiesData => HAND_STRATEGIES_DATA_FILE_NAME,
            Self::HandStrategiesIndex => HAND_STRATEGIES_INDEX_FILE_NAME,
        }
    }

    pub const fn is_index(self) -> bool {
        matches!(
            self,
            Self::DrillScenariosIndex | Self::AbstractActionPathsIndex | Self::HandStrategiesIndex
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHeader {
    pub kind: FileKind,
    pub primary_count: u64,
    pub secondary_count: u64,
    pub section_count: u32,
    pub flags: u32,
}

impl FileHeader {
    pub const fn new(
        kind: FileKind,
        primary_count: u64,
        secondary_count: u64,
        section_count: u32,
    ) -> Self {
        Self {
            kind,
            primary_count,
            secondary_count,
            section_count,
            flags: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum SectionKind {
    PageLocators = 1,
    PrimaryHashLocators = 2,
    SecondaryHashLocators = 3,
    PayloadLocators = 4,
}

impl TryFrom<u16> for SectionKind {
    type Error = ToolError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::PageLocators),
            2 => Ok(Self::PrimaryHashLocators),
            3 => Ok(Self::SecondaryHashLocators),
            4 => Ok(Self::PayloadLocators),
            _ => Err(ToolError::invalid_format(format!(
                "Unknown V3 index section kind {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionDescriptor {
    pub kind: SectionKind,
    pub record_size: u16,
    pub offset: u64,
    pub record_count: u64,
    pub byte_length: u64,
}

impl SectionDescriptor {
    pub fn new(
        kind: SectionKind,
        record_size: u16,
        offset: u64,
        record_count: u64,
    ) -> Result<Self, ToolError> {
        let byte_length = record_count
            .checked_mul(u64::from(record_size))
            .ok_or_else(|| ToolError::invalid_format("V3 index section byte length overflow"))?;
        Ok(Self {
            kind,
            record_size,
            offset,
            record_count,
            byte_length,
        })
    }

    pub fn end(self) -> Result<u64, ToolError> {
        self.offset
            .checked_add(self.byte_length)
            .ok_or_else(|| ToolError::invalid_format("V3 index section end overflow"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadLocator {
    pub offset: u64,
    pub byte_length: u32,
    pub crc32c: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashLocator {
    pub hash: u64,
    pub page_id: u32,
    pub entry_index: u32,
    pub value_index: u32,
    pub reserved: u32,
}

impl HashLocator {
    pub const fn entry(hash: u64, page_id: u32, entry_index: u32) -> Self {
        Self {
            hash,
            page_id,
            entry_index,
            value_index: NO_VALUE_INDEX,
            reserved: 0,
        }
    }

    pub const fn value(hash: u64, page_id: u32, entry_index: u32, value_index: u32) -> Self {
        Self {
            hash,
            page_id,
            entry_index,
            value_index,
            reserved: 0,
        }
    }
}

pub fn encode_header(header: FileHeader) -> [u8; HEADER_SIZE] {
    let mut encoded = [0u8; HEADER_SIZE];
    encoded[0..4].copy_from_slice(&header.kind.magic());
    encoded[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    encoded[6..8].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
    encoded[8..16].copy_from_slice(&header.primary_count.to_le_bytes());
    encoded[16..24].copy_from_slice(&header.secondary_count.to_le_bytes());
    encoded[24..28].copy_from_slice(&header.section_count.to_le_bytes());
    encoded[28..32].copy_from_slice(&header.flags.to_le_bytes());
    encoded
}

pub fn decode_header(bytes: &[u8], expected_kind: FileKind) -> Result<FileHeader, ToolError> {
    let encoded = bytes
        .get(..HEADER_SIZE)
        .ok_or_else(|| ToolError::invalid_format("V3 file header is truncated"))?;
    if encoded[0..4] != expected_kind.magic() {
        return Err(ToolError::invalid_format(format!(
            "V3 file magic does not match {}",
            expected_kind.file_name()
        )));
    }
    let version = u16::from_le_bytes(encoded[4..6].try_into().expect("V3 version"));
    if version != FORMAT_VERSION {
        return Err(ToolError::invalid_format(format!(
            "Unsupported V3 file format version {version}"
        )));
    }
    let header_size = u16::from_le_bytes(encoded[6..8].try_into().expect("V3 header size"));
    if usize::from(header_size) != HEADER_SIZE {
        return Err(ToolError::invalid_format(format!(
            "Invalid V3 file header size {header_size}"
        )));
    }
    let header = FileHeader {
        kind: expected_kind,
        primary_count: u64::from_le_bytes(encoded[8..16].try_into().expect("V3 primary count")),
        secondary_count: u64::from_le_bytes(
            encoded[16..24].try_into().expect("V3 secondary count"),
        ),
        section_count: u32::from_le_bytes(encoded[24..28].try_into().expect("V3 section count")),
        flags: u32::from_le_bytes(encoded[28..32].try_into().expect("V3 flags")),
    };
    if header.flags != 0 {
        return Err(ToolError::invalid_format(format!(
            "Unsupported V3 file flags {}",
            header.flags
        )));
    }
    if !expected_kind.is_index() && header.section_count != 0 {
        return Err(ToolError::invalid_format(
            "V3 protobuf data files cannot contain index sections",
        ));
    }
    Ok(header)
}

pub fn encode_section_descriptor(descriptor: SectionDescriptor) -> [u8; SECTION_DESCRIPTOR_SIZE] {
    let mut encoded = [0u8; SECTION_DESCRIPTOR_SIZE];
    encoded[0..2].copy_from_slice(&(descriptor.kind as u16).to_le_bytes());
    encoded[2..4].copy_from_slice(&descriptor.record_size.to_le_bytes());
    encoded[8..16].copy_from_slice(&descriptor.offset.to_le_bytes());
    encoded[16..24].copy_from_slice(&descriptor.record_count.to_le_bytes());
    encoded[24..32].copy_from_slice(&descriptor.byte_length.to_le_bytes());
    encoded
}

pub fn decode_section_descriptor(bytes: &[u8]) -> Result<SectionDescriptor, ToolError> {
    let encoded = bytes
        .get(..SECTION_DESCRIPTOR_SIZE)
        .ok_or_else(|| ToolError::invalid_format("V3 section descriptor is truncated"))?;
    if encoded[4..8] != [0, 0, 0, 0] {
        return Err(ToolError::invalid_format(
            "V3 section descriptor reserved bytes must be zero",
        ));
    }
    let descriptor = SectionDescriptor {
        kind: SectionKind::try_from(u16::from_le_bytes(
            encoded[0..2].try_into().expect("V3 section kind"),
        ))?,
        record_size: u16::from_le_bytes(encoded[2..4].try_into().expect("V3 section record size")),
        offset: u64::from_le_bytes(encoded[8..16].try_into().expect("V3 section offset")),
        record_count: u64::from_le_bytes(
            encoded[16..24].try_into().expect("V3 section record count"),
        ),
        byte_length: u64::from_le_bytes(
            encoded[24..32].try_into().expect("V3 section byte length"),
        ),
    };
    if descriptor.record_size == 0 {
        return Err(ToolError::invalid_format(
            "V3 section record size must be non-zero",
        ));
    }
    let expected_length = descriptor
        .record_count
        .checked_mul(u64::from(descriptor.record_size))
        .ok_or_else(|| ToolError::invalid_format("V3 index section byte length overflow"))?;
    if descriptor.byte_length != expected_length {
        return Err(ToolError::invalid_format(
            "V3 section byte length does not match record count and size",
        ));
    }
    descriptor.end()?;
    Ok(descriptor)
}

pub fn encode_payload_locator(locator: PayloadLocator) -> [u8; PAYLOAD_LOCATOR_SIZE] {
    let mut encoded = [0u8; PAYLOAD_LOCATOR_SIZE];
    encoded[0..8].copy_from_slice(&locator.offset.to_le_bytes());
    encoded[8..12].copy_from_slice(&locator.byte_length.to_le_bytes());
    encoded[12..16].copy_from_slice(&locator.crc32c.to_le_bytes());
    encoded
}

pub fn decode_payload_locator(bytes: &[u8]) -> Result<PayloadLocator, ToolError> {
    let encoded = bytes
        .get(..PAYLOAD_LOCATOR_SIZE)
        .ok_or_else(|| ToolError::invalid_format("V3 payload locator is truncated"))?;
    Ok(PayloadLocator {
        offset: u64::from_le_bytes(encoded[0..8].try_into().expect("V3 payload offset")),
        byte_length: u32::from_le_bytes(encoded[8..12].try_into().expect("V3 payload byte length")),
        crc32c: u32::from_le_bytes(encoded[12..16].try_into().expect("V3 payload CRC32C")),
    })
}

pub fn encode_hash_locator(locator: HashLocator) -> [u8; HASH_LOCATOR_SIZE] {
    let mut encoded = [0u8; HASH_LOCATOR_SIZE];
    encoded[0..8].copy_from_slice(&locator.hash.to_le_bytes());
    encoded[8..12].copy_from_slice(&locator.page_id.to_le_bytes());
    encoded[12..16].copy_from_slice(&locator.entry_index.to_le_bytes());
    encoded[16..20].copy_from_slice(&locator.value_index.to_le_bytes());
    encoded[20..24].copy_from_slice(&locator.reserved.to_le_bytes());
    encoded
}

pub fn decode_hash_locator(bytes: &[u8]) -> Result<HashLocator, ToolError> {
    let encoded = bytes
        .get(..HASH_LOCATOR_SIZE)
        .ok_or_else(|| ToolError::invalid_format("V3 hash locator is truncated"))?;
    let locator = HashLocator {
        hash: u64::from_le_bytes(encoded[0..8].try_into().expect("V3 hash")),
        page_id: u32::from_le_bytes(encoded[8..12].try_into().expect("V3 page id")),
        entry_index: u32::from_le_bytes(encoded[12..16].try_into().expect("V3 entry index")),
        value_index: u32::from_le_bytes(encoded[16..20].try_into().expect("V3 value index")),
        reserved: u32::from_le_bytes(encoded[20..24].try_into().expect("V3 reserved")),
    };
    if locator.reserved != 0 {
        return Err(ToolError::invalid_format(
            "V3 hash locator reserved bytes must be zero",
        ));
    }
    Ok(locator)
}

pub fn stable_hash64(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(FNV1A_OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV1A_PRIME)
        })
}

pub fn equal_hash_range(locators: &[HashLocator], hash: u64) -> std::ops::Range<usize> {
    let start = locators.partition_point(|locator| locator.hash < hash);
    let end = locators.partition_point(|locator| locator.hash <= hash);
    start..end
}
