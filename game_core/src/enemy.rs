use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::combat::{AttackRequested, MeleeAttack};
use crate::movement::{MoveSpeed, Position, Velocity};
use crate::player::Player;

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
    &'static MoveSpeed,
    &'static MeleeAttack,
    &'static Aggro,
);

/// Simple chase-and-attack pattern: idle outside `Aggro::range`, close the
/// distance while outside melee range, and attack once in range.
pub fn ai_system(
    player_query: Query<&Position, With<Player>>,
    mut enemies: Query<EnemyQueryData, With<Enemy>>,
    mut attack_events: MessageWriter<AttackRequested>,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };

    for (entity, enemy_pos, mut velocity, speed, melee, aggro) in &mut enemies {
        let distance = enemy_pos.distance(player_pos);
        if distance > aggro.range {
            *velocity = Velocity::ZERO;
        } else if distance > melee.range {
            *velocity = Velocity::toward(enemy_pos, player_pos, speed.0);
        } else {
            *velocity = Velocity::ZERO;
            attack_events.write(AttackRequested { attacker: entity });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn spawn_enemy(world: &mut World, position: Position, aggro_range: f32) -> Entity {
        world
            .spawn((
                Enemy,
                position,
                Velocity::ZERO,
                MoveSpeed(3.0),
                MeleeAttack {
                    range: 1.0,
                    damage: 5.0,
                    cooldown: 1.0,
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
