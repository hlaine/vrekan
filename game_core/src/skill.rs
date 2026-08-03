use std::collections::HashMap;

use bevy_ecs::prelude::*;
use rand::{Rng, RngExt};
use serde::{Deserialize, Serialize};

use crate::combat::{apply_damage, resolve_damage, CombatStats, DamageType, Health, Resistances};
use crate::movement::Position;
use crate::player::{Downed, Player};
use crate::progression::{grant_xp, Level, UnspentStatPoints, XpReward};
use crate::status_effect::{ActiveEffects, EffectDefinition, Stunned};
use crate::DeltaSeconds;

/// Single resource pool ("od") that power attacks consume — see
/// MECHANICS.md's Resources section. "Od"/"fury/rage" are the same
/// underlying pool, just flavor-named differently in UI copy; the
/// type/field stay plain ASCII rather than "Öd" so they're normal Rust
/// identifiers. Player-only; not named `Resource` since that name is
/// already Bevy's own ECS-resource derive macro.
#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Od {
    pub current: f32,
    pub max: f32,
    pub regen_rate: f32,
}

impl Od {
    pub fn new(max: f32, regen_rate: f32) -> Self {
        Od {
            current: max,
            max,
            regen_rate,
        }
    }

    pub fn gain(&mut self, amount: f32) {
        self.current = (self.current + amount).min(self.max);
    }

    /// Deducts `amount` only if there's enough banked, returning whether it
    /// succeeded — lets a caller gate an action on having enough without a
    /// separate check-then-spend race (see `skill_cast_system`).
    pub fn try_spend(&mut self, amount: f32) -> bool {
        if self.current < amount {
            return false;
        }
        self.current -= amount;
        true
    }
}

/// Passive regeneration each tick — the other half of MECHANICS.md's "dual
/// generation" (the other being combat/action-generated gains, see
/// `combat::attack_system`'s `OD_GAIN_PER_HIT`).
pub fn tick_od_regen(delta: Res<DeltaSeconds>, mut query: Query<&mut Od>) {
    let dt = delta.0;
    for mut od in &mut query {
        let amount = od.regen_rate * dt;
        od.gain(amount);
    }
}

/// A single skill's authoritative shape and numbers — content-driven, the
/// same data-not-engine-code principle as `EnemyTemplate`/`MeleeAttack`.
/// Loaded once into a `Res<SkillLibrary>` and looked up by id at cast time,
/// rather than attached per-entity like `MeleeAttack` — a skill's
/// definition doesn't vary per caster, so there's nothing to individualize.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillDefinition {
    pub od_cost: f32,
    pub cooldown: f32,
    pub kind: SkillKind,
}

/// The handful of mechanically distinct shapes a skill can resolve as. A
/// new *shape* here is an engine change; a new skill reusing an existing
/// shape is just a content file — the same data-vs-engine split as
/// `DamageType`/`EffectKind` (see MECHANICS.md's Combat section).
#[derive(Debug, Clone, PartialEq)]
pub enum SkillKind {
    /// Single hit against the nearest valid target in range — same
    /// targeting rule as a basic melee attack (see `combat::attack_system`),
    /// just its own damage/range/effects and an `Od` cost.
    PowerStrike {
        damage: f32,
        damage_type: DamageType,
        range: f32,
        effects: Vec<EffectDefinition>,
    },
    /// Hits every valid target within `radius` of the caster, not just the
    /// nearest one — genuinely different resolution from `PowerStrike`,
    /// not a variant of the same search.
    AoeBurst {
        damage: f32,
        damage_type: DamageType,
        radius: f32,
        effects: Vec<EffectDefinition>,
    },
    /// No target at all — applies `effect` to the caster's own
    /// `ActiveEffects` (e.g. a temporary crit-chance or move-speed buff).
    SelfBuff { effect: EffectDefinition },
}

/// Maps a skill's content-file key (e.g. `"power_strike"`) to its
/// definition — a resource, not a component, since the definition itself
/// is identical for every caster (see `SkillDefinition`'s doc comment).
#[derive(Resource, Debug, Default, Clone)]
pub struct SkillLibrary(pub HashMap<String, SkillDefinition>);

/// A character's currently-known skills, gated behind spending
/// `UnspentSkillPoints` in the not-yet-built (M8) skill-tree UI — see
/// DESIGN.md's menus list. Keyed by the skill's content-file id, valued by
/// its upgrade level (starts at 1 once known; there's no "known but
/// unranked" state). Empty for every character right now: nothing is
/// castable until that UI exists to spend points and populate this — same
/// "accumulate now, gated later" shape as M5's `Stats`/`UnspentStatPoints`,
/// not a gap to patch around before then.
#[derive(Component, Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnownSkills(pub HashMap<String, u32>);

