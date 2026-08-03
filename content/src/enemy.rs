use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use bevy_ecs::prelude::*;
use game_core::{
    Aggro, AttackTimer, CombatStats, DamageType, Enemy, EnemyKind, Facing, Health, MeleeAttack,
    MoveSpeed, Position, Resistances, Velocity,
};
use serde::Deserialize;

use crate::ContentError;

/// Data-driven stats + appearance for an enemy type. Appearance is plain
/// data (no `bevy_sprite`/`bevy_render` types) so `content` stays reusable by
/// the headless server, which never wants rendering dependencies.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct EnemyTemplate {
    pub max_health: f32,
    pub move_speed: f32,
    pub aggro_range: f32,
    pub melee_range: f32,
    pub melee_damage: f32,
    pub melee_cooldown: f32,
    pub melee_damage_type: String,
    pub crit_chance: f32,
    pub crit_multiplier: f32,
    /// Resistance fraction per damage type (see `game_core::Resistances`).
    /// Types not listed here default to no resistance — most enemies won't
    /// need every type spelled out, only the ones where they differ.
    #[serde(default)]
    pub resistances: HashMap<String, f32>,
    pub color: [f32; 3],
    pub size: f32,
}

pub fn parse_enemy_template(ron_str: &str) -> ron::error::SpannedResult<EnemyTemplate> {
    ron::from_str(ron_str)
}

pub fn load_enemy_template(path: &Path) -> Result<EnemyTemplate, ContentError> {
    let contents = fs::read_to_string(path).map_err(|source| ContentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_enemy_template(&contents).map_err(|source| ContentError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

/// Loads every `.ron` file directly inside `dir` as an `EnemyTemplate`,
/// keyed by filename (without extension). Fails on the first malformed
/// file rather than silently skipping it.
pub fn load_all_enemy_templates(dir: &Path) -> Result<Vec<(String, EnemyTemplate)>, ContentError> {
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
            let template = load_enemy_template(&path)?;
            let kind = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            Ok((kind, template))
        })
        .collect()
}

/// Spawns the gameplay-simulation entity for a template — no visual
/// components. The caller (client) is responsible for turning
/// `EnemyTemplate::color`/`size` into an actual `Sprite`; `kind` (the
/// template's key, e.g. from `load_all_enemy_templates`) is attached as
/// `EnemyKind` so a client that only sees this entity via replication can
/// still look up that appearance data itself.
pub fn spawn_enemy(
    commands: &mut Commands,
    kind: impl Into<String>,
    template: &EnemyTemplate,
    position: Position,
) -> Entity {
    let resistances = Resistances(
        template
            .resistances
            .iter()
            .map(|(damage_type, fraction)| (DamageType(damage_type.clone()), *fraction))
            .collect(),
    );

    commands
        .spawn((
            Enemy,
            EnemyKind(kind.into()),
            position,
            Velocity::ZERO,
            Facing::default(),
            MoveSpeed(template.move_speed),
            Health::new(template.max_health),
            MeleeAttack {
                range: template.melee_range,
                damage: template.melee_damage,
                cooldown: template.melee_cooldown,
                damage_type: DamageType(template.melee_damage_type.clone()),
            },
            CombatStats {
                crit_chance: template.crit_chance,
                crit_multiplier: template.crit_multiplier,
            },
            resistances,
            AttackTimer(0.0),
            Aggro {
                range: template.aggro_range,
            },
        ))
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TEMPLATE: &str = r#"(
        max_health: 40.0,
        move_speed: 90.0,
        aggro_range: 150.0,
        melee_range: 40.0,
        melee_damage: 8.0,
        melee_cooldown: 1.0,
        melee_damage_type: "primal",
        crit_chance: 0.05,
        crit_multiplier: 1.5,
        resistances: {"holy": 0.2},
        color: (0.8, 0.15, 0.15),
        size: 28.0,
    )"#;

    #[test]
    fn parses_a_well_formed_template() {
        let template = parse_enemy_template(VALID_TEMPLATE).unwrap();
        assert_eq!(template.max_health, 40.0);
        assert_eq!(template.color, [0.8, 0.15, 0.15]);
        assert_eq!(template.resistances.get("holy"), Some(&0.2));
    }

    #[test]
    fn resistances_default_to_empty_when_omitted() {
        let template = parse_enemy_template(
            r#"(
                max_health: 40.0,
                move_speed: 90.0,
                aggro_range: 150.0,
                melee_range: 40.0,
                melee_damage: 8.0,
                melee_cooldown: 1.0,
                melee_damage_type: "primal",
                crit_chance: 0.05,
                crit_multiplier: 1.5,
                color: (0.8, 0.15, 0.15),
                size: 28.0,
            )"#,
        )
        .unwrap();

        assert!(template.resistances.is_empty());
    }

    #[test]
    fn rejects_malformed_ron_syntax() {
        let result = parse_enemy_template("(max_health: 40.0,");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_a_missing_required_field() {
        let result = parse_enemy_template(
            r#"(
                move_speed: 90.0,
                aggro_range: 150.0,
                melee_range: 40.0,
                melee_damage: 8.0,
                melee_cooldown: 1.0,
                melee_damage_type: "primal",
                crit_chance: 0.05,
                crit_multiplier: 1.5,
                color: (0.8, 0.15, 0.15),
                size: 28.0,
            )"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn load_enemy_template_reports_missing_file_clearly() {
        let error = load_enemy_template(Path::new("does/not/exist.ron")).unwrap_err();
        assert!(matches!(error, ContentError::Io { .. }));
        assert!(error.to_string().contains("does/not/exist.ron"));
    }

    #[test]
    fn load_all_enemy_templates_reads_every_ron_file_sorted_by_name() {
        let dir = std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{}",
            std::process::id(),
            "load_all_sorted"
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b_missionary.ron"), VALID_TEMPLATE).unwrap();
        fs::write(dir.join("a_farmer.ron"), VALID_TEMPLATE).unwrap();
        fs::write(dir.join("not_a_template.txt"), "ignored").unwrap();

        let templates = load_all_enemy_templates(&dir).unwrap();

        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].0, "a_farmer");
        assert_eq!(templates[1].0, "b_missionary");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_all_enemy_templates_fails_loudly_on_first_malformed_file() {
        let dir = std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{}",
            std::process::id(),
            "load_all_malformed"
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("broken.ron"), "(max_health: not_a_number)").unwrap();

        let result = load_all_enemy_templates(&dir);

        assert!(matches!(result, Err(ContentError::Parse { .. })));

        fs::remove_dir_all(&dir).unwrap();
    }
}
