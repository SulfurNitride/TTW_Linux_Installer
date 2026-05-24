use anyhow::Result;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameKind {
    Fallout3,
    FalloutNewVegas,
    Oblivion,
}

impl GameKind {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Fallout3 => "Fallout 3",
            Self::FalloutNewVegas => "Fallout New Vegas",
            Self::Oblivion => "Oblivion",
        }
    }

    fn steam_app_ids(self) -> &'static [&'static str] {
        match self {
            Self::Fallout3 => &["22300", "22370"],
            Self::FalloutNewVegas => &["22380"],
            Self::Oblivion => &["22330"],
        }
    }

    fn title_markers(self) -> &'static [&'static str] {
        match self {
            Self::Fallout3 => &["fallout 3"],
            Self::FalloutNewVegas => &["fallout new vegas", "falloutnv"],
            Self::Oblivion => &["oblivion"],
        }
    }

    fn required_files(self) -> &'static [&'static str] {
        match self {
            Self::Fallout3 => &[
                "Data/Fallout3.esm",
                "Data/Fallout - Meshes.bsa",
                "Data/Fallout - Textures.bsa",
                "Data/Fallout - Voices.bsa",
            ],
            Self::FalloutNewVegas => &[
                "Data/FalloutNV.esm",
                "Data/Fallout - Meshes.bsa",
                "Data/Fallout - Textures.bsa",
                "Data/Fallout - Voices1.bsa",
            ],
            Self::Oblivion => &[
                "Data/Oblivion.esm",
                "Data/Oblivion - Meshes.bsa",
                "Data/Oblivion - Textures - Compressed.bsa",
            ],
        }
    }

    fn known_dir_names(self) -> &'static [&'static str] {
        match self {
            Self::Fallout3 => &["Fallout 3", "Fallout 3 goty", "Fallout 3 GOTY"],
            Self::FalloutNewVegas => &["Fallout New Vegas", "FalloutNV"],
            Self::Oblivion => &["Oblivion"],
        }
    }
}

#[derive(Debug, Clone)]
pub struct DetectedGame {
    pub kind: GameKind,
    pub path: PathBuf,
    pub source: String,
}

#[derive(Debug, Clone, Default)]
pub struct GameDetection {
    pub fallout3: Option<DetectedGame>,
    pub falloutnv: Option<DetectedGame>,
    pub oblivion: Option<DetectedGame>,
}

impl GameDetection {
    pub fn detect() -> Self {
        let mut candidates = Vec::new();
        candidates.extend(detect_steam_games());
        candidates.extend(detect_heroic_games());
        candidates.extend(detect_common_game_dirs());

        let mut seen = HashSet::new();
        candidates.retain(|game| {
            let key = (game.kind, canonical_or_clone(&game.path));
            seen.insert(key)
        });

        Self {
            fallout3: best_candidate(&candidates, GameKind::Fallout3),
            falloutnv: best_candidate(&candidates, GameKind::FalloutNewVegas),
            oblivion: best_candidate(&candidates, GameKind::Oblivion),
        }
    }

    pub fn detected_count(&self) -> usize {
        [&self.fallout3, &self.falloutnv, &self.oblivion]
            .iter()
            .filter(|game| game.is_some())
            .count()
    }
}

pub fn validate_game_path(kind: GameKind, path: &Path) -> bool {
    path.is_dir()
        && kind
            .required_files()
            .iter()
            .all(|relative| path_has_case_insensitive(path, relative))
}

fn best_candidate(candidates: &[DetectedGame], kind: GameKind) -> Option<DetectedGame> {
    candidates.iter().find(|game| game.kind == kind).cloned()
}

