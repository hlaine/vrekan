use bevy::prelude::*;
use game_core::combat::{
    attack_system, death_system, tick_attack_timers, AttackRequested, AttackTimer, Health,
    MeleeAttack,
};
use game_core::enemy::{ai_system, Aggro, Enemy};
use game_core::movement::{movement_system, MoveSpeed, Position, Velocity};
use game_core::player::Player;
use game_core::DeltaSeconds;

const PLAYER_SPEED: f32 = 200.0;
const PLAYER_MAX_HEALTH: f32 = 100.0;
const PLAYER_ATTACK_RANGE: f32 = 60.0;
const PLAYER_ATTACK_DAMAGE: f32 = 15.0;
const PLAYER_ATTACK_COOLDOWN: f32 = 0.4;

const ENEMY_SPEED: f32 = 90.0;
const ENEMY_MAX_HEALTH: f32 = 40.0;
const ENEMY_ATTACK_RANGE: f32 = 40.0;
const ENEMY_ATTACK_DAMAGE: f32 = 8.0;
const ENEMY_ATTACK_COOLDOWN: f32 = 1.0;
// Kept below the player-enemy spawn distance (~224 units) so an idle player
// isn't auto-aggro'd; the player has to approach before the enemy engages.
const ENEMY_AGGRO_RANGE: f32 = 150.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Vrekan".into(),
                ..default()
            }),
            ..default()
        }))
        .init_resource::<DeltaSeconds>()
        .add_message::<AttackRequested>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                update_delta_seconds,
                player_input_system,
                ai_system,
                movement_system,
                tick_attack_timers,
                attack_system,
                death_system,
                sync_transform_system,
            )
                .chain(),
        )
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);

    commands.spawn((
        Player,
        Position { x: 0.0, y: 0.0 },
        Velocity::ZERO,
        MoveSpeed(PLAYER_SPEED),
        Health::new(PLAYER_MAX_HEALTH),
        MeleeAttack {
            range: PLAYER_ATTACK_RANGE,
            damage: PLAYER_ATTACK_DAMAGE,
            cooldown: PLAYER_ATTACK_COOLDOWN,
        },
        AttackTimer(0.0),
        Sprite::from_color(Color::srgb(0.2, 0.7, 0.3), Vec2::splat(32.0)),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    commands.spawn((
        Enemy,
        Position { x: 200.0, y: 100.0 },
        Velocity::ZERO,
        MoveSpeed(ENEMY_SPEED),
        Health::new(ENEMY_MAX_HEALTH),
        MeleeAttack {
            range: ENEMY_ATTACK_RANGE,
            damage: ENEMY_ATTACK_DAMAGE,
            cooldown: ENEMY_ATTACK_COOLDOWN,
        },
        AttackTimer(0.0),
        Aggro {
            range: ENEMY_AGGRO_RANGE,
        },
        Sprite::from_color(Color::srgb(0.8, 0.15, 0.15), Vec2::splat(28.0)),
        Transform::from_xyz(200.0, 100.0, 0.0),
    ));
}

fn update_delta_seconds(time: Res<Time>, mut delta: ResMut<DeltaSeconds>) {
    delta.0 = time.delta_secs();
}

fn player_input_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut attack_events: MessageWriter<AttackRequested>,
    mut query: Query<(Entity, &mut Velocity, &MoveSpeed), With<Player>>,
) {
    let Ok((entity, mut velocity, speed)) = query.single_mut() else {
        return;
    };

    let mut direction = Vec2::ZERO;
    if keyboard.pressed(KeyCode::KeyW) {
        direction.y += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyS) {
        direction.y -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyA) {
        direction.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyD) {
        direction.x += 1.0;
    }

    *velocity = if direction == Vec2::ZERO {
        Velocity::ZERO
    } else {
        let normalized = direction.normalize();
        Velocity {
            x: normalized.x * speed.0,
            y: normalized.y * speed.0,
        }
    };

    if keyboard.just_pressed(KeyCode::Space) {
        attack_events.write(AttackRequested { attacker: entity });
    }
}

fn sync_transform_system(mut query: Query<(&Position, &mut Transform)>) {
    for (position, mut transform) in &mut query {
        transform.translation.x = position.x;
        transform.translation.y = position.y;
    }
}
