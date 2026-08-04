use std::collections::HashMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::item::{pickup_loot, Inventory, ItemDrop, RuneInventory};
use crate::movement::Position;
use crate::status_effect::{ActiveEffects, EffectDefinition};

/// How close a player needs to be to a dropped item/rune to pick it up, when
/// `interact_or_pickup_system` falls back to a plain pickup (no `Interactable`
/// in range) — moved here from `server` now that the pickup resolution
/// itself lives in `game_core` (see this module's doc comment). Tuned
/// loosely against typical interaction ranges, not meant to require pixel
/// precision.
pub const PICKUP_RANGE: f32 = 50.0;

/// A world entity a player can trigger with the interact button (`E`) —
/// an NPC (blacksmith, sage) or a world object (runestone). Replicated as
/// just the content-template key + range, not the template's dialog/effect
/// data — the same "replicate a content-template key, not template data"
/// pattern as `EnemyKind` (see `DECISIONS.md`'s M8 planning entry): the
/// client resolves dialog text from its own locally-loaded
/// `content::InteractableTemplate`s, and the server resolves any effect
/// grant from its own `InteractableLibrary`. An embedded `EffectDefinition`
/// isn't an option here regardless — it doesn't derive
/// `Serialize`/`Deserialize` and is otherwise server-only.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interactable {
    pub template_key: String,
    pub range: f32,
}

/// Server-side-only resolution of an `Interactable`'s template — mirrors
/// `content::InteractableTemplate` but with `effect` already converted to
/// the engine's `EffectDefinition` (server-only, unlike the wire-safe
/// `Interactable` component above). `opens_panel` isn't read by
/// `interact_or_pickup_system` yet — that's M8 step 10's job (gating the
/// forging panel to a blacksmith-kind `Interactable`); the field exists now
/// so a content author can write it into a template today rather than
/// needing a schema migration later.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InteractableDefinition {
    pub effect: Option<EffectDefinition>,
    pub opens_panel: Option<String>,
}

/// Maps an `Interactable::template_key` to its `InteractableDefinition` —
/// a resource, not a component, since the definition is identical for
/// every instance of a given template (same shape as `SkillLibrary`/
/// `ItemLibrary`). Starts empty via `Default` rather than a fallible
/// content-file load: no real interactable content exists yet (that's M8
/// step 8's job, placed alongside actual map placement), so
/// `interact_or_pickup_system` needs to behave correctly against an empty
/// library today — a placed `Interactable` whose key isn't in the library
/// just grants no effect, not a startup panic.
#[derive(Resource, Debug, Default, Clone)]
pub struct InteractableLibrary(pub HashMap<String, InteractableDefinition>);

/// Sent when a player presses the interact/pickup button (`E`) — the
/// server-side translation of `protocol::PickupItemInput`, same
/// two-phase "server writes an event carrying who acted, `game_core`
/// resolves it" split as `AttackRequested`/`SkillCastRequested`.
#[derive(Message, Debug, Clone, Copy)]
pub struct InteractOrPickupRequested {
    pub actor: Entity,
}