fn detect_steam_games() -> Vec<DetectedGame> {
    let mut detected = Vec::new();

    for steam_root in steam_roots() {
        for library in steam_libraries(&steam_root) {
            let steamapps = library.join("steamapps");
            if !steamapps.is_dir() {
                continue;
            }

            let manifests = match fs::read_dir(&steamapps) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for entry in manifests.filter_map(|entry| entry.ok()) {
                let path = entry.path();
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
                    continue;
                }

                let manifest = match fs::read_to_string(&path) {
                    Ok(data) => parse_vdf_pairs(&data),
                    Err(_) => continue,
                };
                let app_id = manifest.get("appid").map(String::as_str).unwrap_or("");
                let install_dir = manifest.get("installdir").map(String::as_str).unwrap_or("");
                if app_id.is_empty() || install_dir.is_empty() {
                    continue;
                }

                let install_path = steamapps.join("common").join(install_dir);
                if let Some(kind) = game_kind_for_steam_app(app_id) {
                    push_if_valid(&mut detected, kind, install_path, "Steam");
                }
            }
        }
    }

    detected
}

fn detect_heroic_games() -> Vec<DetectedGame> {
    let mut detected = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return detected;
    };

    for heroic_root in [
        home.join(".config/heroic"),
        home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"),
    ] {
        detect_heroic_gog(&heroic_root, &mut detected);
        detect_heroic_epic(&heroic_root, &mut detected);
    }

    detected
}

fn detect_common_game_dirs() -> Vec<DetectedGame> {
    let mut detected = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return detected;
    };

    let roots = [
        home.join("Games"),
        home.join("games"),
        home.join("GOG Games"),
        home.join(".local/share/bottles/bottles"),
        home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
    ];

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for kind in [
            GameKind::Fallout3,
            GameKind::FalloutNewVegas,
            GameKind::Oblivion,
        ] {
            for dir_name in kind.known_dir_names() {
                push_if_valid(
                    &mut detected,
                    kind,
                    root.join(dir_name),
                    "Common game folder",
                );
                push_if_valid(
                    &mut detected,
                    kind,
                    root.join("drive_c/Program Files (x86)/Steam/steamapps/common")
                        .join(dir_name),
                    "Bottles",
                );
                push_if_valid(
                    &mut detected,
                    kind,
                    root.join("drive_c/GOG Games").join(dir_name),
                    "Bottles",
                );
            }
        }
    }

    detected
}

fn detect_heroic_gog(heroic_root: &Path, detected: &mut Vec<DetectedGame>) {
    let path = heroic_root.join("gog_store/installed.json");
    let Ok(json) = read_json(&path) else {
        return;
    };

    let installed = json
        .get("installed")
        .and_then(Value::as_array)
        .or_else(|| json.as_array());
    let Some(installed) = installed else {
        return;
    };

    for game in installed {
        let install_path = game
            .get("install_path")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let title = game.get("title").and_then(Value::as_str).unwrap_or("");
        if let Some(path) = install_path {
            if let Some(kind) = classify_title_or_path(title, &path) {
                push_if_valid(detected, kind, path, "Heroic (GOG)");
            }
        }
    }
}

fn detect_heroic_epic(heroic_root: &Path, detected: &mut Vec<DetectedGame>) {
    for path in [
        heroic_root.join("store_cache/legendary_library.json"),
        heroic_root.join("legendaryConfig/legendary/installed.json"),
    ] {
        let Ok(json) = read_json(&path) else {
            continue;
        };
        let Some(obj) = json.as_object() else {
            continue;
        };

        for game in obj.values() {
            let install_path = game
                .get("install_path")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let title = game.get("title").and_then(Value::as_str).unwrap_or("");
            if let Some(path) = install_path {
                if let Some(kind) = classify_title_or_path(title, &path) {
                    push_if_valid(detected, kind, path, "Heroic (Epic)");
                }
            }
        }
    }
}

fn steam_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };

    [
        ".local/share/Steam",
        ".steam/debian-installation",
        ".steam/steam",
        ".var/app/com.valvesoftware.Steam/data/Steam",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        "snap/steam/common/.local/share/Steam",
    ]
    .iter()
    .map(|relative| home.join(relative))
    .filter(|path| path.join("steamapps").is_dir() || path.join("steam.pid").exists())
    .collect()
}

