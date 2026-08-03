use bevy_ecs::prelude::*;
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

#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct MeleeAttack {
    pub range: f32,
    pub damage: f32,
    pub cooldown: f32,
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
/// and applies damage to it.
pub fn attack_system(
    mut events: MessageReader<AttackRequested>,
    mut attackers: Query<(&Position, &MeleeAttack, &mut AttackTimer)>,
    targets: Query<(Entity, &Position), With<Health>>,
    mut healths: Query<&mut Health>,
) {
    for event in events.read() {
        let Ok((attacker_pos, melee, mut timer)) = attackers.get_mut(event.attacker) else {
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

        if let Ok(mut health) = healths.get_mut(target) {
            apply_damage(&mut health, melee.damage);
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
