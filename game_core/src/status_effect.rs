use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::combat::{apply_damage, DamageType, Health, Resistances};
use crate::DeltaSeconds;

/// Which stat a `StatModifier` effect (or, since M7, a socketed rune)
/// adjusts. A small fixed set — extend only once a new stat is actually
/// wired up somewhere, not speculatively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum Stat {
    CritChance,
    CritMultiplier,
    MoveSpeed,
}

/// Same-effect-reapplied behavior, a per-effect data field rather than a
/// single global rule — see MECHANICS.md's Combat section.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub enum StackMode {
    RefreshDuration,
    AddMagnitude,
    Independent,
}

/// Which side of a landed attack an effect applies to: the entity that got
/// hit, or the attacker themselves (e.g. a self-buff like "fury" that
/// procs on a landed hit).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub enum EffectTarget {
    Target,
    Attacker,
}

/// Which generic behavior this effect applies — a fixed set of shapes the
/// engine knows how to interpret. The effect's specific identity/flavor
/// (name, numbers) is content (`EffectDefinition`), not this enum — the
/// same data-vs-engine split as `DamageType`: new *instances* (bleed vs.
/// poison) are data, a new *behavior category* is an engine change.
#[derive(Debug, Clone, PartialEq)]
pub enum EffectKind {
    DamageOverTime {
        damage_type: DamageType,
        tick_interval: f32,
    },
    Stun,
    StatModifier {
        stat: Stat,
    },
}

/// A content-defined effect an attack can attach — see MECHANICS.md's
/// Combat section. `id` identifies the effect for stacking purposes only
/// (is a newly-applied instance "the same effect" as one already active?),
/// not for gameplay lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectDefinition {
    pub id: String,
    pub kind: EffectKind,
    pub duration: f32,
    pub magnitude: f32,
    pub stack_mode: StackMode,
    pub applies_to: EffectTarget,
    /// Fraction of landed hits that actually apply this effect (`1.0` =
    /// always). Rolled fresh per hit in `attack_system` — found necessary
    /// via live testing: a guaranteed, `RefreshDuration` crowd-control
    /// effect whose duration outlasts the inflictor's own attack cooldown
    /// can permalock a target (each hit refreshes the clock before it ever
    /// expires). A `Stun` especially should rarely be 1.0 — see
    /// `assets/enemies/converted_farmer.ron`'s daze for the fix in
    /// practice.
    pub chance: f32,
}

/// One currently-active instance of an `EffectDefinition`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveEffect {
    pub definition: EffectDefinition,
    pub remaining_duration: f32,
    pub time_until_next_tick: f32,
}

impl ActiveEffect {
    fn new(definition: EffectDefinition) -> Self {
        let time_until_next_tick = match &definition.kind {
            EffectKind::DamageOverTime { tick_interval, .. } => *tick_interval,
            _ => 0.0,
        };
        ActiveEffect {
            remaining_duration: definition.duration,
            time_until_next_tick,
            definition,
        }
    }
}

/// Every status effect currently active on an entity. Server-only — not
/// replicated, same as `AttackTimer`. The one thing a client needs to
/// react to (`Stunned`) is its own tiny marker component, kept in sync by
/// `tick_status_effects` and replicated separately.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct ActiveEffects(pub Vec<ActiveEffect>);

impl ActiveEffects {
    /// Applies `definition`, honoring its `stack_mode` against any
    /// already-active instance sharing the same `id`.
    pub fn apply(&mut self, definition: EffectDefinition) {
        if let Some(existing) = self.0.iter_mut().find(|e| e.definition.id == definition.id) {
            match definition.stack_mode {
                StackMode::RefreshDuration => {
                    existing.remaining_duration = definition.duration;
                }
                StackMode::AddMagnitude => {
                    existing.definition.magnitude += definition.magnitude;
                    existing.remaining_duration = definition.duration;
                }
                StackMode::Independent => {
                    self.0.push(ActiveEffect::new(definition));
                }
            }
            return;
        }
        self.0.push(ActiveEffect::new(definition));
    }

