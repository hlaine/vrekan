use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use bevy_ecs::prelude::*;
use game_core::{
    ActiveEffects, DamageType, Destructible, DestructibleKind, Health, LootEntry, LootTable,
    Position, Resistances,
};
use serde::Deserialize;

use crate::enemy::LootEntryTemplate;
use crate::ContentError;

/// Data-driven stats + appearance for a destructible (crate/barrel/etc.) —
/// same shape as `EnemyTemplate` minus every AI/attack/`XpReward` field, see
/// MECHANICS.md's Dynamic objects section: "an enemy template stripped
/// down." A distinct struct rather than reusing `EnemyTemplate` with
/// optional fields, so a destructible `.ron` file can't accidentally carry
/// (or omit) combat fields that would silently do nothing.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct DestructibleTemplate {
    pub max_health: f32,
    /// Resistance fraction per damage type — same shape/defaulting as
    /// `EnemyTemplate::resistances`.
    #[serde(default)]
    pub resistances: HashMap<String, f32>,
    pub color: [f32; 3],
    pub size: f32,
    /// Chance (0.0-1.0) this destructible drops anything at all on death —
    /// see `game_core::roll_loot`. Optional: defaults to never dropping.
    #[serde(default)]
    pub drop_chance: f32,
    /// Weighted item/rune/currency entries rolled if `drop_chance` hits.
    #[serde(default)]
    pub loot_table: Vec<LootEntryTemplate>,
    /// Whether this destructible is a pushable `RigidBody::Dynamic` body
    /// (e.g. a crate) rather than a fixed `RigidBody::Static` one (e.g. a
    /// stone pillar) — see MECHANICS.md's Dynamic objects section.
    /// Optional: defaults to `false` (static), the safer default for
    /// content authored before this field existed.
    #[serde(default)]
    pub movable: bool,
    /// How much this destructible resists being pushed — only meaningful
    /// when `movable` is `true`. Higher feels heavier (harder to budge,
    /// settles back to rest almost immediately once no longer pushed);
    /// lower feels lighter (easier to nudge, a touch more give before
    /// settling). See `server::pushable_physics`. Tuning data, not a
    /// settled number.
    #[serde(default = "default_weight")]
    pub weight: f32,
}

fn default_weight() -> f32 {
    1.0
}

pub fn parse_destructible_template(
    ron_str: &str,
) -> ron::error::SpannedResult<DestructibleTemplate> {
    ron::from_str(ron_str)
}

