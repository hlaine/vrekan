use std::fs;
use std::path::{Path, PathBuf};

use game_core::{DamageType, SkillDefinition, SkillKind};
use serde::Deserialize;

use crate::enemy::EffectTemplate;
use crate::ContentError;

/// RON-facing mirror of `game_core::SkillKind` — kept separate the same
/// way `EffectKindTemplate` mirrors `game_core::EffectKind`: content schema
/// stays decoupled from `game_core`'s internal representation (e.g.
/// `damage_type` is a plain `String` here, converted to `DamageType` at
/// this same load-time boundary).
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub enum SkillKindTemplate {
    PowerStrike {
        damage: f32,
        damage_type: String,
        range: f32,
        #[serde(default)]
        effects: Vec<EffectTemplate>,
    },
    AoeBurst {
        damage: f32,
        damage_type: String,
        radius: f32,
        #[serde(default)]
        effects: Vec<EffectTemplate>,
    },
    SelfBuff {
        effect: EffectTemplate,
    },
}

/// Data-driven shape and numbers for a skill — see `EnemyTemplate`'s doc
/// comment for the same content-not-engine-code principle applied here.
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct SkillTemplate {
    pub od_cost: f32,
    pub cooldown: f32,
    pub kind: SkillKindTemplate,
}

impl SkillTemplate {
    pub fn into_definition(self) -> SkillDefinition {
        let kind = match self.kind {
            SkillKindTemplate::PowerStrike {
                damage,
                damage_type,
                range,
                effects,
            } => SkillKind::PowerStrike {
                damage,
                damage_type: DamageType(damage_type),
                range,
                effects: effects
                    .into_iter()
                    .map(EffectTemplate::into_definition)
                    .collect(),
            },
            SkillKindTemplate::AoeBurst {
                damage,
                damage_type,
                radius,
                effects,
            } => SkillKind::AoeBurst {
                damage,
                damage_type: DamageType(damage_type),
                radius,
                effects: effects
                    .into_iter()
                    .map(EffectTemplate::into_definition)
                    .collect(),
            },
            SkillKindTemplate::SelfBuff { effect } => SkillKind::SelfBuff {
                effect: effect.into_definition(),
            },
        };
        SkillDefinition {
            od_cost: self.od_cost,
            cooldown: self.cooldown,
            kind,
        }
    }
}

pub fn parse_skill_template(ron_str: &str) -> ron::error::SpannedResult<SkillTemplate> {
    ron::from_str(ron_str)
}

pub fn load_skill_template(path: &Path) -> Result<SkillTemplate, ContentError> {
    let contents = fs::read_to_string(path).map_err(|source| ContentError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_skill_template(&contents).map_err(|source| ContentError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

/// Loads every `.ron` file directly inside `dir` as a `SkillTemplate`,
/// keyed by filename (without extension) — same shape as
/// `load_all_enemy_templates`, including failing on the first malformed
/// file rather than silently skipping it.
pub fn load_all_skill_templates(dir: &Path) -> Result<Vec<(String, SkillTemplate)>, ContentError> {
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
            let template = load_skill_template(&path)?;
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            Ok((id, template))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the actual shipped content files, not just the
    /// inline fixtures below — see `EnemyTemplate`'s equivalent test for why.
    #[test]
    fn real_skill_templates_load_successfully() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/spells");
        load_all_skill_templates(&dir).unwrap();
    }

    const POWER_STRIKE: &str = r#"(
        od_cost: 20.0,
        cooldown: 2.0,
        kind: PowerStrike(
            damage: 40.0,
            damage_type: "primal",
            range: 80.0,
        ),
    )"#;

    #[test]
    fn parses_a_power_strike_template() {
        let template = parse_skill_template(POWER_STRIKE).unwrap();
        assert_eq!(template.od_cost, 20.0);
        match template.kind {
            SkillKindTemplate::PowerStrike {
                damage,
                ref damage_type,
                range,
                ref effects,
            } => {
                assert_eq!(damage, 40.0);
                assert_eq!(damage_type, "primal");
                assert_eq!(range, 80.0);
                assert!(effects.is_empty());
            }
            _ => panic!("expected PowerStrike"),
        }
    }

    #[test]
    fn parses_a_self_buff_template() {
        let template = parse_skill_template(
            r#"(
                od_cost: 25.0,
                cooldown: 8.0,
                kind: SelfBuff(
                    effect: (
                        id: "berserk",
                        kind: StatModifier(stat: CritChance),
                        duration: 4.0,
                        magnitude: 0.2,
                        stack_mode: Independent,
                        applies_to: Attacker,
                    ),
                ),
            )"#,
        )
        .unwrap();

        assert!(matches!(template.kind, SkillKindTemplate::SelfBuff { .. }));
    }

    #[test]
    fn rejects_malformed_ron_syntax() {
        let result = parse_skill_template("(od_cost: 20.0,");
        assert!(result.is_err());
    }

    #[test]
    fn into_definition_converts_damage_type_string_into_damage_type() {
        let template = parse_skill_template(POWER_STRIKE).unwrap();
        let definition = template.into_definition();
        assert_eq!(definition.od_cost, 20.0);
        match definition.kind {
            SkillKind::PowerStrike { damage_type, .. } => {
                assert_eq!(damage_type, DamageType("primal".to_string()));
            }
            _ => panic!("expected PowerStrike"),
        }
    }

    #[test]
    fn load_all_skill_templates_reads_every_ron_file_sorted_by_name() {
        let dir = std::env::temp_dir().join(format!(
            "vrekan_content_test_{}_{}",
            std::process::id(),
            "skill_load_all_sorted"
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b_aoe.ron"), POWER_STRIKE).unwrap();
        fs::write(dir.join("a_power.ron"), POWER_STRIKE).unwrap();

        let templates = load_all_skill_templates(&dir).unwrap();

        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].0, "a_power");
        assert_eq!(templates[1].0, "b_aoe");

        fs::remove_dir_all(&dir).unwrap();
    }
}
