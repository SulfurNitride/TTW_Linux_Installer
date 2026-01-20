use serde::{Deserialize, Serialize};
use std::fmt;

/// Asset operation types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetOpType {
    /// Copy file from source BSA/location
    Copy = 0,
    /// New file from MPI package
    New = 1,
    /// Apply binary patch (.xd3)
    Patch = 2,
    /// OggEnc2 audio conversion (resample OGG)
    OggEnc2 = 4,
    /// Audio encoding/format conversion
    AudioEnc = 5,
}

impl AssetOpType {
    pub fn from_int(value: i32) -> Option<Self> {
        match value {
            0 => Some(AssetOpType::Copy),
            1 => Some(AssetOpType::New),
            2 => Some(AssetOpType::Patch),
            4 => Some(AssetOpType::OggEnc2),
            5 => Some(AssetOpType::AudioEnc),
            _ => None,
        }
    }
}

/// Represents an installation asset operation
/// Asset format: [Tags, OpType, Params, Status, SourceLoc, TargetLoc, SourcePath, TargetPath?]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    /// Tag bitmask (FO3=1, FNV=2, TTW=512, etc.)
    pub tags: i32,
    /// Operation type: 0=Copy, 1=New, 2=Patch, 4=OggEnc2, 5=AudioEnc
    pub op_type: i32,
    /// Operation parameters (e.g., "-f:24000 -q:5" for audio encoding)
    pub params: String,
    /// Status flags
    pub status: i32,
    /// Source location index (into Locations array)
    pub source_loc: i32,
    /// Target location index (into Locations array)
    pub target_loc: i32,
    /// Source file path (relative to source location)
    pub source_path: String,
    /// Target file path (relative to target location), optional
    pub target_path: Option<String>,
}

impl Asset {
    /// Parse asset from JSON array
    pub fn from_json_array(array: &[serde_json::Value]) -> anyhow::Result<Self> {
        if array.len() < 7 {
            anyhow::bail!("Invalid asset array length: {}", array.len());
        }

        let mut asset = Asset {
            tags: array[0].as_i64().unwrap_or(0) as i32,
            op_type: array[1].as_i64().unwrap_or(0) as i32,
            params: array[2].as_str().unwrap_or("").to_string(),
            status: array[3].as_i64().unwrap_or(0) as i32,
            source_loc: array[4].as_i64().unwrap_or(0) as i32,
            target_loc: array[5].as_i64().unwrap_or(0) as i32,
            source_path: array[6].as_str().unwrap_or("").to_string(),
            target_path: None,
        };

        // TargetPath is optional (some assets only have 7 elements)
        if array.len() > 7 {
            asset.target_path = array[7].as_str().map(|s| s.to_string());
        }

        Ok(asset)
    }

    /// Get the effective target path (target_path or source_path if not specified)
    pub fn effective_target_path(&self) -> &str {
        self.target_path.as_deref().unwrap_or(&self.source_path)
    }

    /// Get the operation type as enum
    pub fn op_type_enum(&self) -> Option<AssetOpType> {
        AssetOpType::from_int(self.op_type)
    }

    /// Parse audio parameters from params string (format: "-key:value -key:value")
    pub fn parse_audio_params(&self) -> std::collections::HashMap<String, String> {
        let mut result = std::collections::HashMap::new();

        for part in self.params.split_whitespace() {
            if part.starts_with('-') && part.contains(':') {
                if let Some(colon_idx) = part.find(':') {
                    let key = &part[1..colon_idx];
                    let value = &part[colon_idx + 1..];
                    result.insert(key.to_string(), value.to_string());
                }
            }
        }

        result
    }
}

impl fmt::Display for Asset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[Op{}] {} -> {}",
            self.op_type,
            self.source_path,
            self.effective_target_path()
        )
    }
}
