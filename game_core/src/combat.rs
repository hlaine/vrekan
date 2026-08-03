use std::collections::HashMap;

use bevy_ecs::prelude::*;
use rand::{Rng, RngExt};
use serde::{Deserialize, Serialize};

use crate::movement::Position;
use crate::DeltaSeconds;

#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    pub fn new(max: f32) -> Self {
        Health { current: max, max }
    }

    pub fn is_dead(&self) -> bool {
        self.current <= 0.0
    }
}

/// Reduces `health.current` by `amount`, clamped at zero. `amount` is
/// incoming damage and must be non-negative — a negative value is a caller
/// bug, not a recoverable game state.
pub fn apply_damage(health: &mut Health, amount: f32) {
    assert!(
        amount >= 0.0,
        "damage amount must be non-negative, got {amount}"
    );
    health.current = (health.current - amount).max(0.0);
}

/// Identifies a kind of damage (e.g. "primal", "holy") for resistance
/// lookups. A data-keyed string rather than a fixed enum, so new damage
/// types can be added as content, not engine changes — see DESIGN.md's
/// Damage & faction system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DamageType(pub String);

/// Per-`DamageType` resistance fractions for an entity that can take
/// damage. A type with no entry defaults to `0.0` (no resistance) — most
/// entities won't need to list every type, only the ones where they differ
/// from that default.
#[derive(Component, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Resistances(pub HashMap<DamageType, f32>);

impl Resistances {
    /// Resistance fraction for `damage_type`, clamped to `[-1.0, 1.0]`
    /// (0.5 = 50% reduction, negative = weakness/bonus damage taken)
    /// regardless of what content data specifies — a safeguard against a
    /// bad RON value producing negative damage or absurd amplification, see
    /// MECHANICS.md's damage formula.
    pub fn get(&self, damage_type: &DamageType) -> f32 {
        self.0
            .get(damage_type)
            .copied()
            .unwrap_or(0.0)
            .clamp(-1.0, 1.0)
    }
}

/// Character-level combat stats. Crit chance/multiplier apply to every
/// attack this entity lands, regardless of which specific attack (melee,
/// later ranged/spells) is used — see MECHANICS.md's Combat section.
#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CombatStats {
    pub crit_chance: f32,
    pub crit_multiplier: f32,
}

/// Resolves MECHANICS.md's damage formula: a crit is rolled and applied to
/// `base_damage` first, then the target's resistance for `damage_type` is
/// applied. `rng` is generic so tests can inject a seeded RNG for
/// deterministic crit/non-crit assertions — production callers use
/// `rand::rng()` (see `attack_system`).
pub fn resolve_damage(
    base_damage: f32,
    damage_type: &DamageType,
    attacker_stats: &CombatStats,
    target_resistances: &Resistances,
    rng: &mut impl Rng,
) -> f32 {
    let is_crit = rng.random_bool(attacker_stats.crit_chance as f64);
    let after_crit = if is_crit {
        base_damage * attacker_stats.crit_multiplier
    } else {
        base_damage
    };
    after_crit * (1.0 - target_resistances.get(damage_type))
}

#[derive(Component, Debug, Clone, PartialEq)]
pub struct MeleeAttack {
    pub range: f32,
    pub damage: f32,
    pub cooldown: f32,
    pub damage_type: DamageType,
}

/// Seconds remaining before this entity can attack again; `0.0` means ready.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq)]
pub struct AttackTimer(pub f32);

#[derive(Message, Debug, Clone, Copy)]
pub struct AttackRequested {
    pub attacker: Entity,
}

pub fn tick_attack_timers(delta: Res<DeltaSeconds>, mut query: Query<&mut AttackTimer>) {
    let dt = delta.0;
    for mut timer in &mut query {
        timer.0 = (timer.0 - dt).max(0.0);
    }
}

