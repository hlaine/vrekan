use bevy_ecs::prelude::*;

use crate::combat::Health;
use crate::movement::Position;
use crate::player::Downed;
use crate::status_effect::Stunned;
use crate::DeltaSeconds;

/// Set (via network input translation in `server`) while a player holds the
/// revive/interact button. Not inherently `Downed`/`Stunned`-safe —
/// `revive_system` filters those out wherever a downed or stunned entity
/// shouldn't be able to act as a reviver, including for itself.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct Reviving;

/// Seconds of channel time accumulated toward reviving this entity. Dropped
/// entirely whenever no ally is currently in range — no partial progress is
/// banked across separate revive attempts, a reasonable starting assumption
/// per MECHANICS.md's open questions, not a settled design.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct ReviveProgress(pub f32);

/// Distance within which a `Reviving` ally counts toward reviving a downed
/// player — matches `PLAYER_ATTACK_RANGE` in `server`.
pub const REVIVE_RANGE: f32 = 60.0;
pub const REVIVE_DURATION_SECS: f32 = 3.0;
/// Fraction of max health a revived player comes back with — see
/// MECHANICS.md's Death, downed state, and revive section.
pub const REVIVE_HEALTH_FRACTION: f32 = 0.5;

type DownedAllies<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Position,
        &'static mut Health,
        Option<&'static mut ReviveProgress>,
    ),
    With<Downed>,
>;

/// A downed or stunned entity can't act as a reviver — see `revive_system`.
type Revivers<'w, 's> =
    Query<'w, 's, &'static Position, (With<Reviving>, Without<Downed>, Without<Stunned>)>;

/// For each downed entity, checks whether any non-downed, non-stunned
/// `Reviving` entity is within `REVIVE_RANGE`. If not, any accumulated
/// `ReviveProgress` is dropped. If so, progress accumulates by
/// `DeltaSeconds`; on reaching `REVIVE_DURATION_SECS` the entity is
/// revived at `REVIVE_HEALTH_FRACTION` of max health and stops being
/// `Downed`.
pub fn revive_system(
    delta: Res<DeltaSeconds>,
    revivers: Revivers,
    mut downed: DownedAllies,
    mut commands: Commands,
) {
    let dt = delta.0;
    for (entity, downed_pos, mut health, progress) in &mut downed {
        let being_revived = revivers
            .iter()
            .any(|reviver_pos| reviver_pos.distance(downed_pos) <= REVIVE_RANGE);

        if !being_revived {
            if progress.is_some() {
                commands.entity(entity).remove::<ReviveProgress>();
            }
            continue;
        }

        let elapsed = progress.as_deref().map_or(0.0, |p| p.0) + dt;
        if elapsed >= REVIVE_DURATION_SECS {
            health.current = health.max * REVIVE_HEALTH_FRACTION;
            commands.entity(entity).remove::<Downed>();
            commands.entity(entity).remove::<ReviveProgress>();
        } else if let Some(mut existing) = progress {
            existing.0 = elapsed;
        } else {
            commands.entity(entity).insert(ReviveProgress(elapsed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn setup(dt: f32) -> World {
        let mut world = World::new();
        world.insert_resource(DeltaSeconds(dt));
        world
    }

    #[test]
    fn revive_system_accumulates_progress_while_ally_in_range() {
        let mut world = setup(1.0);
        let downed = world
            .spawn((
                Downed,
                Position { x: 0.0, y: 0.0 },
                Health {
                    current: 0.0,
                    max: 100.0,
                },
            ))
            .id();
        world.spawn((Reviving, Position { x: 10.0, y: 0.0 }));

        let _ = world.run_system_once(revive_system);

        assert_eq!(world.get::<ReviveProgress>(downed).unwrap().0, 1.0);
        assert!(world.get::<Downed>(downed).is_some());
        assert_eq!(world.get::<Health>(downed).unwrap().current, 0.0);
    }

    #[test]
    fn revive_system_resets_progress_when_no_ally_in_range() {
        let mut world = setup(1.0);
        let downed = world
            .spawn((
                Downed,
                Position { x: 0.0, y: 0.0 },
                Health::new(100.0),
                ReviveProgress(2.0),
            ))
            .id();
        world.spawn((Reviving, Position { x: 500.0, y: 0.0 }));

        let _ = world.run_system_once(revive_system);

        assert!(world.get::<ReviveProgress>(downed).is_none());
    }

    #[test]
    fn revive_system_completes_revive_after_full_duration() {
        let mut world = setup(0.5);
        let downed = world
            .spawn((
                Downed,
                Position { x: 0.0, y: 0.0 },
                Health::new(100.0),
                ReviveProgress(2.9),
            ))
            .id();
        world.spawn((Reviving, Position { x: 0.0, y: 0.0 }));

        let _ = world.run_system_once(revive_system);

        assert!(world.get::<Downed>(downed).is_none());
        assert!(world.get::<ReviveProgress>(downed).is_none());
        assert_eq!(world.get::<Health>(downed).unwrap().current, 50.0);
    }

    #[test]
    fn revive_system_ignores_a_downed_reviver() {
        let mut world = setup(1.0);
        let downed = world
            .spawn((Downed, Position { x: 0.0, y: 0.0 }, Health::new(100.0)))
            .id();
        // Pathological state: an entity that's both Downed and (somehow)
        // Reviving must never count toward reviving someone else.
        world.spawn((Reviving, Downed, Position { x: 0.0, y: 0.0 }));

        let _ = world.run_system_once(revive_system);

        assert!(world.get::<ReviveProgress>(downed).is_none());
    }
}