pub fn load_destructible_template(path: &Path) -> Result<DestructibleTemplate, ContentError> {
    let contents = fs::read_to_string(path).map_err(|source| ContentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_destructible_template(&contents).map_err(|source| ContentError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

/// Loads every `.ron` file directly inside `dir` as a `DestructibleTemplate`,
/// keyed by filename (without extension) — same "sorted, fail on first
/// malformed file" shape as `load_all_enemy_templates`.
pub fn load_all_destructible_templates(
    dir: &Path,
) -> Result<Vec<(String, DestructibleTemplate)>, ContentError> {
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
            let template = load_destructible_template(&path)?;
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
/// components, no physics; the caller (`server`) attaches both, the same
/// split `spawn_enemy` uses. Deliberately includes `ActiveEffects::default()`
/// even though a destructible has no AI to ever apply an effect to itself:
/// `combat::resolve_melee_hit` reads *both* the attacker's and the target's
/// `ActiveEffects` unconditionally (`Query::get_many_mut` on `[attacker,
/// target]`) and silently no-ops the whole hit — no damage applied at all —
/// if either is missing it. Omitting this here would make every attack
/// against a destructible a quiet do-nothing.
pub fn spawn_destructible(
    commands: &mut Commands,
    kind: impl Into<String>,
    template: &DestructibleTemplate,
    position: Position,
) -> Entity {
    let resistances = Resistances(
        template
            .resistances
            .iter()
            .map(|(damage_type, fraction)| (DamageType(damage_type.clone()), *fraction))
            .collect(),
    );
    let loot_table = LootTable {
        drop_chance: template.drop_chance,
        entries: template
            .loot_table
            .iter()
            .cloned()
            .map(LootEntryTemplate::into_entry)
            .collect::<Vec<LootEntry>>(),
    };

    commands
        .spawn((
            Destructible,
            DestructibleKind(kind.into()),
            position,
            Health::new(template.max_health),
            resistances,
            ActiveEffects::default(),
            loot_table,
        ))
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the actual shipped content files — same
    /// "hand-typed RON only fails loudly at server startup otherwise"
    /// reasoning as `enemy::tests::real_enemy_templates_load_successfully`.
    #[test]
    fn real_destructible_templates_load_successfully() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/destructibles");
        load_all_destructible_templates(&dir).unwrap();
    }

    const VALID_TEMPLATE: &str = r#"(
        max_health: 15.0,
        resistances: {"holy": 0.2},
        color: (0.55, 0.35, 0.15),
        size: 24.0,
    )"#;

    #[test]
    fn parses_a_well_formed_template() {
        let template = parse_destructible_template(VALID_TEMPLATE).unwrap();
        assert_eq!(template.max_health, 15.0);
        assert_eq!(template.color, [0.55, 0.35, 0.15]);
        assert_eq!(template.resistances.get("holy"), Some(&0.2));
    }

    #[test]
    fn resistances_and_loot_default_when_omitted() {
        let template = parse_destructible_template(
            r#"(
                max_health: 15.0,
                color: (0.55, 0.35, 0.15),
                size: 24.0,
            )"#,
        )
        .unwrap();

        assert!(template.resistances.is_empty());
        assert_eq!(template.drop_chance, 0.0);
        assert!(template.loot_table.is_empty());
    }

    #[test]
    fn movable_defaults_to_false_and_weight_defaults_to_one() {
        let template = parse_destructible_template(VALID_TEMPLATE).unwrap();

        assert!(!template.movable);
        assert_eq!(template.weight, 1.0);
    }

    #[test]
    fn parses_a_movable_template_with_an_explicit_weight() {
        let template = parse_destructible_template(
            r#"(
                max_health: 15.0,
                color: (0.55, 0.35, 0.15),
                size: 24.0,
                movable: true,
                weight: 2.5,
            )"#,
        )
        .unwrap();

        assert!(template.movable);
        assert_eq!(template.weight, 2.5);
    }

    #[test]
    fn rejects_malformed_ron_syntax() {
        let result = parse_destructible_template("(max_health: 15.0,");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_a_missing_required_field() {
        let result = parse_destructible_template(
            r#"(
                resistances: {},
                color: (0.55, 0.35, 0.15),
                size: 24.0,
            )"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn load_destructible_template_reports_missing_file_clearly() {
        let error = load_destructible_template(Path::new("does/not/exist.ron")).unwrap_err();
        assert!(matches!(error, ContentError::Io { .. }));
        assert!(error.to_string().contains("does/not/exist.ron"));
    }

    #[test]
    fn load_all_destructible_templates_reads_every_ron_file_sorted_by_name() {
        let dir = std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{}",
            std::process::id(),
            "destructible_load_all_sorted"
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b_barrel.ron"), VALID_TEMPLATE).unwrap();
        fs::write(dir.join("a_crate.ron"), VALID_TEMPLATE).unwrap();
        fs::write(dir.join("not_a_template.txt"), "ignored").unwrap();

        let templates = load_all_destructible_templates(&dir).unwrap();

        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].0, "a_crate");
        assert_eq!(templates[1].0, "b_barrel");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_all_destructible_templates_fails_loudly_on_first_malformed_file() {
        let dir = std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{}",
            std::process::id(),
            "destructible_load_all_malformed"
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("broken.ron"), "(max_health: not_a_number)").unwrap();

        let result = load_all_destructible_templates(&dir);

        assert!(matches!(result, Err(ContentError::Parse { .. })));

        fs::remove_dir_all(&dir).unwrap();
    }
}