/// Resolves queued `AttackRequested` events: if the attacker is off cooldown,
/// finds the nearest other entity with `Health` within `MeleeAttack::range`
/// and applies damage to it, resolved via `resolve_damage` (crit, then the
/// target's resistance for the attack's `DamageType`).
pub fn attack_system(
    mut events: MessageReader<AttackRequested>,
    mut attackers: Query<(&Position, &MeleeAttack, &CombatStats, &mut AttackTimer)>,
    targets: Query<(Entity, &Position), With<Health>>,
    mut healths: Query<(&mut Health, Option<&Resistances>)>,
) {
    let mut rng = rand::rng();
    for event in events.read() {
        let Ok((attacker_pos, melee, attacker_stats, mut timer)) =
            attackers.get_mut(event.attacker)
        else {
            continue;
        };
        if timer.0 > 0.0 {
            continue;
        }

        let nearest = targets
            .iter()
            .filter(|(entity, _)| *entity != event.attacker)
            .filter(|(_, pos)| attacker_pos.distance(pos) <= melee.range)
            .min_by(|(_, a), (_, b)| {
                attacker_pos
                    .distance(a)
                    .total_cmp(&attacker_pos.distance(b))
            })
            .map(|(entity, _)| entity);

        let Some(target) = nearest else {
            continue;
        };

        if let Ok((mut health, resistances)) = healths.get_mut(target) {
            let no_resistances = Resistances::default();
            let resistances = resistances.unwrap_or(&no_resistances);
            let amount = resolve_damage(
                melee.damage,
                &melee.damage_type,
                attacker_stats,
                resistances,
                &mut rng,
            );
            apply_damage(&mut health, amount);
            timer.0 = melee.cooldown;
        }
    }
}