/// Points banked from leveling up, spent to unlock/upgrade a skill once the
/// M8 skill-tree UI exists — granted by `progression::grant_xp` alongside
/// `UnspentStatPoints`, same per-level-up shape.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UnspentSkillPoints(pub u32);

/// Remaining cooldown, in seconds, per skill id currently on cooldown for
/// this caster. Absence of an entry means "ready", not "zero cooldown" —
/// `tick_skill_cooldowns` drops an entry entirely once it reaches zero,
/// the same tick-then-drop shape as status-effect expiry.
#[derive(Component, Debug, Default, Clone, PartialEq)]
pub struct SkillCooldowns(pub HashMap<String, f32>);

pub fn tick_skill_cooldowns(delta: Res<DeltaSeconds>, mut query: Query<&mut SkillCooldowns>) {
    let dt = delta.0;
    for mut cooldowns in &mut query {
        cooldowns.0.retain(|_, remaining| {
            *remaining -= dt;
            *remaining > 0.0
        });
    }
}

#[derive(Message, Debug, Clone)]
pub struct SkillCastRequested {
    pub caster: Entity,
    pub skill_id: String,
}

/// A downed or stunned caster can't cast, same as `combat::Attackers` for
/// basic attacks. The `Level`/`UnspentStatPoints`/`UnspentSkillPoints`
/// triple is `Option` since only players have them — used to grant XP on a
/// killing blow, mirroring `combat::attack_system`.
type SkillCasters<'w, 's> = Query<
    'w,
    's,
    (
        &'static Position,
        &'static mut Od,
        &'static mut SkillCooldowns,
        &'static KnownSkills,
        &'static CombatStats,
        Has<Player>,
        Option<&'static mut Level>,
        Option<&'static mut UnspentStatPoints>,
        Option<&'static mut UnspentSkillPoints>,
    ),
    (Without<Downed>, Without<Stunned>),
>;

