use std::fs;
use std::path::{Path, PathBuf};

use game_core::{EquipSlot, ItemDefinition, RuneDefinition, Stat};
use serde::Deserialize;

use crate::ContentError;

/// Data-driven shape for an item template — `EquipSlot` is reused directly
/// from `game_core` (not mirrored like `EffectKindTemplate` mirrors
/// `EffectKind`) since it carries no data that needs a content-vs-engine
/// conversion, unlike e.g. `DamageType`.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct ItemTemplate {
    pub slot: EquipSlot,
    pub socket_count: u32,
}

impl ItemTemplate {
    pub fn into_definition(self) -> ItemDefinition {
        ItemDefinition {
            slot: self.slot,
            socket_count: self.socket_count,
        }
    }
}

/// Data-driven shape for a rune template — `Stat` is likewise reused
/// directly from `game_core` for the same reason `EquipSlot` is above.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct RuneTemplate {
    pub stat: Stat,
    pub magnitude: f32,
}

impl RuneTemplate {
    pub fn into_definition(self) -> RuneDefinition {
        RuneDefinition {
            stat: self.stat,
            magnitude: self.magnitude,
        }
    }
}

pub fn parse_item_template(ron_str: &str) -> ron::error::SpannedResult<ItemTemplate> {
    ron::from_str(ron_str)
}

pub fn load_item_template(path: &Path) -> Result<ItemTemplate, ContentError> {
    let contents = fs::read_to_string(path).map_err(|source| ContentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_item_template(&contents).map_err(|source| ContentError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

/// Loads every `.ron` file directly inside `dir` as an `ItemTemplate`,
/// keyed by filename (without extension) — same load-all shape as
/// `load_all_enemy_templates`.
pub fn load_all_item_templates(dir: &Path) -> Result<Vec<(String, ItemTemplate)>, ContentError> {
    load_all_ron(dir, load_item_template)
}

pub fn parse_rune_template(ron_str: &str) -> ron::error::SpannedResult<RuneTemplate> {
    ron::from_str(ron_str)
}

pub fn load_rune_template(path: &Path) -> Result<RuneTemplate, ContentError> {
    let contents = fs::read_to_string(path).map_err(|source| ContentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_rune_template(&contents).map_err(|source| ContentError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

pub fn load_all_rune_templates(dir: &Path) -> Result<Vec<(String, RuneTemplate)>, ContentError> {
    load_all_ron(dir, load_rune_template)
}

/// Shared "load every `.ron` file in `dir`, keyed by filename, sorted,
/// fail on the first malformed one" shape used by both item and rune
/// loaders (and, separately, `enemy::load_all_enemy_templates`/
/// `skill::load_all_skill_templates`, which predate this helper and
/// weren't worth refactoring to share it retroactively).
fn load_all_ron<T>(
    dir: &Path,
    load_one: impl Fn(&Path) -> Result<T, ContentError>,
) -> Result<Vec<(String, T)>, ContentError> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|source| ContentError::Io {
            path: dir.to_path_buf(),
            source,
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("ron"))
        .collect();
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let template = load_one(&path)?;
            let key = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            Ok((key, template))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_item_templates_load_successfully() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/items");
        load_all_item_templates(&dir).unwrap();
    }

    #[test]
    fn real_rune_templates_load_successfully() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/runes");
        load_all_rune_templates(&dir).unwrap();
    }

    #[test]
    fn parses_a_well_formed_item_template() {
        let template = parse_item_template(
            r#"(
                slot: Weapon,
                socket_count: 2,
            )"#,
        )
        .unwrap();

        assert_eq!(template.slot, EquipSlot::Weapon);
        assert_eq!(template.socket_count, 2);
    }

    #[test]
    fn rejects_malformed_item_ron_syntax() {
        assert!(parse_item_template("(slot: Weapon,").is_err());
    }

    #[test]
    fn parses_a_well_formed_rune_template() {
        let template = parse_rune_template(
            r#"(
                stat: CritChance,
                magnitude: 0.05,
            )"#,
        )
        .unwrap();

        assert_eq!(template.stat, Stat::CritChance);
        assert_eq!(template.magnitude, 0.05);
    }

    #[test]
    fn rejects_malformed_rune_ron_syntax() {
        assert!(parse_rune_template("(stat: CritChance,").is_err());
    }

    #[test]
    fn load_all_item_templates_reads_every_ron_file_sorted_by_name() {
        let dir = std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{}",
            std::process::id(),
            "item_load_all_sorted"
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b_helmet.ron"), "(slot: Helmet, socket_count: 1)").unwrap();
        fs::write(dir.join("a_sword.ron"), "(slot: Weapon, socket_count: 2)").unwrap();

        let templates = load_all_item_templates(&dir).unwrap();

        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].0, "a_sword");
        assert_eq!(templates[1].0, "b_helmet");

        fs::remove_dir_all(&dir).unwrap();
    }
}
