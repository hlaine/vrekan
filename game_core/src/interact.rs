use std::collections::HashMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::economy::Currency;
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

/// The `opens_panels` value that gates forging to blacksmith-kind
/// `Interactable`s — a shared identifier between the content file
/// (`blacksmith.ron`'s `opens_panels: ["forging", ...]`), the server's
/// proximity gate (`nearest_interactable_with_panel`), and the client's
/// panel trigger, so the three can't drift apart into mismatched strings.
pub const FORGING_PANEL_ID: &str = "forging";

/// The `opens_panels` value that gates buy/sell to vendor-kind
/// `Interactable`s — same shared-identifier reasoning as `FORGING_PANEL_ID`.
pub const VENDOR_PANEL_ID: &str = "vendor";

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
/// `Interactable` component above). `opens_panels` gates forging/vendor
/// proximity (see `nearest_interactable_with_panel`, `FORGING_PANEL_ID`/
/// `VENDOR_PANEL_ID`) — not read by `interact_or_pickup_system` itself,
/// since opening a panel is a purely client-side reaction to the same
/// button press, not a server-resolved outcome. A `Vec` rather than a
/// single `Option<String>`, since one NPC can offer more than one
/// capability (e.g. a blacksmith is both `"forging"` and `"vendor"`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InteractableDefinition {
    pub effect: Option<EffectDefinition>,
    pub opens_panels: Vec<String>,
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

/// The nearest `Interactable` within its own `range`, if any — shared by
/// `interact_or_pickup_system` (server-authoritative effect resolution) and
/// `client`'s dialog trigger (instant, no-round-trip dialog display per
/// `DECISIONS.md`'s M8 planning entry). One priority rule implemented once,
/// so both sides of the "dialog is read locally, effect is a round-trip"
/// split can never disagree on which `Interactable` is "the" nearest one.
pub fn nearest_interactable_in_range<'a>(
    actor_pos: &Position,
    interactables: impl Iterator<Item = (&'a Position, &'a Interactable)>,
) -> Option<&'a Interactable> {
    interactables
        .filter(|(pos, interactable)| actor_pos.distance(pos) <= interactable.range)
        .min_by(|(a, _), (b, _)| actor_pos.distance(a).total_cmp(&actor_pos.distance(b)))
        .map(|(_, interactable)| interactable)
}

