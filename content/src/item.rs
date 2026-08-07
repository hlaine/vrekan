use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use game_core::{
    DamageType, EquipSlot, ItemDefinition, Resistances, RuneDefinition, Stat, WeaponStats,
};
use serde::Deserialize;

use crate::ContentError;

/// Data-driven shape for a weapon's attack stats — see MECHANICS.md's
/// Weapons & attack timing section. `damage_type` is a plain `String` here
/// (converted to `game_core::DamageType` in `ItemTemplate::into_definition`),
/// mirroring `EnemyTemplate::melee_damage_type`'s content-vs-engine split.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct WeaponTemplate {
    pub damage: f32,
    pub damage_type: String,
    pub range: f32,
    pub attack_duration: f32,
    pub recovery: f32,
}

/// Data-driven shape for an item template — `EquipSlot` is reused directly
/// from `game_core` (not mirrored like `EffectKindTemplate` mirrors
/// `EffectKind`) since it carries no data that needs a content-vs-engine
/// conversion, unlike e.g. `DamageType`. `sell_value` is required, not
/// defaulted — same "a content author always makes a conscious choice"
/// convention as `EnemyTemplate::xp_reward`/`RuneTemplate::socket_cost`.
///
/// `weapon` and `resistances` are each meaningful for only one side of
/// `slot` (`weapon` for `Weapon` items, `resistances` for `Armor`/`Helmet`
/// — see MECHANICS.md) — `load_item_template` rejects a template that gets
/// this backwards (a `Weapon` with no `weapon` block, or *any* slot other
/// than `Weapon` carrying one, likewise `resistances` on a `Weapon`) rather
/// than silently ignoring the mismatched field, the same "a stat that
/// exists but does nothing is a bug, not a quirk" convention MECHANICS.md's
/// "Effective combat values are always computed fresh" section names
/// explicitly.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct ItemTemplate {
    pub slot: EquipSlot,
    pub socket_count: u32,
    pub sell_value: u32,
    #[serde(default)]
    pub weapon: Option<WeaponTemplate>,
    #[serde(default)]
    pub resistances: HashMap<String, f32>,
}

impl ItemTemplate {
    pub fn into_definition(self) -> ItemDefinition {
        let weapon = self.weapon.map(|weapon| WeaponStats {
            damage: weapon.damage,
            damage_type: DamageType(weapon.damage_type),
            range: weapon.range,
            attack_duration: weapon.attack_duration,
            recovery: weapon.recovery,
        });
        let resistances = Resistances(
            self.resistances
                .into_iter()
                .map(|(damage_type, fraction)| (DamageType(damage_type), fraction))
                .collect(),
        );
        ItemDefinition {
            slot: self.slot,
            socket_count: self.socket_count,
            sell_value: self.sell_value,
            weapon,
            resistances,
        }
    }
}

/// Rejects a template that gets `weapon`/`resistances` backwards relative to
/// `slot` — see `ItemTemplate`'s doc comment. Called from `load_item_template`
/// (which has `path` for the error), not `into_definition` (which doesn't),
/// so a malformed file fails at load time alongside RON syntax errors rather
/// than silently producing bad `ItemDefinition` data.
fn validate_item_template(template: &ItemTemplate) -> Result<(), String> {
    let is_weapon = template.slot == EquipSlot::Weapon;
    if is_weapon && template.weapon.is_none() {
        return Err("slot: Weapon requires a `weapon` block".to_string());
    }
    if !is_weapon && template.weapon.is_some() {
        return Err(format!(
            "slot: {:?} must not carry a `weapon` block (only Weapon items can)",
            template.slot
        ));
    }
    if is_weapon && !template.resistances.is_empty() {
        return Err(
            "slot: Weapon must not carry `resistances` (only Armor/Helmet can)".to_string(),
        );
    }
    Ok(())
}

/// Data-driven shape for a rune template — `Stat` is likewise reused
/// directly from `game_core` for the same reason `EquipSlot` is above.
/// `socket_cost` is required, not defaulted — same "a content author
/// always makes a conscious choice" convention as `EnemyTemplate::xp_reward`,
/// so a new rune can't silently become free to socket by omission.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct RuneTemplate {
    pub stat: Stat,
    pub magnitude: f32,
    pub socket_cost: u32,
}