/// Resolves `InteractOrPickupRequested`: the nearest `Interactable` within
/// its own `range` takes priority — applying its `InteractableLibrary`
/// effect (if any) to the actor unconditionally (there's no "attacker"
/// here, unlike `EffectDefinition::applies_to`'s target/attacker split for
/// a landed hit) — and if none is in range, falls back to the nearest
/// `ItemDrop` within `PICKUP_RANGE` (the exact behavior `server`'s pickup
/// handler had before this system existed). One button, one clear
/// priority rule (see `DECISIONS.md`'s M8 planning entry) — a player
/// standing near both an `Interactable` and a loose item drop always
/// interacts, never picks up, regardless of which is actually closer.
pub fn interact_or_pickup_system(
    mut events: MessageReader<InteractOrPickupRequested>,
    mut actors: Query<(
        &Position,
        &mut Inventory,
        &mut RuneInventory,
        &mut ActiveEffects,
    )>,
    interactables: Query<(&Position, &Interactable)>,
    drops: Query<(Entity, &Position, &ItemDrop)>,
    library: Res<InteractableLibrary>,
    mut commands: Commands,
) {
    for event in events.read() {
        let Ok((actor_pos, mut inventory, mut runes, mut effects)) = actors.get_mut(event.actor)
        else {
            continue;
        };

        let nearest_interactable = interactables
            .iter()
            .filter(|(pos, interactable)| actor_pos.distance(pos) <= interactable.range)
            .min_by(|(a, _), (b, _)| actor_pos.distance(a).total_cmp(&actor_pos.distance(b)));

        if let Some((_, interactable)) = nearest_interactable {
            if let Some(effect) = library
                .0
                .get(&interactable.template_key)
                .and_then(|definition| definition.effect.clone())
            {
                effects.apply(effect);
            }
            continue;
        }

        let nearest_drop = drops
            .iter()
            .filter(|(_, pos, _)| actor_pos.distance(pos) <= PICKUP_RANGE)
            .min_by(|(_, a, _), (_, b, _)| actor_pos.distance(a).total_cmp(&actor_pos.distance(b)));

        let Some((drop_entity, _, drop)) = nearest_drop else {
            continue;
        };
        pickup_loot(&mut inventory, &mut runes, drop.0.clone());
        commands.entity(drop_entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::DroppedLoot;
    use crate::status_effect::{EffectKind, StackMode, Stat};
    use bevy_ecs::system::RunSystemOnce;

    fn actor_bundle() -> (Position, Inventory, RuneInventory, ActiveEffects) {
        (
            Position { x: 0.0, y: 0.0 },
            Inventory::default(),
            RuneInventory::default(),
            ActiveEffects::default(),
        )
    }

    fn crit_buff() -> EffectDefinition {
        EffectDefinition {
            id: "test_buff".to_string(),
            kind: EffectKind::StatModifier {
                stat: Stat::CritChance,
            },
            duration: 5.0,
            magnitude: 0.1,
            stack_mode: StackMode::Independent,
            applies_to: crate::status_effect::EffectTarget::Attacker,
            chance: 1.0,
        }
    }

    fn item(template_key: &str) -> crate::item::Item {
        crate::item::Item {
            template_key: template_key.to_string(),
            sockets: vec![],
        }
    }

    #[test]
    fn applies_effect_from_nearest_interactable_in_range_and_skips_pickup() {
        let mut world = World::new();
        world.init_resource::<Messages<InteractOrPickupRequested>>();
        let mut library = InteractableLibrary::default();
        library.0.insert(
            "runestone".to_string(),
            InteractableDefinition {
                effect: Some(crit_buff()),
                opens_panel: None,
            },
        );
        world.insert_resource(library);

        let actor = world.spawn(actor_bundle()).id();
        world.spawn((
            Position { x: 10.0, y: 0.0 },
            Interactable {
                template_key: "runestone".to_string(),
                range: 50.0,
            },
        ));
        world.spawn((
            Position { x: 5.0, y: 0.0 },
            ItemDrop(DroppedLoot::Item(item("sword"))),
        ));

        world
            .resource_mut::<Messages<InteractOrPickupRequested>>()
            .write(InteractOrPickupRequested { actor });

        let _ = world.run_system_once(interact_or_pickup_system);

        assert_eq!(world.get::<ActiveEffects>(actor).unwrap().0.len(), 1);
        assert!(world.get::<Inventory>(actor).unwrap().0.is_empty());
    }

    #[test]
    fn falls_back_to_nearest_item_drop_when_no_interactable_in_range() {
        let mut world = World::new();
        world.init_resource::<Messages<InteractOrPickupRequested>>();
        world.init_resource::<InteractableLibrary>();

        let actor = world.spawn(actor_bundle()).id();
        let drop = world
            .spawn((
                Position { x: 5.0, y: 0.0 },
                ItemDrop(DroppedLoot::Item(item("sword"))),
            ))
            .id();

        world
            .resource_mut::<Messages<InteractOrPickupRequested>>()
            .write(InteractOrPickupRequested { actor });

        let _ = world.run_system_once(interact_or_pickup_system);

        assert_eq!(world.get::<Inventory>(actor).unwrap().0.len(), 1);
        assert!(world.get_entity(drop).is_err());
    }

    #[test]
    fn ignores_an_interactable_out_of_its_own_range_and_falls_back_to_pickup() {
        let mut world = World::new();
        world.init_resource::<Messages<InteractOrPickupRequested>>();
        let mut library = InteractableLibrary::default();
        library.0.insert(
            "runestone".to_string(),
            InteractableDefinition {
                effect: Some(crit_buff()),
                opens_panel: None,
            },
        );
        world.insert_resource(library);

        let actor = world.spawn(actor_bundle()).id();
        world.spawn((
            Position { x: 500.0, y: 0.0 },
            Interactable {
                template_key: "runestone".to_string(),
                range: 10.0,
            },
        ));
        world.spawn((
            Position { x: 5.0, y: 0.0 },
            ItemDrop(DroppedLoot::Item(item("sword"))),
        ));

        world
            .resource_mut::<Messages<InteractOrPickupRequested>>()
            .write(InteractOrPickupRequested { actor });

        let _ = world.run_system_once(interact_or_pickup_system);

        assert!(world.get::<ActiveEffects>(actor).unwrap().0.is_empty());
        assert_eq!(world.get::<Inventory>(actor).unwrap().0.len(), 1);
    }

    #[test]
    fn does_nothing_when_neither_an_interactable_nor_a_drop_is_in_range() {
        let mut world = World::new();
        world.init_resource::<Messages<InteractOrPickupRequested>>();
        world.init_resource::<InteractableLibrary>();

        let actor = world.spawn(actor_bundle()).id();
        world.spawn((
            Position { x: 5000.0, y: 0.0 },
            ItemDrop(DroppedLoot::Item(item("sword"))),
        ));

        world
            .resource_mut::<Messages<InteractOrPickupRequested>>()
            .write(InteractOrPickupRequested { actor });

        let _ = world.run_system_once(interact_or_pickup_system);

        assert!(world.get::<Inventory>(actor).unwrap().0.is_empty());
    }

    #[test]
    fn prefers_the_nearest_interactable_when_multiple_are_in_range() {
        let mut world = World::new();
        world.init_resource::<Messages<InteractOrPickupRequested>>();
        let mut library = InteractableLibrary::default();
        library.0.insert(
            "near".to_string(),
            InteractableDefinition {
                effect: Some(crit_buff()),
                opens_panel: None,
            },
        );
        library.0.insert(
            "far".to_string(),
            InteractableDefinition {
                effect: None,
                opens_panel: None,
            },
        );
        world.insert_resource(library);

        let actor = world.spawn(actor_bundle()).id();
        world.spawn((
            Position { x: 40.0, y: 0.0 },
            Interactable {
                template_key: "far".to_string(),
                range: 100.0,
            },
        ));
        world.spawn((
            Position { x: 10.0, y: 0.0 },
            Interactable {
                template_key: "near".to_string(),
                range: 100.0,
            },
        ));

        world
            .resource_mut::<Messages<InteractOrPickupRequested>>()
            .write(InteractOrPickupRequested { actor });

        let _ = world.run_system_once(interact_or_pickup_system);

        // Only "near"'s effect (which has one) should have applied.
        assert_eq!(world.get::<ActiveEffects>(actor).unwrap().0.len(), 1);
    }

    #[test]
    fn an_interactable_with_no_matching_library_entry_still_takes_priority_over_pickup() {
        let mut world = World::new();
        world.init_resource::<Messages<InteractOrPickupRequested>>();
        world.init_resource::<InteractableLibrary>(); // empty: unknown key

        let actor = world.spawn(actor_bundle()).id();
        world.spawn((
            Position { x: 10.0, y: 0.0 },
            Interactable {
                template_key: "mystery".to_string(),
                range: 50.0,
            },
        ));
        world.spawn((
            Position { x: 5.0, y: 0.0 },
            ItemDrop(DroppedLoot::Item(item("sword"))),
        ));

        world
            .resource_mut::<Messages<InteractOrPickupRequested>>()
            .write(InteractOrPickupRequested { actor });

        let _ = world.run_system_once(interact_or_pickup_system);

        assert!(world.get::<ActiveEffects>(actor).unwrap().0.is_empty());
        assert!(world.get::<Inventory>(actor).unwrap().0.is_empty());
    }
}