pub fn death_system(mut commands: Commands, query: Query<(Entity, &Health)>) {
    for (entity, health) in &query {
        if health.is_dead() {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;
    use rand::rngs::SmallRng;
    use rand::SeedableRng;

    fn primal() -> DamageType {
        DamageType("primal".to_string())
    }

    #[test]
    fn resolve_damage_applies_crit_multiplier_when_crit_chance_is_guaranteed() {
        let mut rng = SmallRng::seed_from_u64(0);
        let stats = CombatStats {
            crit_chance: 1.0,
            crit_multiplier: 2.0,
        };

        let damage = resolve_damage(10.0, &primal(), &stats, &Resistances::default(), &mut rng);

        assert_eq!(damage, 20.0);
    }

    #[test]
    fn resolve_damage_skips_crit_multiplier_when_crit_chance_is_zero() {
        let mut rng = SmallRng::seed_from_u64(0);
        let stats = CombatStats {
            crit_chance: 0.0,
            crit_multiplier: 2.0,
        };

        let damage = resolve_damage(10.0, &primal(), &stats, &Resistances::default(), &mut rng);

        assert_eq!(damage, 10.0);
    }

    #[test]
    fn resolve_damage_applies_resistance_reduction() {
        let mut rng = SmallRng::seed_from_u64(0);
        let stats = CombatStats {
            crit_chance: 0.0,
            crit_multiplier: 1.0,
        };
        let holy = DamageType("holy".to_string());
        let resistances = Resistances(HashMap::from([(holy.clone(), 0.5)]));

        let damage = resolve_damage(10.0, &holy, &stats, &resistances, &mut rng);

        assert_eq!(damage, 5.0);
    }

    #[test]
    fn resolve_damage_negative_resistance_amplifies_damage() {
        let mut rng = SmallRng::seed_from_u64(0);
        let stats = CombatStats {
            crit_chance: 0.0,
            crit_multiplier: 1.0,
        };
        let holy = DamageType("holy".to_string());
        let resistances = Resistances(HashMap::from([(holy.clone(), -1.0)]));

        let damage = resolve_damage(10.0, &holy, &stats, &resistances, &mut rng);

        assert_eq!(damage, 20.0);
    }

    #[test]
    fn resistances_get_clamps_values_beyond_the_valid_range() {
        let holy = DamageType("holy".to_string());
        let resistances = Resistances(HashMap::from([(holy.clone(), 5.0)]));

        assert_eq!(resistances.get(&holy), 1.0);
    }

    #[test]
    fn resistances_get_defaults_to_zero_for_an_unlisted_damage_type() {
        let resistances = Resistances::default();

        assert_eq!(resistances.get(&DamageType("holy".to_string())), 0.0);
    }

    #[test]
    fn apply_damage_reduces_current_health() {
        let mut health = Health::new(100.0);
        apply_damage(&mut health, 30.0);
        assert_eq!(health.current, 70.0);
    }

    #[test]
    fn apply_damage_clamps_at_zero_on_overkill() {
        let mut health = Health::new(50.0);
        apply_damage(&mut health, 999.0);
        assert_eq!(health.current, 0.0);
        assert!(health.is_dead());
    }

    #[test]
    fn apply_damage_exact_lethal_amount_kills() {
        let mut health = Health::new(40.0);
        apply_damage(&mut health, 40.0);
        assert!(health.is_dead());
    }

    #[test]
    fn apply_damage_zero_amount_is_a_no_op() {
        let mut health = Health::new(20.0);
        apply_damage(&mut health, 0.0);
        assert_eq!(health.current, 20.0);
        assert!(!health.is_dead());
    }

    #[test]
    #[should_panic(expected = "non-negative")]
    fn apply_damage_rejects_negative_amount() {
        let mut health = Health::new(20.0);
        apply_damage(&mut health, -5.0);
    }

    #[test]
    fn attack_system_damages_nearest_target_in_range_and_starts_cooldown() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();

        let attacker = world
            .spawn((
                Position { x: 0.0, y: 0.0 },
                MeleeAttack {
                    range: 5.0,
                    damage: 10.0,
                    cooldown: 1.0,
                    damage_type: primal(),
                },
                CombatStats {
                    crit_chance: 0.0,
                    crit_multiplier: 1.0,
                },
                AttackTimer(0.0),
            ))
            .id();
        let near_target = world
            .spawn((Position { x: 2.0, y: 0.0 }, Health::new(30.0)))
            .id();
        let far_target = world
            .spawn((Position { x: 100.0, y: 0.0 }, Health::new(30.0)))
            .id();

        world
            .resource_mut::<Messages<AttackRequested>>()
            .write(AttackRequested { attacker });

        let _ = world.run_system_once(attack_system);

        assert_eq!(world.get::<Health>(near_target).unwrap().current, 20.0);
        assert_eq!(world.get::<Health>(far_target).unwrap().current, 30.0);
        assert_eq!(world.get::<AttackTimer>(attacker).unwrap().0, 1.0);
    }

    #[test]
    fn attack_system_ignores_requests_still_on_cooldown() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();

        let attacker = world
            .spawn((
                Position { x: 0.0, y: 0.0 },
                MeleeAttack {
                    range: 5.0,
                    damage: 10.0,
                    cooldown: 1.0,
                    damage_type: primal(),
                },
                CombatStats {
                    crit_chance: 0.0,
                    crit_multiplier: 1.0,
                },
                AttackTimer(0.4),
            ))
            .id();
        let target = world
            .spawn((Position { x: 1.0, y: 0.0 }, Health::new(30.0)))
            .id();

        world
            .resource_mut::<Messages<AttackRequested>>()
            .write(AttackRequested { attacker });

        let _ = world.run_system_once(attack_system);

        assert_eq!(world.get::<Health>(target).unwrap().current, 30.0);
    }

    #[test]
    fn death_system_despawns_entities_at_zero_health() {
        let mut world = World::new();
        let alive = world.spawn(Health::new(10.0)).id();
        let dead = world.spawn(Health::new(0.0)).id();

        let _ = world.run_system_once(death_system);

        assert!(world.get_entity(alive).is_ok());
        assert!(world.get_entity(dead).is_err());
    }
}