impl RuneTemplate {
    pub fn into_definition(self) -> RuneDefinition {
        RuneDefinition {
            stat: self.stat,
            magnitude: self.magnitude,
            socket_cost: self.socket_cost,
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
    let template = parse_item_template(&contents).map_err(|source| ContentError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })?;
    validate_item_template(&template).map_err(|message| ContentError::Validation {
        path: path.to_path_buf(),
        message,
    })?;
    Ok(template)
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
/// loaders, and (since M8) `interact::load_all_interactable_templates`
/// (and, separately, `enemy::load_all_enemy_templates`/
/// `skill::load_all_skill_templates`, which predate this helper and
/// weren't worth refactoring to share it retroactively). `pub(crate)`
/// rather than private now that a second module reuses it.
pub(crate) fn load_all_ron<T>(
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
                sell_value: 5,
            )"#,
        )
        .unwrap();

        assert_eq!(template.slot, EquipSlot::Weapon);
        assert_eq!(template.socket_count, 2);
        assert_eq!(template.sell_value, 5);
    }

    #[test]
    fn rejects_malformed_item_ron_syntax() {
        assert!(parse_item_template("(slot: Weapon,").is_err());
    }

    fn write_template(dir: &Path, name: &str, ron_str: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, ron_str).unwrap();
        path
    }

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{label}",
            std::process::id()
        ))
    }

    #[test]
    fn load_rejects_a_weapon_slot_item_with_no_weapon_block() {
        let dir = temp_dir("weapon_missing_block");
        let path = write_template(
            &dir,
            "sword.ron",
            "(slot: Weapon, socket_count: 2, sell_value: 5)",
        );

        let error = load_item_template(&path).unwrap_err();
        assert!(matches!(error, ContentError::Validation { .. }));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_rejects_a_non_weapon_item_that_carries_a_weapon_block() {
        let dir = temp_dir("non_weapon_with_block");
        let path = write_template(
            &dir,
            "armor.ron",
            "(slot: Armor, socket_count: 1, sell_value: 5, weapon: Some((\
             damage: 5.0, damage_type: \"primal\", range: 40.0, \
             attack_duration: 0.2, recovery: 0.5)))",
        );

        let error = load_item_template(&path).unwrap_err();
        assert!(matches!(error, ContentError::Validation { .. }));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_rejects_a_weapon_that_carries_resistances() {
        let dir = temp_dir("weapon_with_resistances");
        let path = write_template(
            &dir,
            "sword.ron",
            "(slot: Weapon, socket_count: 2, sell_value: 5, weapon: Some((\
             damage: 5.0, damage_type: \"primal\", range: 40.0, \
             attack_duration: 0.2, recovery: 0.5)), \
             resistances: {\"holy\": 0.1})",
        );

        let error = load_item_template(&path).unwrap_err();
        assert!(matches!(error, ContentError::Validation { .. }));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_accepts_a_well_formed_weapon_and_a_well_formed_armor_with_resistances() {
        let dir = temp_dir("well_formed");
        let weapon_path = write_template(
            &dir,
            "sword.ron",
            "(slot: Weapon, socket_count: 2, sell_value: 5, weapon: Some((\
             damage: 8.0, damage_type: \"primal\", range: 60.0, \
             attack_duration: 0.3, recovery: 0.5)))",
        );
        let armor_path = write_template(
            &dir,
            "plate.ron",
            "(slot: Armor, socket_count: 1, sell_value: 15, \
             resistances: {\"holy\": 0.2})",
        );

        let weapon = load_item_template(&weapon_path).unwrap();
        let armor = load_item_template(&armor_path).unwrap();

        assert!(weapon.weapon.is_some());
        assert_eq!(armor.resistances.get("holy"), Some(&0.2));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_a_well_formed_rune_template() {
        let template = parse_rune_template(
            r#"(
                stat: CritChance,
                magnitude: 0.05,
                socket_cost: 15,
            )"#,
        )
        .unwrap();

        assert_eq!(template.stat, Stat::CritChance);
        assert_eq!(template.magnitude, 0.05);
        assert_eq!(template.socket_cost, 15);
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
        fs::write(
            dir.join("b_helmet.ron"),
            "(slot: Helmet, socket_count: 1, sell_value: 5)",
        )
        .unwrap();
        fs::write(
            dir.join("a_sword.ron"),
            "(slot: Weapon, socket_count: 2, sell_value: 5, weapon: Some((\
             damage: 8.0, damage_type: \"primal\", range: 60.0, \
             attack_duration: 0.3, recovery: 0.5)))",
        )
        .unwrap();

        let templates = load_all_item_templates(&dir).unwrap();

        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].0, "a_sword");
        assert_eq!(templates[1].0, "b_helmet");

        fs::remove_dir_all(&dir).unwrap();
    }
}
