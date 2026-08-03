use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::combat::{AttackRequested, MeleeAttack};
use crate::movement::{Facing, MoveSpeed, Position, Velocity};
use crate::player::{Downed, Player};
use crate::status_effect::Stunned;

#[derive(Component, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Enemy;

/// Which content template (`content::EnemyTemplate`, keyed by filename stem —
/// e.g. "converted_farmer") this enemy was spawned from. Replicated so
/// clients can pick the right appearance for a server-spawned enemy without
/// needing the full template sent over the wire — the client already has
/// the same `.ron` files loaded locally for that lookup.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnemyKind(pub String);

/// Distance within which an enemy notices and chases the player. Should
/// generally be larger than `MeleeAttack::range`, or the enemy never moves
/// close enough to attack.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Aggro {
    pub range: f32,
}

type EnemyQueryData = (
    Entity,
    &'static Position,
    &'static mut Velocity,
    &'static mut Facing,
    &'static MoveSpeed,
    &'static MeleeAttack,
    &'static Aggro,
    Has<Stunned>,
);

/// Simple chase-and-attack pattern: each enemy targets whichever non-downed
/// player is nearest to it (independently per enemy, so different enemies
/// can chase different players), idles outside `Aggro::range`, closes the
/// distance while outside melee range, and attacks once in range. A downed
/// player is out of combat entirely (see MECHANICS.md) and a party with no
/// eligible player at all (zero connected, or the sole player downed) means
/// every enemy just idles. `Facing` tracks the same movement direction as
/// `velocity` (a no-op while idle or attacking, since `velocity` is zero
/// then) — same movement-derived mechanism as players, see MECHANICS.md's
/// Combat section.
///
/// A stunned enemy is force-stopped and skipped entirely, every tick — not
/// filtered out of the query. `ai_system` runs unconditionally each tick
/// and is what feeds `sync_enemy_velocity_to_physics`, so excluding a
/// stunned enemy from the query would leave its stale `Velocity` copied
/// into `LinearVelocity` forever, the same "forgot to re-zero every tick"
/// bug already found and fixed for downed players (see DECISIONS.md).
pub fn ai_system(
    player_query: Query<&Position, (With<Player>, Without<Downed>)>,
    mut enemies: Query<EnemyQueryData, With<Enemy>>,
    mut attack_events: MessageWriter<AttackRequested>,
) {
    for (entity, enemy_pos, mut velocity, mut facing, speed, melee, aggro, stunned) in &mut enemies
    {
        if stunned {
            *velocity = Velocity::ZERO;
            continue;
        }

        let nearest_player = player_query
            .iter()
            .min_by(|a, b| enemy_pos.distance(a).total_cmp(&enemy_pos.distance(b)));

        let Some(player_pos) = nearest_player else {
            *velocity = Velocity::ZERO;
            continue;
        };

        let distance = enemy_pos.distance(player_pos);
        if distance > aggro.range {
            *velocity = Velocity::ZERO;
        } else if distance > melee.range {
            *velocity = Velocity::toward(enemy_pos, player_pos, speed.0);
        } else {
            *velocity = Velocity::ZERO;
            attack_events.write(AttackRequested { attacker: entity });
        }
        facing.update_from_direction(velocity.x, velocity.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::DamageType;
    use bevy_ecs::system::RunSystemOnce;

    fn spawn_enemy(world: &mut World, position: Position, aggro_range: f32) -> Entity {
        world
            .spawn((
                Enemy,
                position,
                Velocity::ZERO,
                Facing::default(),
                MoveSpeed(3.0),
                MeleeAttack {
                    range: 1.0,
                    damage: 5.0,
                    cooldown: 1.0,
                    damage_type: DamageType("primal".to_string()),
                    effects: vec![],
                },
                Aggro { range: aggro_range },
            ))
            .id()
    }

    #[test]
    fn enemy_idles_when_player_outside_aggro_range() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        world.spawn((Player, Position { x: 100.0, y: 0.0 }));
        let enemy = spawn_enemy(&mut world, Position { x: 0.0, y: 0.0 }, 10.0);

        let _ = world.run_system_once(ai_system);

        assert_eq!(*world.get::<Velocity>(enemy).unwrap(), Velocity::ZERO);
    }

    #[test]
    fn enemy_chases_player_within_aggro_but_outside_melee_range() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        world.spawn((Player, Position { x: 10.0, y: 0.0 }));
        let enemy = spawn_enemy(&mut world, Position { x: 0.0, y: 0.0 }, 20.0);

        let _ = world.run_system_once(ai_system);

        let velocity = *world.get::<Velocity>(enemy).unwrap();
        assert!(velocity.x > 0.0);
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn enemy_facing_tracks_chase_direction() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        world.spawn((Player, Position { x: 10.0, y: 0.0 }));
        let enemy = spawn_enemy(&mut world, Position { x: 0.0, y: 0.0 }, 20.0);

        let _ = world.run_system_once(ai_system);

        let facing = *world.get::<Facing>(enemy).unwrap();
        assert!(facing.x > 0.0);
        assert_eq!(facing.y, 0.0);
    }

    #[test]
    fn enemy_facing_holds_steady_while_idle() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        world.spawn((Player, Position { x: 100.0, y: 0.0 }));
        let enemy = spawn_enemy(&mut world, Position { x: 0.0, y: 0.0 }, 10.0);
        world.entity_mut(enemy).insert(Facing { x: 0.0, y: 1.0 });

        let _ = world.run_system_once(ai_system);

        assert_eq!(
            *world.get::<Facing>(enemy).unwrap(),
            Facing { x: 0.0, y: 1.0 }
        );
    }

    #[test]
    fn enemy_freezes_and_skips_targeting_when_stunned() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        world.spawn((Player, Position { x: 0.5, y: 0.0 }));
        let enemy = spawn_enemy(&mut world, Position { x: 0.0, y: 0.0 }, 20.0);
        world.entity_mut(enemy).insert(Stunned);

        let _ = world.run_system_once(ai_system);

        assert_eq!(*world.get::<Velocity>(enemy).unwrap(), Velocity::ZERO);
        let events = world.resource::<Messages<AttackRequested>>();
        let mut cursor = events.get_cursor();
        assert_eq!(cursor.read(events).count(), 0);
    }

    #[test]
    fn enemy_ignores_a_downed_player_even_at_melee_range() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        world.spawn((Player, Downed, Position { x: 0.5, y: 0.0 }));
        let enemy = spawn_enemy(&mut world, Position { x: 0.0, y: 0.0 }, 20.0);

        let _ = world.run_system_once(ai_system);

        assert_eq!(*world.get::<Velocity>(enemy).unwrap(), Velocity::ZERO);
        let events = world.resource::<Messages<AttackRequested>>();
        let mut cursor = events.get_cursor();
        assert_eq!(cursor.read(events).count(), 0);
    }

    #[test]
    fn enemy_targets_nearest_of_two_players() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        // Regression test: `ai_system` used to pick its target via
        // `player_query.single()`, which errors (and made every enemy idle,
        // full stop) as soon as more than one player was connected — this
        // is the exact two-client scenario that broke in practice.
        world.spawn((Player, Position { x: 0.5, y: 0.0 }));
        world.spawn((Player, Position { x: 100.0, y: 0.0 }));
        let enemy = spawn_enemy(&mut world, Position { x: 0.0, y: 0.0 }, 20.0);

        let _ = world.run_system_once(ai_system);

        assert_eq!(*world.get::<Velocity>(enemy).unwrap(), Velocity::ZERO);
        let events = world.resource::<Messages<AttackRequested>>();
        let mut cursor = events.get_cursor();
        assert_eq!(cursor.read(events).count(), 1);
    }

    #[test]
    fn enemy_idles_when_no_players_are_connected() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        let enemy = spawn_enemy(&mut world, Position { x: 0.0, y: 0.0 }, 20.0);

        let _ = world.run_system_once(ai_system);

        assert_eq!(*world.get::<Velocity>(enemy).unwrap(), Velocity::ZERO);
    }

    #[test]
    fn enemy_attacks_and_stops_when_within_melee_range() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        world.spawn((Player, Position { x: 0.5, y: 0.0 }));
        let enemy = spawn_enemy(&mut world, Position { x: 0.0, y: 0.0 }, 20.0);

        let _ = world.run_system_once(ai_system);

        assert_eq!(*world.get::<Velocity>(enemy).unwrap(), Velocity::ZERO);
        let events = world.resource::<Messages<AttackRequested>>();
        let mut cursor = events.get_cursor();
        let sent: Vec<_> = cursor.read(events).collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].attacker, enemy);
    }
}