    /// Sum of magnitudes from active `StatModifier` effects matching
    /// `stat` — the effective bonus to add to the entity's base stat value
    /// at the point of use. Never mutates the base stat component itself,
    /// so there's nothing to leave stale when a buff expires (see
    /// DECISIONS.md's sprite-tint-overlay lesson from ally-revive).
    pub fn stat_bonus(&self, stat: Stat) -> f32 {
        self.0
            .iter()
            .filter_map(|e| match &e.definition.kind {
                EffectKind::StatModifier { stat: s } if *s == stat => Some(e.definition.magnitude),
                _ => None,
            })
            .sum()
    }

    pub fn is_stunned(&self) -> bool {
        self.0
            .iter()
            .any(|e| matches!(e.definition.kind, EffectKind::Stun))
    }
}

/// Marks an entity currently affected by a `Stun`-kind effect — cheap and
/// query-filterable, kept in sync by `tick_status_effects`, mirroring
/// `Downed`'s replication shape exactly. Unlike `Downed`, a stunned entity
/// stays a valid attack target: it can't act, but it isn't "out of
/// combat" (see MECHANICS.md's Death, downed state, and revive section for
/// the contrast).
#[derive(Component, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Stunned;

/// Ticks every active effect's duration and (for damage-over-time) its
/// per-tick timer, applying damage through the target's `Resistances` the
/// same way `attack_system` does, dropping expired effects, and keeping
/// the `Stunned` marker in sync with whether a `Stun`-kind effect is
/// currently active.
pub fn tick_status_effects(
    delta: Res<DeltaSeconds>,
    mut query: Query<(
        Entity,
        &mut ActiveEffects,
        &mut Health,
        Option<&Resistances>,
    )>,
    mut commands: Commands,
) {
    let dt = delta.0;
    let no_resistances = Resistances::default();

    for (entity, mut effects, mut health, resistances) in &mut query {
        let resistances = resistances.unwrap_or(&no_resistances);

        for effect in &mut effects.0 {
            effect.remaining_duration -= dt;
            if let EffectKind::DamageOverTime {
                damage_type,
                tick_interval,
            } = &effect.definition.kind
            {
                effect.time_until_next_tick -= dt;
                if effect.time_until_next_tick <= 0.0 {
                    let resistance = resistances.get(damage_type);
                    apply_damage(
                        &mut health,
                        effect.definition.magnitude * (1.0 - resistance),
                    );
                    effect.time_until_next_tick += tick_interval;
                }
            }
        }
        effects.0.retain(|e| e.remaining_duration > 0.0);

        if effects.is_stunned() {
            commands.entity(entity).insert(Stunned);
        } else {
            commands.entity(entity).remove::<Stunned>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn bleed(magnitude: f32, stack_mode: StackMode) -> EffectDefinition {
        EffectDefinition {
            id: "bleed".to_string(),
            kind: EffectKind::DamageOverTime {
                damage_type: DamageType("primal".to_string()),
                tick_interval: 1.0,
            },
            duration: 4.0,
            magnitude,
            stack_mode,
            applies_to: EffectTarget::Target,
            chance: 1.0,
        }
    }

    #[test]
    fn apply_pushes_a_new_effect_when_none_matches() {
        let mut effects = ActiveEffects::default();
        effects.apply(bleed(3.0, StackMode::RefreshDuration));
        assert_eq!(effects.0.len(), 1);
        assert_eq!(effects.0[0].remaining_duration, 4.0);
    }

    #[test]
    fn refresh_duration_resets_clock_without_changing_magnitude() {
        let mut effects = ActiveEffects::default();
        effects.apply(bleed(3.0, StackMode::RefreshDuration));
        effects.0[0].remaining_duration = 1.0;

        effects.apply(bleed(3.0, StackMode::RefreshDuration));

        assert_eq!(effects.0.len(), 1);
        assert_eq!(effects.0[0].remaining_duration, 4.0);
        assert_eq!(effects.0[0].definition.magnitude, 3.0);
    }

    #[test]
    fn add_magnitude_increases_magnitude_and_resets_clock() {
        let mut effects = ActiveEffects::default();
        effects.apply(bleed(3.0, StackMode::AddMagnitude));
        effects.0[0].remaining_duration = 1.0;

        effects.apply(bleed(3.0, StackMode::AddMagnitude));

        assert_eq!(effects.0.len(), 1);
        assert_eq!(effects.0[0].remaining_duration, 4.0);
        assert_eq!(effects.0[0].definition.magnitude, 6.0);
    }

    #[test]
    fn independent_stacking_always_pushes_a_separate_instance() {
        let mut effects = ActiveEffects::default();
        effects.apply(bleed(3.0, StackMode::Independent));
        effects.apply(bleed(3.0, StackMode::Independent));

        assert_eq!(effects.0.len(), 2);
    }

    #[test]
    fn stat_bonus_sums_matching_stat_modifiers_only() {
        let mut effects = ActiveEffects::default();
        effects.apply(EffectDefinition {
            id: "fury".to_string(),
            kind: EffectKind::StatModifier {
                stat: Stat::CritChance,
            },
            duration: 3.0,
            magnitude: 0.05,
            stack_mode: StackMode::Independent,
            applies_to: EffectTarget::Attacker,
            chance: 1.0,
        });
        effects.apply(EffectDefinition {
            id: "fury".to_string(),
            kind: EffectKind::StatModifier {
                stat: Stat::CritChance,
            },
            duration: 3.0,
            magnitude: 0.05,
            stack_mode: StackMode::Independent,
            applies_to: EffectTarget::Attacker,
            chance: 1.0,
        });

        assert!((effects.stat_bonus(Stat::CritChance) - 0.10).abs() < 1e-6);
        assert_eq!(effects.stat_bonus(Stat::CritMultiplier), 0.0);
    }

    #[test]
    fn is_stunned_reflects_an_active_stun_effect() {
        let mut effects = ActiveEffects::default();
        assert!(!effects.is_stunned());

        effects.apply(EffectDefinition {
            id: "daze".to_string(),
            kind: EffectKind::Stun,
            duration: 1.5,
            magnitude: 0.0,
            stack_mode: StackMode::RefreshDuration,
            applies_to: EffectTarget::Target,
            chance: 1.0,
        });

        assert!(effects.is_stunned());
    }

    #[test]
    fn tick_status_effects_applies_damage_on_tick_and_respects_resistances() {
        let mut world = World::new();
        world.insert_resource(DeltaSeconds(1.0));
        let mut effects = ActiveEffects::default();
        effects.apply(bleed(10.0, StackMode::RefreshDuration));
        let entity = world
            .spawn((
                effects,
                Health::new(100.0),
                Resistances(std::collections::HashMap::from([(
                    DamageType("primal".to_string()),
                    0.5,
                )])),
            ))
            .id();

        let _ = world.run_system_once(tick_status_effects);

        assert_eq!(world.get::<Health>(entity).unwrap().current, 95.0);
    }

    #[test]
    fn tick_status_effects_removes_expired_effects() {
        let mut world = World::new();
        world.insert_resource(DeltaSeconds(5.0));
        let mut effects = ActiveEffects::default();
        effects.apply(bleed(1.0, StackMode::RefreshDuration));
        let entity = world.spawn((effects, Health::new(100.0))).id();

        let _ = world.run_system_once(tick_status_effects);

        assert!(world.get::<ActiveEffects>(entity).unwrap().0.is_empty());
    }

    #[test]
    fn tick_status_effects_maintains_the_stunned_marker() {
        let mut world = World::new();
        world.insert_resource(DeltaSeconds(1.0));
        let mut effects = ActiveEffects::default();
        effects.apply(EffectDefinition {
            id: "daze".to_string(),
            kind: EffectKind::Stun,
            duration: 1.5,
            magnitude: 0.0,
            stack_mode: StackMode::RefreshDuration,
            applies_to: EffectTarget::Target,
            chance: 1.0,
        });
        let entity = world.spawn((effects, Health::new(100.0))).id();

        let _ = world.run_system_once(tick_status_effects);
        assert!(world.get::<Stunned>(entity).is_some());

        world.insert_resource(DeltaSeconds(1.0));
        let _ = world.run_system_once(tick_status_effects);
        assert!(world.get::<Stunned>(entity).is_none());
    }
}
