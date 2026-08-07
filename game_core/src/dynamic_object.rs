use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::movement::Position;

/// A `RigidBody::Dynamic` object with no `Health`/AI, pushed exactly the way
/// a player already pushes an enemy (mass-scaled momentum transfer via
/// physics collision, no new physics concept) — see MECHANICS.md's Dynamic
/// objects section. The marker itself carries no data; it exists so
/// `server` can widen its physics-position sync filter to include these
/// entities, and so `client` can give a newly-replicated one an appearance.
#[derive(Component, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct PushableObject;

/// Axis-aligned world-space bounds an `UnlockCondition::ObjectInZone`
/// checks a pushable object's `Position` against — resolved once at spawn
/// time from a Tiled rectangle object's world-space extent, not a live
/// reference to the Tiled object itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zone {
    pub min_x: f32,
    pub max_x: f32,
    pub min_y: f32,
    pub max_y: f32,
}

impl Zone {
    pub fn contains(&self, position: &Position) -> bool {
        position.x >= self.min_x
            && position.x <= self.max_x
            && position.y >= self.min_y
            && position.y <= self.max_y
    }
}

/// One condition an `Unlockable` gate can require — see that component's
/// doc comment for the AND-semantics across the full condition list.
/// `HasKeyItem(String)` (M8.13) will be a second variant here, added once
/// that system lands — shaped so it slots in without restructuring this
/// enum or `unlock_conditions_met`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnlockCondition {
    ObjectInZone { object: Entity, zone: Zone },
}

/// A generic gate: passable once **every** condition in `conditions` holds
/// simultaneously — AND semantics, not OR, stated explicitly here since
/// M8.13 builds a second condition variant against this same type and the
/// semantics matter for that milestone's multi-key gate idea (e.g. two
/// different party members each holding their own key). Starts with one
/// variant, `ObjectInZone` (a pushed object's position overlapping a
/// defined target area) — see MECHANICS.md's Dynamic objects section.
///
/// Server-only, not replicated: the client doesn't need to know *why* a
/// gate is open, only that it currently is — see `GateOpen`, the small
/// replicated presence marker `update_unlockables` toggles instead.
#[derive(Component, Debug, Clone)]
pub struct Unlockable {
    pub conditions: Vec<UnlockCondition>,
}

/// Always-present replicated marker for a gate entity — spawned alongside
/// `Unlockable`, which itself can't be replicated (see that component's
/// doc comment). Unlike `GateOpen` (present only while open), this exists
/// so the client can identify a gate at all, the same "spawn-time marker
/// the client reacts to `Added<>` on" role `Destructible`/`PushableObject`
/// play for their own kinds.
#[derive(Component, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Gate;

/// Whether every condition in `conditions` currently holds — `Query` access
/// is needed (not a fully pure function) since `ObjectInZone` has to look
/// up its referenced object's *current* `Position`, computed fresh every
/// call rather than cached, same "effective values always computed fresh"
/// principle the rest of the codebase follows. An object that's vanished
/// (despawned, or otherwise missing `Position`) reads as "not in the zone"
/// rather than panicking or short-circuiting the whole check.
pub fn unlock_conditions_met(conditions: &[UnlockCondition], positions: &Query<&Position>) -> bool {
    conditions.iter().all(|condition| match condition {
        UnlockCondition::ObjectInZone { object, zone } => positions
            .get(*object)
            .is_ok_and(|position| zone.contains(position)),
    })
}

/// Short-lived-in-name-only marker (unlike `RecentCrit`, this has no
/// countdown — it's present for as long as `Unlockable`'s conditions hold,
/// however long that is) — replicated like `Downed`/`Stunned` so the client
/// can render a gate as open/closed without needing to evaluate any
/// conditions itself. `server` bridges this to the actual physics
/// collision toggle (avian2d's `ColliderDisabled`) in its own sync system —
/// `game_core` has no physics-engine dependency (see `CLAUDE.md`'s crate
/// boundaries), so that bridging can't live here.
#[derive(Component, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct GateOpen;

