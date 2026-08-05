use std::collections::HashMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::item::{Inventory, Item, ItemLibrary, RuneLibrary};

/// A player's individual currency balance — not shared/pooled across the
/// party, consistent with no player-to-player trading (see `DESIGN.md`,
/// `MECHANICS.md`'s Economy section). Spent via `socket_rune` (forging
/// cost) and `buy_item`/`sell_item` (vendor economy) — both M7 part 2.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Currency(pub u32);

/// One buyable entry in a vendor's stock: an item template and the price
/// to buy it — distinct from `ItemDefinition::sell_value` (see its doc
/// comment), since what a vendor sells and what it'll buy back are
/// independent; any vendor buys any item at its universal `sell_value`,
/// but only a specific vendor's own listing says what it sells and for
/// how much.
#[derive(Debug, Clone, PartialEq)]
pub struct VendorListing {
    pub item_template_key: String,
    pub price: u32,
}

/// Maps an `Interactable::template_key` (of a vendor-kind `Interactable`,
/// see `VENDOR_PANEL_ID`) to its buyable stock — same "one key, multiple
/// resources depending on capability" shape as `InteractableLibrary`.
#[derive(Resource, Debug, Default, Clone)]
pub struct VendorLibrary(pub HashMap<String, Vec<VendorListing>>);

/// Buys `listing` into `inventory`, charging its `price` from `currency`.
/// Rejects (returns `false`, no state changed) insufficient currency or an
/// unknown item template — the latter would only happen from a malformed
/// `VendorTemplate` (a content bug, not untrusted input), but there's
/// nothing sensible to construct either way.
pub fn buy_item(
    inventory: &mut Inventory,
    currency: &mut Currency,
    items: &ItemLibrary,
    listing: &VendorListing,
) -> bool {
    if currency.0 < listing.price {
        return false;
    }
    let Some(definition) = items.0.get(&listing.item_template_key) else {
        return false;
    };
    currency.0 -= listing.price;
    inventory.0.push(Item {
        template_key: listing.item_template_key.clone(),
        sockets: vec![None; definition.socket_count as usize],
    });
    true
}

/// The price a vendor pays for an item: its template's base `sell_value`
/// plus each socketed rune's own `socket_cost` (reused as the rune's
/// implicit worth — runes can't be bought/sold on their own, see
/// `DECISIONS.md`'s M7 part 2 planning entry). Deliberately break-even on
/// the rune side: socketing a rune and immediately reselling the item
/// nets back exactly what the socketing cost, never a profit.
///
/// Takes a lookup closure rather than `&RuneLibrary` directly so the
/// client can reuse this exact formula against its own locally-loaded
/// `content::RuneTemplate`s for a sell-price preview, without needing to
/// build a full `RuneLibrary` just to call it — same "one formula, shared
/// by both callers, so they can't disagree" principle as
/// `nearest_interactable_in_range`.
pub fn socketed_item_sell_value(
    base_sell_value: u32,
    sockets: &[Option<String>],
    rune_socket_cost: impl Fn(&str) -> Option<u32>,
) -> u32 {
    base_sell_value
        + sockets
            .iter()
            .flatten()
            .filter_map(|rune_id| rune_socket_cost(rune_id.as_str()))
            .sum::<u32>()
}

/// Sells `inventory[inventory_index]`, removing it and crediting
/// `currency` with `socketed_item_sell_value`. Rejects (returns `false`,
/// no state changed) an out-of-range index or an unknown item template —
/// same "rejected action, not a panic" treatment `equip_item` already
/// gives an unknown template.
pub fn sell_item(
    inventory: &mut Inventory,
    currency: &mut Currency,
    items: &ItemLibrary,
    runes: &RuneLibrary,
    inventory_index: usize,
) -> bool {
    let Some(item) = inventory.0.get(inventory_index) else {
        return false;
    };
    let Some(definition) = items.0.get(&item.template_key) else {
        return false;
    };
    let value = socketed_item_sell_value(definition.sell_value, &item.sockets, |rune_id| {
        runes
            .0
            .get(rune_id)
            .map(|definition| definition.socket_cost)
    });
    inventory.0.remove(inventory_index);
    currency.0 += value;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{EquipSlot, ItemDefinition, RuneDefinition};
    use crate::status_effect::Stat;

    fn sword_library() -> ItemLibrary {
        let mut items = ItemLibrary::default();
        items.0.insert(
            "sword".to_string(),
            ItemDefinition {
                slot: EquipSlot::Weapon,
                socket_count: 2,
                sell_value: 5,
            },
        );
        items
    }

    fn rune_library() -> RuneLibrary {
        let mut runes = RuneLibrary::default();
        runes.0.insert(
            "crit_shard".to_string(),
            RuneDefinition {
                stat: Stat::CritChance,
                magnitude: 0.05,
                socket_cost: 20,
            },
        );
        runes
    }

    #[test]
    fn buy_item_deducts_price_and_adds_a_correctly_socketed_item() {
        let mut inventory = Inventory::default();
        let mut currency = Currency(100);
        let listing = VendorListing {
            item_template_key: "sword".to_string(),
            price: 30,
        };

        assert!(buy_item(
            &mut inventory,
            &mut currency,
            &sword_library(),
            &listing
        ));

        assert_eq!(currency.0, 70);
        assert_eq!(inventory.0.len(), 1);
        assert_eq!(inventory.0[0].template_key, "sword");
        assert_eq!(inventory.0[0].sockets, vec![None, None]);
    }

    #[test]
    fn buy_item_rejects_insufficient_currency_and_charges_nothing() {
        let mut inventory = Inventory::default();
        let mut currency = Currency(10);
        let listing = VendorListing {
            item_template_key: "sword".to_string(),
            price: 30,
        };

        assert!(!buy_item(
            &mut inventory,
            &mut currency,
            &sword_library(),
            &listing
        ));

        assert_eq!(currency.0, 10);
        assert!(inventory.0.is_empty());
    }

    #[test]
    fn buy_item_rejects_an_unknown_item_template() {
        let mut inventory = Inventory::default();
        let mut currency = Currency(100);
        let listing = VendorListing {
            item_template_key: "mystery_sword".to_string(),
            price: 30,
        };

        assert!(!buy_item(
            &mut inventory,
            &mut currency,
            &ItemLibrary::default(),
            &listing
        ));

        assert_eq!(currency.0, 100);
        assert!(inventory.0.is_empty());
    }

    #[test]
    fn socketed_item_sell_value_sums_base_value_and_known_rune_costs() {
        let sockets = vec![Some("crit_shard".to_string()), None];

        let value = socketed_item_sell_value(5, &sockets, |rune_id| {
            rune_library().0.get(rune_id).map(|def| def.socket_cost)
        });

        assert_eq!(value, 25);
    }

    #[test]
    fn sell_item_removes_the_item_and_credits_base_plus_socketed_rune_value() {
        let mut inventory = Inventory(vec![Item {
            template_key: "sword".to_string(),
            sockets: vec![Some("crit_shard".to_string()), None],
        }]);
        let mut currency = Currency(0);

        assert!(sell_item(
            &mut inventory,
            &mut currency,
            &sword_library(),
            &rune_library(),
            0
        ));

        assert!(inventory.0.is_empty());
        assert_eq!(currency.0, 25);
    }

    #[test]
    fn sell_item_rejects_an_out_of_range_index() {
        let mut inventory = Inventory::default();
        let mut currency = Currency(0);

        assert!(!sell_item(
            &mut inventory,
            &mut currency,
            &sword_library(),
            &rune_library(),
            0
        ));

        assert_eq!(currency.0, 0);
    }
}
