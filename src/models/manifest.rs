use serde::{Deserialize, Serialize};

/// Root manifest structure for TTW installation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TtwManifest {
    pub package: Option<PackageInfo>,
    pub variables: Option<Vec<Vec<Variable>>>,
    pub locations: Option<Vec<Vec<Location>>>,
    pub tags: Option<Vec<Tag>>,
    pub assets: Option<Vec<serde_json::Value>>,
    pub checks: Option<Vec<Check>>,
    pub file_attrs: Option<Vec<FileAttr>>,
    pub post_commands: Option<Vec<PostCommand>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackageInfo {
    pub title: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub home_page: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Variable {
    pub name: Option<String>,
    #[serde(rename = "Type")]
    pub var_type: i32,
    pub value: Option<String>,
    #[serde(default)]
    pub exclude_delimiter: bool,
}

/// Location types:
/// 0 = Directory
/// 1 = BSA source (read from)
/// 2 = BSA creation target (write to)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Location {
    pub name: Option<String>,
    #[serde(rename = "Type")]
    pub loc_type: i32,
    pub value: Option<String>,
    #[serde(default)]
    pub create_folder: Option<bool>,
    // BSA-specific properties (Type = 2)
    pub archive_type: Option<u16>,
    pub archive_flags: Option<u32>,
    pub files_flags: Option<u32>,
    #[serde(default)]
    pub archive_compressed: Option<bool>,
}

impl Location {
    /// Check if this is a directory location
    pub fn is_directory(&self) -> bool {
        self.loc_type == 0
    }

    /// Check if this is a BSA source location
    pub fn is_bsa_source(&self) -> bool {
        self.loc_type == 1
    }

    /// Check if this is a BSA creation target
    pub fn is_bsa_creation(&self) -> bool {
        self.loc_type == 2
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Tag {
    pub name: Option<String>,
    #[serde(rename = "ID")]
    pub id: i32,
    pub text_color: Option<String>,
    pub back_color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Check {
    #[serde(rename = "Type")]
    pub check_type: i32,
    #[serde(default)]
    pub inverted: bool,
    pub loc: i32,
    pub file: Option<String>,
    pub checksums: Option<String>,
    pub free_size: Option<i64>,
    pub custom_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FileAttr {
    pub value: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PostCommand {
    pub value: Option<String>,
    #[serde(default)]
    pub wait: bool,
    #[serde(default)]
    pub hidden: bool,
}