/// The nearest in-range `Interactable` whose `InteractableLibrary`
/// definition declares `panel` in `opens_panels` — the server-side
/// resolution `apply_socket_rune_input`/`apply_unsocket_rune_input` use to
/// require being near a blacksmith-kind `Interactable` (see
/// `FORGING_PANEL_ID`), a real behavior change from M7's free-anywhere
/// hotkey socketing (see `DECISIONS.md`'s M8 planning entry), and that
/// `apply_buy_item_input`/`apply_sell_item_input` use to both gate *and*
/// identify *which* vendor's listing to resolve against (via the returned
/// `Interactable::template_key`) — the server re-derives this itself
/// rather than trusting a client-claimed vendor id. Unlike
/// `nearest_interactable_in_range`, this filters to matching
/// `Interactable`s first — a player standing near both a runestone and a
/// blacksmith should still be able to forge, even though the runestone
/// might be closer overall.
pub fn nearest_interactable_with_panel<'a>(
    actor_pos: &Position,
    panel: &str,
    interactables: impl Iterator<Item = (&'a Position, &'a Interactable)>,
    library: &InteractableLibrary,
) -> Option<&'a Interactable> {
    interactables
        .filter(|(pos, interactable)| {
            actor_pos.distance(pos) <= interactable.range
                && library
                    .0
                    .get(&interactable.template_key)
                    .is_some_and(|definition| definition.opens_panels.iter().any(|p| p == panel))
        })
        .min_by(|(a, _), (b, _)| actor_pos.distance(a).total_cmp(&actor_pos.distance(b)))
        .map(|(_, interactable)| interactable)
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
        &mut Currency,
        &mut ActiveEffects,
    )>,
    interactables: Query<(&Position, &Interactable)>,
    drops: Query<(Entity, &Position, &ItemDrop)>,
    library: Res<InteractableLibrary>,
    mut commands: Commands,
) {
    for event in events.read() {
        let Ok((actor_pos, mut inventory, mut runes, mut currency, mut effects)) =
            actors.get_mut(event.actor)
        else {
            continue;
        };

        let nearest_interactable = nearest_interactable_in_range(actor_pos, interactables.iter());

        if let Some(interactable) = nearest_interactable {
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
        pickup_loot(&mut inventory, &mut runes, &mut currency, drop.0.clone());
        commands.entity(drop_entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::DroppedLoot;
    use crate::status_effect::{EffectKind, StackMode, Stat};
    use bevy_ecs::system::RunSystemOnce;

    fn actor_bundle() -> (Position, Inventory, RuneInventory, Currency, ActiveEffects) {
        (
            Position { x: 0.0, y: 0.0 },
            Inventory::default(),
            RuneInventory::default(),
            Currency::default(),
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
                opens_panels: vec![],
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
    fn picks_up_a_currency_drop_and_credits_the_actors_balance() {
        let mut world = World::new();
        world.init_resource::<Messages<InteractOrPickupRequested>>();
        world.init_resource::<InteractableLibrary>();

        let actor = world.spawn(actor_bundle()).id();
        let drop = world
            .spawn((
                Position { x: 5.0, y: 0.0 },
                ItemDrop(DroppedLoot::Currency(15)),
            ))
            .id();

        world
            .resource_mut::<Messages<InteractOrPickupRequested>>()
            .write(InteractOrPickupRequested { actor });

        let _ = world.run_system_once(interact_or_pickup_system);

        assert_eq!(world.get::<Currency>(actor).unwrap().0, 15);
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
                opens_panels: vec![],
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
                opens_panels: vec![],
            },
        );
        library.0.insert(
            "far".to_string(),
            InteractableDefinition {
                effect: None,
                opens_panels: vec![],
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

    fn forging_library() -> InteractableLibrary {
        let mut library = InteractableLibrary::default();
        library.0.insert(
            "blacksmith".to_string(),
            InteractableDefinition {
                effect: None,
                opens_panels: vec![FORGING_PANEL_ID.to_string()],
            },
        );
        library.0.insert(
            "runestone".to_string(),
            InteractableDefinition {
                effect: None,
                opens_panels: vec![],
            },
        );
        library
    }

    #[test]
    fn nearest_interactable_with_panel_finds_a_matching_interactable_in_range() {
        let actor_pos = Position { x: 0.0, y: 0.0 };
        let blacksmith_pos = Position { x: 10.0, y: 0.0 };
        let blacksmith = Interactable {
            template_key: "blacksmith".to_string(),
            range: 50.0,
        };

        assert_eq!(
            nearest_interactable_with_panel(
                &actor_pos,
                FORGING_PANEL_ID,
                std::iter::once((&blacksmith_pos, &blacksmith)),
                &forging_library(),
            ),
            Some(&blacksmith)
        );
    }

    #[test]
    fn nearest_interactable_with_panel_ignores_an_in_range_interactable_with_a_different_panel() {
        let actor_pos = Position { x: 0.0, y: 0.0 };
        let runestone_pos = Position { x: 10.0, y: 0.0 };
        let runestone = Interactable {
            template_key: "runestone".to_string(),
            range: 50.0,
        };

        assert_eq!(
            nearest_interactable_with_panel(
                &actor_pos,
                FORGING_PANEL_ID,
                std::iter::once((&runestone_pos, &runestone)),
                &forging_library(),
            ),
            None
        );
    }

    #[test]
    fn nearest_interactable_with_panel_ignores_a_matching_interactable_out_of_range() {
        let actor_pos = Position { x: 0.0, y: 0.0 };
        let blacksmith_pos = Position { x: 500.0, y: 0.0 };
        let blacksmith = Interactable {
            template_key: "blacksmith".to_string(),
            range: 50.0,
        };

        assert_eq!(
            nearest_interactable_with_panel(
                &actor_pos,
                FORGING_PANEL_ID,
                std::iter::once((&blacksmith_pos, &blacksmith)),
                &forging_library(),
            ),
            None
        );
    }

    #[test]
    fn nearest_interactable_with_panel_finds_it_even_when_a_farther_non_matching_interactable_is_also_in_range(
    ) {
        let actor_pos = Position { x: 0.0, y: 0.0 };
        let runestone_pos = Position { x: 5.0, y: 0.0 };
        let runestone = Interactable {
            template_key: "runestone".to_string(),
            range: 50.0,
        };
        let blacksmith_pos = Position { x: 40.0, y: 0.0 };
        let blacksmith = Interactable {
            template_key: "blacksmith".to_string(),
            range: 50.0,
        };

        assert_eq!(
            nearest_interactable_with_panel(
                &actor_pos,
                FORGING_PANEL_ID,
                [(&runestone_pos, &runestone), (&blacksmith_pos, &blacksmith)].into_iter(),
                &forging_library(),
            ),
            Some(&blacksmith)
        );
    }
}