/// A downed target can't be targeted, same rule as `combat::AttackTargets`.
type SkillTargets<'w, 's> =
    Query<'w, 's, (Entity, &'static Position, Has<Player>), (With<Health>, Without<Downed>)>;

/// Resolves queued `SkillCastRequested`s: validates the caster actually
/// knows the skill, isn't on cooldown for it, and has enough `Od`, then
/// dispatches on `SkillKind`. Mirrors `combat::attack_system`'s no-PvP rule
/// (a `Player` caster never selects another `Player` as a target) and
/// crit/resistance resolution for both attack shapes; `SelfBuff` has no
/// target at all.
#[allow(clippy::too_many_lines)] // a genuine three-way dispatch, not something worth splitting further
pub fn skill_cast_system(
    mut events: MessageReader<SkillCastRequested>,
    library: Res<SkillLibrary>,
    mut casters: SkillCasters,
    targets: SkillTargets,
    mut healths: Query<(&mut Health, Option<&Resistances>, Option<&XpReward>)>,
    mut all_effects: Query<&mut ActiveEffects>,
) {
    let mut rng = rand::rng();
    for event in events.read() {
        let Some(definition) = library.0.get(&event.skill_id) else {
            continue;
        };
        let Ok((
            caster_pos,
            mut od,
            mut cooldowns,
            known,
            caster_stats,
            caster_is_player,
            mut caster_level,
            mut caster_stat_points,
            mut caster_skill_points,
        )) = casters.get_mut(event.caster)
        else {
            continue;
        };

        if !known.0.contains_key(&event.skill_id) {
            continue;
        }
        if cooldowns.0.contains_key(&event.skill_id) {
            continue;
        }
        if !od.try_spend(definition.od_cost) {
            continue;
        }
        cooldowns
            .0
            .insert(event.skill_id.clone(), definition.cooldown);

        match &definition.kind {
            SkillKind::PowerStrike {
                damage,
                damage_type,
                range,
                effects,
            } => {
                let Some(target) =
                    nearest_target(&targets, event.caster, caster_pos, caster_is_player, *range)
                else {
                    continue;
                };
                resolve_hit(
                    event.caster,
                    target,
                    *damage,
                    damage_type,
                    effects,
                    caster_stats,
                    &mut rng,
                    &mut healths,
                    &mut all_effects,
                    caster_level.as_deref_mut(),
                    caster_stat_points.as_deref_mut(),
                    caster_skill_points.as_deref_mut(),
                );
            }
            SkillKind::AoeBurst {
                damage,
                damage_type,
                radius,
                effects,
            } => {
                let hit_targets = targets_in_radius(
                    &targets,
                    event.caster,
                    caster_pos,
                    caster_is_player,
                    *radius,
                );
                for target in hit_targets {
                    resolve_hit(
                        event.caster,
                        target,
                        *damage,
                        damage_type,
                        effects,
                        caster_stats,
                        &mut rng,
                        &mut healths,
                        &mut all_effects,
                        caster_level.as_deref_mut(),
                        caster_stat_points.as_deref_mut(),
                        caster_skill_points.as_deref_mut(),
                    );
                }
            }
            SkillKind::SelfBuff { effect } => {
                if let Ok(mut caster_effects) = all_effects.get_mut(event.caster) {
                    caster_effects.apply(effect.clone());
                }
            }
        }
    }
}

/// Nearest valid target within `range` of `caster_pos` — same filter chain
/// as `combat::attack_system`'s targeting (excludes the caster itself,
/// applies the no-PvP rule).
fn nearest_target(
    targets: &SkillTargets,
    caster: Entity,
    caster_pos: &Position,
    caster_is_player: bool,
    range: f32,
) -> Option<Entity> {
    targets
        .iter()
        .filter(|(entity, _, _)| *entity != caster)
        .filter(|(_, _, target_is_player)| !(caster_is_player && *target_is_player))
        .filter(|(_, pos, _)| caster_pos.distance(pos) <= range)
        .min_by(|(_, a, _), (_, b, _)| caster_pos.distance(a).total_cmp(&caster_pos.distance(b)))
        .map(|(entity, _, _)| entity)
}

/// Every valid target within `radius` of `caster_pos` — same filters as
/// `nearest_target`, but collects all matches instead of the closest one.
fn targets_in_radius(
    targets: &SkillTargets,
    caster: Entity,
    caster_pos: &Position,
    caster_is_player: bool,
    radius: f32,
) -> Vec<Entity> {
    targets
        .iter()
        .filter(|(entity, _, _)| *entity != caster)
        .filter(|(_, _, target_is_player)| !(caster_is_player && *target_is_player))
        .filter(|(_, pos, _)| caster_pos.distance(pos) <= radius)
        .map(|(entity, _, _)| entity)
        .collect()
}

/// Applies one skill hit to `target`: resolves damage through crit/
/// resistance exactly like `combat::attack_system`, rolls each of
/// `effects` independently against its own `chance`, and grants the
/// killing blow's `XpReward` to the caster if they have `Level`.
#[allow(clippy::too_many_arguments)] // mirrors combat::attack_system's inherent per-hit resolution shape
fn resolve_hit(
    caster: Entity,
    target: Entity,
    damage: f32,
    damage_type: &DamageType,
    effects: &[EffectDefinition],
    caster_stats: &CombatStats,
    rng: &mut impl Rng,
    healths: &mut Query<(&mut Health, Option<&Resistances>, Option<&XpReward>)>,
    all_effects: &mut Query<&mut ActiveEffects>,
    caster_level: Option<&mut Level>,
    caster_stat_points: Option<&mut UnspentStatPoints>,
    caster_skill_points: Option<&mut UnspentSkillPoints>,
) {
    let Ok((mut health, resistances, xp_reward)) = healths.get_mut(target) else {
        return;
    };
    let no_resistances = Resistances::default();
    let resistances = resistances.unwrap_or(&no_resistances);
    let amount = resolve_damage(damage, damage_type, caster_stats, resistances, rng);
    apply_damage(&mut health, amount);

    if let Ok([mut caster_effects, mut target_effects]) = all_effects.get_many_mut([caster, target])
    {
        for effect in effects {
            if rng.random_bool(effect.chance as f64) {
                match effect.applies_to {
                    crate::status_effect::EffectTarget::Attacker => {
                        caster_effects.apply(effect.clone())
                    }
                    crate::status_effect::EffectTarget::Target => {
                        target_effects.apply(effect.clone())
                    }
                }
            }
        }
    }

    if health.is_dead() {
        if let (Some(xp_reward), Some(level), Some(stat_points), Some(skill_points)) = (
            xp_reward,
            caster_level,
            caster_stat_points,
            caster_skill_points,
        ) {
            grant_xp(level, stat_points, skill_points, xp_reward.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status_effect::{EffectKind, EffectTarget, StackMode};
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn od_gain_clamps_at_max() {
        let mut od = Od::new(100.0, 10.0);
        od.gain(50.0);
        assert_eq!(od.current, 100.0);
    }

    #[test]
    fn try_spend_fails_and_leaves_current_unchanged_when_insufficient() {
        let mut od = Od::new(100.0, 10.0);
        od.current = 20.0;
        assert!(!od.try_spend(50.0));
        assert_eq!(od.current, 20.0);
    }

    #[test]
    fn try_spend_succeeds_and_deducts_when_sufficient() {
        let mut od = Od::new(100.0, 10.0);
        assert!(od.try_spend(30.0));
        assert_eq!(od.current, 70.0);
    }

    #[test]
    fn tick_od_regen_adds_regen_rate_times_delta_clamped_to_max() {
        let mut world = World::new();
        world.insert_resource(DeltaSeconds(2.0));
        let mut od = Od::new(100.0, 10.0);
        od.current = 85.0;
        let entity = world.spawn(od).id();

        let _ = world.run_system_once(tick_od_regen);

        assert_eq!(world.get::<Od>(entity).unwrap().current, 100.0);
    }

    #[test]
    fn tick_skill_cooldowns_drops_entries_once_they_reach_zero() {
        let mut world = World::new();
        world.insert_resource(DeltaSeconds(1.0));
        let mut cooldowns = SkillCooldowns::default();
        cooldowns.0.insert("power_strike".to_string(), 1.5);
        cooldowns.0.insert("aoe_burst".to_string(), 0.5);
        let entity = world.spawn(cooldowns).id();

        let _ = world.run_system_once(tick_skill_cooldowns);

        let cooldowns = &world.get::<SkillCooldowns>(entity).unwrap().0;
        assert!((cooldowns["power_strike"] - 0.5).abs() < 1e-4);
        assert!(!cooldowns.contains_key("aoe_burst"));
    }

    fn power_strike_library() -> SkillLibrary {
        let mut lib = SkillLibrary::default();
        lib.0.insert(
            "power_strike".to_string(),
            SkillDefinition {
                od_cost: 20.0,
                cooldown: 2.0,
                kind: SkillKind::PowerStrike {
                    damage: 50.0,
                    damage_type: DamageType("primal".to_string()),
                    range: 100.0,
                    effects: vec![],
                },
            },
        );
        lib
    }

    fn caster_bundle(
        known: &[&str],
    ) -> (
        Position,
        Od,
        SkillCooldowns,
        KnownSkills,
        CombatStats,
        Player,
    ) {
        (
            Position { x: 0.0, y: 0.0 },
            Od::new(100.0, 0.0),
            SkillCooldowns::default(),
            KnownSkills(known.iter().map(|id| (id.to_string(), 1)).collect()),
            CombatStats {
                crit_chance: 0.0,
                crit_multiplier: 1.0,
            },
            Player,
        )
    }

    #[test]
    fn skill_cast_ignores_an_unknown_skill() {
        let mut world = World::new();
        world.insert_resource(power_strike_library());
        let caster = world.spawn(caster_bundle(&[])).id();
        world.init_resource::<Messages<SkillCastRequested>>();
        world
            .resource_mut::<Messages<SkillCastRequested>>()
            .write(SkillCastRequested {
                caster,
                skill_id: "power_strike".to_string(),
            });

        let _ = world.run_system_once(skill_cast_system);

        assert_eq!(world.get::<Od>(caster).unwrap().current, 100.0);
    }

    #[test]
    fn power_strike_spends_od_sets_cooldown_and_damages_nearest_target() {
        let mut world = World::new();
        world.insert_resource(power_strike_library());
        let caster = world.spawn(caster_bundle(&["power_strike"])).id();
        let target = world
            .spawn((
                Position { x: 10.0, y: 0.0 },
                Health::new(100.0),
                Resistances::default(),
            ))
            .id();
        world.init_resource::<Messages<SkillCastRequested>>();
        world
            .resource_mut::<Messages<SkillCastRequested>>()
            .write(SkillCastRequested {
                caster,
                skill_id: "power_strike".to_string(),
            });

        let _ = world.run_system_once(skill_cast_system);

        assert_eq!(world.get::<Od>(caster).unwrap().current, 80.0);
        assert!(world
            .get::<SkillCooldowns>(caster)
            .unwrap()
            .0
            .contains_key("power_strike"));
        assert_eq!(world.get::<Health>(target).unwrap().current, 50.0);
    }

    #[test]
    fn skill_cast_ignores_insufficient_od() {
        let mut world = World::new();
        world.insert_resource(power_strike_library());
        let mut bundle = caster_bundle(&["power_strike"]);
        bundle.1.current = 5.0;
        let caster = world.spawn(bundle).id();
        world.init_resource::<Messages<SkillCastRequested>>();
        world
            .resource_mut::<Messages<SkillCastRequested>>()
            .write(SkillCastRequested {
                caster,
                skill_id: "power_strike".to_string(),
            });

        let _ = world.run_system_once(skill_cast_system);

        assert_eq!(world.get::<Od>(caster).unwrap().current, 5.0);
        assert!(world.get::<SkillCooldowns>(caster).unwrap().0.is_empty());
    }

    #[test]
    fn skill_cast_ignores_a_skill_still_on_cooldown() {
        let mut world = World::new();
        world.insert_resource(power_strike_library());
        let mut bundle = caster_bundle(&["power_strike"]);
        bundle.2 .0.insert("power_strike".to_string(), 1.0);
        let caster = world.spawn(bundle).id();
        world.init_resource::<Messages<SkillCastRequested>>();
        world
            .resource_mut::<Messages<SkillCastRequested>>()
            .write(SkillCastRequested {
                caster,
                skill_id: "power_strike".to_string(),
            });

        let _ = world.run_system_once(skill_cast_system);

        assert_eq!(world.get::<Od>(caster).unwrap().current, 100.0);
    }

    #[test]
    fn aoe_burst_hits_every_target_in_radius_not_just_nearest() {
        let mut world = World::new();
        let mut lib = SkillLibrary::default();
        lib.0.insert(
            "aoe_burst".to_string(),
            SkillDefinition {
                od_cost: 30.0,
                cooldown: 3.0,
                kind: SkillKind::AoeBurst {
                    damage: 20.0,
                    damage_type: DamageType("primal".to_string()),
                    radius: 50.0,
                    effects: vec![],
                },
            },
        );
        world.insert_resource(lib);
        let caster = world.spawn(caster_bundle(&["aoe_burst"])).id();
        let near = world
            .spawn((
                Position { x: 10.0, y: 0.0 },
                Health::new(100.0),
                Resistances::default(),
            ))
            .id();
        let far_but_in_radius = world
            .spawn((
                Position { x: 40.0, y: 0.0 },
                Health::new(100.0),
                Resistances::default(),
            ))
            .id();
        let outside_radius = world
            .spawn((
                Position { x: 200.0, y: 0.0 },
                Health::new(100.0),
                Resistances::default(),
            ))
            .id();
        world.init_resource::<Messages<SkillCastRequested>>();
        world
            .resource_mut::<Messages<SkillCastRequested>>()
            .write(SkillCastRequested {
                caster,
                skill_id: "aoe_burst".to_string(),
            });

        let _ = world.run_system_once(skill_cast_system);

        assert_eq!(world.get::<Health>(near).unwrap().current, 80.0);
        assert_eq!(
            world.get::<Health>(far_but_in_radius).unwrap().current,
            80.0
        );
        assert_eq!(world.get::<Health>(outside_radius).unwrap().current, 100.0);
    }

    #[test]
    fn self_buff_applies_effect_to_caster_with_no_target() {
        let mut world = World::new();
        let mut lib = SkillLibrary::default();
        lib.0.insert(
            "berserk".to_string(),
            SkillDefinition {
                od_cost: 25.0,
                cooldown: 5.0,
                kind: SkillKind::SelfBuff {
                    effect: EffectDefinition {
                        id: "berserk".to_string(),
                        kind: EffectKind::StatModifier {
                            stat: crate::status_effect::Stat::CritChance,
                        },
                        duration: 4.0,
                        magnitude: 0.2,
                        stack_mode: StackMode::Independent,
                        applies_to: EffectTarget::Attacker,
                        chance: 1.0,
                    },
                },
            },
        );
        world.insert_resource(lib);
        let caster = world
            .spawn((caster_bundle(&["berserk"]), ActiveEffects::default()))
            .id();
        world.init_resource::<Messages<SkillCastRequested>>();
        world
            .resource_mut::<Messages<SkillCastRequested>>()
            .write(SkillCastRequested {
                caster,
                skill_id: "berserk".to_string(),
            });

        let _ = world.run_system_once(skill_cast_system);

        assert_eq!(world.get::<Od>(caster).unwrap().current, 75.0);
        assert!(!world.get::<ActiveEffects>(caster).unwrap().0.is_empty());
    }
}