/// Recomputes every `Unlockable`'s satisfied-or-not state each tick
/// (conditions depend on a pushed object's live `Position`, so this can't
/// be event-driven the way a button-press-triggered system would be) and
/// toggles `GateOpen`'s presence to match. Only touches the entity when the
/// state actually changes, so this is a cheap no-op most ticks for a gate
/// that's settled open or closed.
pub fn update_unlockables(
    mut commands: Commands,
    gates: Query<(Entity, &Unlockable, Has<GateOpen>)>,
    positions: Query<&Position>,
) {
    for (entity, unlockable, currently_open) in &gates {
        let should_be_open = unlock_conditions_met(&unlockable.conditions, &positions);
        if should_be_open && !currently_open {
            commands.entity(entity).insert(GateOpen);
        } else if !should_be_open && currently_open {
            commands.entity(entity).remove::<GateOpen>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    fn zone() -> Zone {
        Zone {
            min_x: 100.0,
            max_x: 140.0,
            min_y: 200.0,
            max_y: 240.0,
        }
    }

    #[test]
    fn zone_contains_a_position_inside_its_bounds() {
        assert!(zone().contains(&Position { x: 120.0, y: 220.0 }));
    }

    #[test]
    fn zone_contains_a_position_exactly_on_its_boundary() {
        assert!(zone().contains(&Position { x: 100.0, y: 200.0 }));
        assert!(zone().contains(&Position { x: 140.0, y: 240.0 }));
    }

    #[test]
    fn zone_excludes_a_position_outside_its_bounds() {
        assert!(!zone().contains(&Position { x: 99.0, y: 220.0 }));
        assert!(!zone().contains(&Position { x: 120.0, y: 241.0 }));
    }

    #[test]
    fn update_unlockables_opens_a_gate_once_its_condition_is_met() {
        let mut world = World::new();
        let object = world.spawn(Position { x: 120.0, y: 220.0 }).id();
        let gate = world
            .spawn(Unlockable {
                conditions: vec![UnlockCondition::ObjectInZone {
                    object,
                    zone: zone(),
                }],
            })
            .id();

        let _ = world.run_system_once(update_unlockables);

        assert!(world.get::<GateOpen>(gate).is_some());
    }

    #[test]
    fn update_unlockables_leaves_a_gate_closed_when_the_referenced_object_is_gone() {
        let mut world = World::new();
        let object = world.spawn(Position { x: 120.0, y: 220.0 }).id();
        world.despawn(object);
        let gate = world
            .spawn(Unlockable {
                conditions: vec![UnlockCondition::ObjectInZone {
                    object,
                    zone: zone(),
                }],
            })
            .id();

        let _ = world.run_system_once(update_unlockables);

        assert!(world.get::<GateOpen>(gate).is_none());
    }

    #[test]
    fn update_unlockables_leaves_a_gate_closed_when_no_condition_is_met() {
        let mut world = World::new();
        let object = world.spawn(Position { x: 0.0, y: 0.0 }).id();
        let gate = world
            .spawn(Unlockable {
                conditions: vec![UnlockCondition::ObjectInZone {
                    object,
                    zone: zone(),
                }],
            })
            .id();

        let _ = world.run_system_once(update_unlockables);

        assert!(world.get::<GateOpen>(gate).is_none());
    }

    #[test]
    fn update_unlockables_closes_a_previously_open_gate_once_a_condition_stops_holding() {
        let mut world = World::new();
        let object = world.spawn(Position { x: 120.0, y: 220.0 }).id();
        let gate = world
            .spawn((
                Unlockable {
                    conditions: vec![UnlockCondition::ObjectInZone {
                        object,
                        zone: zone(),
                    }],
                },
                GateOpen,
            ))
            .id();
        world.get_mut::<Position>(object).unwrap().x = 0.0;

        let _ = world.run_system_once(update_unlockables);

        assert!(world.get::<GateOpen>(gate).is_none());
    }

    #[test]
    fn update_unlockables_requires_all_conditions_before_opening() {
        let mut world = World::new();
        let inside = world.spawn(Position { x: 120.0, y: 220.0 }).id();
        let outside = world.spawn(Position { x: 0.0, y: 0.0 }).id();
        let gate = world
            .spawn(Unlockable {
                conditions: vec![
                    UnlockCondition::ObjectInZone {
                        object: inside,
                        zone: zone(),
                    },
                    UnlockCondition::ObjectInZone {
                        object: outside,
                        zone: zone(),
                    },
                ],
            })
            .id();

        let _ = world.run_system_once(update_unlockables);

        assert!(world.get::<GateOpen>(gate).is_none());
    }
}