fn steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
    let mut libraries = vec![steam_root.to_path_buf()];

    for relative in ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
        let path = steam_root.join(relative);
        let Ok(data) = fs::read_to_string(path) else {
            continue;
        };
        for library in parse_library_folders(&data) {
            if library.is_dir() && !libraries.contains(&library) {
                libraries.push(library);
            }
        }
    }

    libraries
}

fn parse_library_folders(vdf: &str) -> Vec<PathBuf> {
    parse_vdf_pairs(vdf)
        .into_iter()
        .filter_map(|(key, value)| (key == "path").then(|| PathBuf::from(unescape_vdf(&value))))
        .collect()
}

fn parse_vdf_pairs(vdf: &str) -> HashMap<String, String> {
    let mut pairs = HashMap::new();
    for line in vdf.lines() {
        let values = quoted_values(line);
        if values.len() >= 2 {
            pairs.insert(values[0].to_ascii_lowercase(), unescape_vdf(&values[1]));
        }
    }
    pairs
}

fn quoted_values(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escaped = false;

    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' if in_quote => {
                values.push(current.clone());
                current.clear();
                in_quote = false;
            }
            '"' => in_quote = true,
            _ if in_quote => current.push(ch),
            _ => {}
        }
    }

    values
}

fn unescape_vdf(value: &str) -> String {
    value.replace("\\\\", "\\")
}

fn read_json(path: &Path) -> Result<Value> {
    let data = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

fn game_kind_for_steam_app(app_id: &str) -> Option<GameKind> {
    [
        GameKind::Fallout3,
        GameKind::FalloutNewVegas,
        GameKind::Oblivion,
    ]
    .into_iter()
    .find(|kind| kind.steam_app_ids().contains(&app_id))
}

fn classify_title_or_path(title: &str, path: &Path) -> Option<GameKind> {
    let haystack = format!("{} {}", title, path.display()).to_ascii_lowercase();
    [
        GameKind::FalloutNewVegas,
        GameKind::Fallout3,
        GameKind::Oblivion,
    ]
    .into_iter()
    .find(|kind| {
        kind.title_markers()
            .iter()
            .any(|marker| haystack.contains(marker))
    })
}

fn push_if_valid(
    detected: &mut Vec<DetectedGame>,
    kind: GameKind,
    path: PathBuf,
    source: impl Into<String>,
) {
    if validate_game_path(kind, &path) {
        detected.push(DetectedGame {
            kind,
            path,
            source: source.into(),
        });
    }
}

fn path_has_case_insensitive(root: &Path, relative: &str) -> bool {
    let mut current = root.to_path_buf();
    for component in relative.split('/') {
        let exact = current.join(component);
        if exact.exists() {
            current = exact;
            continue;
        }

        let target = component.to_ascii_lowercase();
        let Ok(entries) = fs::read_dir(&current) else {
            return false;
        };
        let Some(found) = entries
            .filter_map(|entry| entry.ok())
            .find(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase() == target)
        else {
            return false;
        };
        current = found.path();
    }

    current.exists()
}

fn canonical_or_clone(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{parse_library_folders, parse_vdf_pairs, quoted_values};

    #[test]
    fn parses_appmanifest_pairs() {
        let pairs = parse_vdf_pairs(
            r#"
            "AppState"
            {
                "appid" "22380"
                "installdir" "Fallout New Vegas"
            }
            "#,
        );

        assert_eq!(pairs.get("appid").unwrap(), "22380");
        assert_eq!(pairs.get("installdir").unwrap(), "Fallout New Vegas");
    }

    #[test]
    fn parses_library_folder_paths() {
        let libraries = parse_library_folders(
            r#"
            "libraryfolders"
            {
                "0"
                {
                    "path" "/mnt/games/SteamLibrary"
                }
            }
            "#,
        );

        assert_eq!(libraries[0].to_string_lossy(), "/mnt/games/SteamLibrary");
    }

    #[test]
    fn quoted_values_handles_escaped_quotes() {
        let values = quoted_values(r#""name" "Fallout \"New\" Vegas""#);
        assert_eq!(values, ["name", "Fallout \"New\" Vegas"]);
    }
}
