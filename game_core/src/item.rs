use std::collections::HashMap;

use bevy_ecs::prelude::*;
use rand::{Rng, RngExt};
use serde::{Deserialize, Serialize};

use crate::status_effect::Stat;

/// Which equipment slot an item template goes into — matches the two
/// slots M8's HUD/menus mention rendering (armor/helmet) plus the weapon
/// itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EquipSlot {
    Weapon,
    Armor,
    Helmet,
}

/// One concrete item instance — unlike e.g. `EnemyKind`, an item can't be
/// represented by just its template key, since two drops of the same
/// template have independently rolled (here: independently empty, then
/// independently socketed) sockets. `sockets.len()` is fixed at creation
/// time from the template's socket count and never resized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub template_key: String,
    pub sockets: Vec<Option<String>>,
}

/// Data-driven shape for an item template — see `EnemyTemplate`'s doc
/// comment for the same content-not-engine-code principle. Loaded once
/// into a `Res<ItemLibrary>` and looked up by template key (e.g. to
/// determine which `EquipSlot` an inventory item belongs in when equipped,
/// or how many sockets a freshly-rolled drop should have).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDefinition {
    pub slot: EquipSlot,
    pub socket_count: u32,
}

#[derive(Resource, Debug, Default, Clone)]
pub struct ItemLibrary(pub HashMap<String, ItemDefinition>);

/// Data-driven shape for a rune template — a permanent stat bonus while
/// socketed, not a timed `StatModifier` effect (see `Equipment::stat_bonus`
/// for how it's summed at point-of-use, same "never bake a buff into the
/// base stat" principle `ActiveEffects::stat_bonus` already established).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuneDefinition {
    pub stat: Stat,
    pub magnitude: f32,
}

#[derive(Resource, Debug, Default, Clone)]
pub struct RuneLibrary(pub HashMap<String, RuneDefinition>);

/// Items not currently equipped. Replicated so a future inventory UI (M8)
/// has something to read; the actual UI doesn't exist yet, same
/// "data ready before the UI" shape as M5's `Stats`.
#[derive(Component, Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inventory(pub Vec<Item>);

/// Currently-equipped items, one per slot. Replicated for the same reason
/// as `Inventory`.
#[derive(Component, Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Equipment {
    pub weapon: Option<Item>,
    pub armor: Option<Item>,
    pub helmet: Option<Item>,
}

impl Equipment {
    fn get_mut(&mut self, slot: EquipSlot) -> Option<&mut Item> {
        match slot {
            EquipSlot::Weapon => self.weapon.as_mut(),
            EquipSlot::Armor => self.armor.as_mut(),
            EquipSlot::Helmet => self.helmet.as_mut(),
        }
    }

    /// Swaps in `item` at `slot`, returning whatever was previously there
    /// (if anything) so the caller can put it back in the inventory.
    fn set(&mut self, slot: EquipSlot, item: Option<Item>) -> Option<Item> {
        let target = match slot {
            EquipSlot::Weapon => &mut self.weapon,
            EquipSlot::Armor => &mut self.armor,
            EquipSlot::Helmet => &mut self.helmet,
        };
        std::mem::replace(target, item)
    }

    /// Sum of `stat` bonuses from every socketed rune across all three
    /// slots — computed fresh at point of use (see `combat::attack_system`'s
    /// effective-stats computation), not merged into the item permanently,
    /// so unsocketing a rune can't leave a stale bonus behind.
    pub fn stat_bonus(&self, stat: Stat, runes: &RuneLibrary) -> f32 {
        [&self.weapon, &self.armor, &self.helmet]
            .into_iter()
            .flatten()
            .flat_map(|item| item.sockets.iter())
            .filter_map(|socket| socket.as_deref())
            .filter_map(|rune_id| runes.0.get(rune_id))
            .filter(|rune| rune.stat == stat)
            .map(|rune| rune.magnitude)
            .sum()
    }
}

/// Rune counts by template id — runes are fungible/stackable, unlike
/// uniquely-rolled equipment, so a simple count suffices rather than
/// tracking individual rune entities/instances.
#[derive(Component, Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuneInventory(pub HashMap<String, u32>);

/// What a loot roll produced — an item (with sockets sized from its
/// template) or a stack-eligible rune, spawned into the world as an
/// `ItemDrop` and merged into the picker's `Inventory`/`RuneInventory` on
/// pickup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DroppedLoot {
    Item(Item),
    Rune(String),
}

/// A world-visible dropped item/rune, picked up via `PickupItemInput` —
/// see `server`'s pickup-resolution system.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemDrop(pub DroppedLoot);

/// One weighted entry in a `LootTable` — either an item or a rune template
/// key. Resolved against `ItemLibrary` at roll time (see `roll_loot`), not
/// pre-resolved, since a `LootTable` is authored once per enemy template
/// but a rolled `Item`'s sockets need to be sized fresh each drop.
#[derive(Debug, Clone, PartialEq)]
pub enum LootKind {
    Item(String),
    Rune(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LootEntry {
    pub kind: LootKind,
    pub weight: f32,
}

/// Attached to an enemy instance at spawn (see `content::spawn_enemy`),
/// mirroring how `MeleeAttack`/`XpReward` are attached from
/// `EnemyTemplate` — rolled once at the moment of death
/// (`combat::death_system`), not a recurring system.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct LootTable {
    pub drop_chance: f32,
    pub entries: Vec<LootEntry>,
}

/// Rolls `table` against `rng`: first whether anything drops at all
/// (`drop_chance`), then a weighted pick among `entries`. `None` covers
/// both "nothing dropped" and the degenerate "table has no entries, or
/// they sum to zero weight" case — there's nothing sensible to return
/// either way.
pub fn roll_loot(
    table: &LootTable,
    items: &ItemLibrary,
    rng: &mut impl Rng,
) -> Option<DroppedLoot> {
    if table.entries.is_empty() || !rng.random_bool(table.drop_chance as f64) {
        return None;
    }
    let total_weight: f32 = table.entries.iter().map(|entry| entry.weight).sum();
    if total_weight <= 0.0 {
        return None;
    }

    let mut roll = rng.random::<f32>() * total_weight;
    for entry in &table.entries {
        if roll < entry.weight {
            return Some(match &entry.kind {
                LootKind::Item(template_key) => {
                    let socket_count = items
                        .0
                        .get(template_key)
                        .map(|definition| definition.socket_count)
                        .unwrap_or(0);
                    DroppedLoot::Item(Item {
                        template_key: template_key.clone(),
                        sockets: vec![None; socket_count as usize],
                    })
                }
                LootKind::Rune(rune_key) => DroppedLoot::Rune(rune_key.clone()),
            });
        }
        roll -= entry.weight;
    }
    None
}

/// Merges a dropped loot into the picker's inventory/rune counts — an
/// item goes onto the (unbounded, for now) inventory list, a rune
/// increments its stack count.
pub fn pickup_loot(inventory: &mut Inventory, runes: &mut RuneInventory, loot: DroppedLoot) {
    match loot {
        DroppedLoot::Item(item) => inventory.0.push(item),
        DroppedLoot::Rune(rune_id) => *runes.0.entry(rune_id).or_insert(0) += 1,
    }
}

/// Moves `inventory[inventory_index]` into its template's `EquipSlot`,
/// displacing whatever was equipped there back into the inventory.
/// Returns `false` (a no-op) if the index is out of range or the item's
/// template is unknown to `items` — both are treated as a rejected action,
/// not a panic, since they're driven by untrusted client input.
pub fn equip_item(
    inventory: &mut Inventory,
    equipment: &mut Equipment,
    items: &ItemLibrary,
    inventory_index: usize,
) -> bool {
    let Some(item) = inventory.0.get(inventory_index) else {
        return false;
    };
    let Some(definition) = items.0.get(&item.template_key) else {
        return false;
    };
    let slot = definition.slot;
    let incoming = inventory.0.remove(inventory_index);
    if let Some(displaced) = equipment.set(slot, Some(incoming)) {
        inventory.0.push(displaced);
    }
    true
}

/// Moves whatever's equipped at `slot` back into the inventory. Returns
/// `false` if nothing was equipped there.
pub fn unequip_item(inventory: &mut Inventory, equipment: &mut Equipment, slot: EquipSlot) -> bool {
    let Some(item) = equipment.set(slot, None) else {
        return false;
    };
    inventory.0.push(item);
    true
}

/// Socket a rune from `runes` into `equipment[slot]`'s socket at
/// `socket_index`, consuming one from the stack. Rejects (returns `false`,
/// no state changed) an unknown rune id, an empty stack, a missing
/// equipped item at `slot`, an out-of-range socket index, or an
/// already-occupied socket — all untrusted-input cases, not invariant
/// violations.
pub fn socket_rune(
    equipment: &mut Equipment,
    runes: &mut RuneInventory,
    rune_library: &RuneLibrary,
    slot: EquipSlot,
    socket_index: usize,
    rune_id: &str,
) -> bool {
    if !rune_library.0.contains_key(rune_id) {
        return false;
    }
    let Some(count) = runes.0.get_mut(rune_id) else {
        return false;
    };
    if *count == 0 {
        return false;
    }
    let Some(item) = equipment.get_mut(slot) else {
        return false;
    };
    let Some(socket) = item.sockets.get_mut(socket_index) else {
        return false;
    };
    if socket.is_some() {
        return false;
    }
    *socket = Some(rune_id.to_string());
    *count -= 1;
    true
}

/// Free and reversible: pulls the rune at `equipment[slot]`'s socket back
/// out into `runes`. Returns `false` if there's no equipped item at
/// `slot`, the socket index is out of range, or it was already empty.
pub fn unsocket_rune(
    equipment: &mut Equipment,
    runes: &mut RuneInventory,
    slot: EquipSlot,
    socket_index: usize,
) -> bool {
    let Some(item) = equipment.get_mut(slot) else {
        return false;
    };
    let Some(socket) = item.sockets.get_mut(socket_index) else {
        return false;
    };
    let Some(rune_id) = socket.take() else {
        return false;
    };
    *runes.0.entry(rune_id).or_insert(0) += 1;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crit_rune() -> RuneDefinition {
        RuneDefinition {
            stat: Stat::CritChance,
            magnitude: 0.05,
        }
    }

    fn item(template_key: &str, socket_count: usize) -> Item {
        Item {
            template_key: template_key.to_string(),
            sockets: vec![None; socket_count],
        }
    }

    #[test]
    fn equip_item_moves_from_inventory_to_the_templates_slot() {
        let mut inventory = Inventory(vec![item("sword", 2)]);
        let mut equipment = Equipment::default();
        let mut items = ItemLibrary::default();
        items.0.insert(
            "sword".to_string(),
            ItemDefinition {
                slot: EquipSlot::Weapon,
                socket_count: 2,
            },
        );

        assert!(equip_item(&mut inventory, &mut equipment, &items, 0));

        assert!(inventory.0.is_empty());
        assert_eq!(equipment.weapon.as_ref().unwrap().template_key, "sword");
    }

    #[test]
    fn equip_item_displaces_the_previously_equipped_item_back_to_inventory() {
        let mut inventory = Inventory(vec![item("new_sword", 0)]);
        let mut equipment = Equipment {
            weapon: Some(item("old_sword", 0)),
            ..Default::default()
        };
        let mut items = ItemLibrary::default();
        items.0.insert(
            "new_sword".to_string(),
            ItemDefinition {
                slot: EquipSlot::Weapon,
                socket_count: 0,
            },
        );

        assert!(equip_item(&mut inventory, &mut equipment, &items, 0));

        assert_eq!(equipment.weapon.as_ref().unwrap().template_key, "new_sword");
        assert_eq!(inventory.0.len(), 1);
        assert_eq!(inventory.0[0].template_key, "old_sword");
    }

    #[test]
    fn equip_item_rejects_an_out_of_range_index() {
        let mut inventory = Inventory::default();
        let mut equipment = Equipment::default();
        let items = ItemLibrary::default();

        assert!(!equip_item(&mut inventory, &mut equipment, &items, 0));
    }

    #[test]
    fn equip_item_rejects_an_unknown_template() {
        let mut inventory = Inventory(vec![item("mystery", 0)]);
        let mut equipment = Equipment::default();
        let items = ItemLibrary::default();

        assert!(!equip_item(&mut inventory, &mut equipment, &items, 0));
        assert_eq!(inventory.0.len(), 1);
    }

    #[test]
    fn unequip_item_moves_it_back_to_inventory() {
        let mut inventory = Inventory::default();
        let mut equipment = Equipment {
            weapon: Some(item("sword", 0)),
            ..Default::default()
        };

        assert!(unequip_item(
            &mut inventory,
            &mut equipment,
            EquipSlot::Weapon
        ));

        assert!(equipment.weapon.is_none());
        assert_eq!(inventory.0[0].template_key, "sword");
    }

    #[test]
    fn unequip_item_rejects_an_empty_slot() {
        let mut inventory = Inventory::default();
        let mut equipment = Equipment::default();

        assert!(!unequip_item(
            &mut inventory,
            &mut equipment,
            EquipSlot::Weapon
        ));
    }

    #[test]
    fn socket_rune_consumes_one_from_the_stack_and_fills_the_socket() {
        let mut equipment = Equipment {
            weapon: Some(item("sword", 1)),
            ..Default::default()
        };
        let mut runes = RuneInventory(HashMap::from([("crit_shard".to_string(), 2)]));
        let mut library = RuneLibrary::default();
        library.0.insert("crit_shard".to_string(), crit_rune());

        assert!(socket_rune(
            &mut equipment,
            &mut runes,
            &library,
            EquipSlot::Weapon,
            0,
            "crit_shard",
        ));

        assert_eq!(runes.0["crit_shard"], 1);
        assert_eq!(
            equipment.weapon.unwrap().sockets[0],
            Some("crit_shard".to_string())
        );
    }

    #[test]
    fn socket_rune_rejects_an_empty_stack() {
        let mut equipment = Equipment {
            weapon: Some(item("sword", 1)),
            ..Default::default()
        };
        let mut runes = RuneInventory::default();
        let mut library = RuneLibrary::default();
        library.0.insert("crit_shard".to_string(), crit_rune());

        assert!(!socket_rune(
            &mut equipment,
            &mut runes,
            &library,
            EquipSlot::Weapon,
            0,
            "crit_shard",
        ));
    }

    #[test]
    fn socket_rune_rejects_an_already_occupied_socket() {
        let mut equipment = Equipment {
            weapon: Some(Item {
                template_key: "sword".to_string(),
                sockets: vec![Some("other_rune".to_string())],
            }),
            ..Default::default()
        };
        let mut runes = RuneInventory(HashMap::from([("crit_shard".to_string(), 1)]));
        let mut library = RuneLibrary::default();
        library.0.insert("crit_shard".to_string(), crit_rune());

        assert!(!socket_rune(
            &mut equipment,
            &mut runes,
            &library,
            EquipSlot::Weapon,
            0,
            "crit_shard",
        ));
        assert_eq!(runes.0["crit_shard"], 1);
    }

    #[test]
    fn unsocket_rune_is_free_and_reversible() {
        let mut equipment = Equipment {
            weapon: Some(Item {
                template_key: "sword".to_string(),
                sockets: vec![Some("crit_shard".to_string())],
            }),
            ..Default::default()
        };
        let mut runes = RuneInventory::default();

        assert!(unsocket_rune(
            &mut equipment,
            &mut runes,
            EquipSlot::Weapon,
            0
        ));

        assert_eq!(equipment.weapon.as_ref().unwrap().sockets[0], None);
        assert_eq!(runes.0["crit_shard"], 1);
    }

    #[test]
    fn unsocket_rune_rejects_an_empty_socket() {
        let mut equipment = Equipment {
            weapon: Some(item("sword", 1)),
            ..Default::default()
        };
        let mut runes = RuneInventory::default();

        assert!(!unsocket_rune(
            &mut equipment,
            &mut runes,
            EquipSlot::Weapon,
            0
        ));
    }

    #[test]
    fn equipment_stat_bonus_sums_matching_runes_across_all_slots() {
        let mut library = RuneLibrary::default();
        library.0.insert("crit_shard".to_string(), crit_rune());
        library.0.insert(
            "speed_shard".to_string(),
            RuneDefinition {
                stat: Stat::MoveSpeed,
                magnitude: 10.0,
            },
        );
        let equipment = Equipment {
            weapon: Some(Item {
                template_key: "sword".to_string(),
                sockets: vec![Some("crit_shard".to_string())],
            }),
            armor: Some(Item {
                template_key: "plate".to_string(),
                sockets: vec![
                    Some("crit_shard".to_string()),
                    Some("speed_shard".to_string()),
                ],
            }),
            helmet: None,
        };

        let crit_bonus = equipment.stat_bonus(Stat::CritChance, &library);
        let speed_bonus = equipment.stat_bonus(Stat::MoveSpeed, &library);

        assert!((crit_bonus - 0.10).abs() < 1e-6);
        assert!((speed_bonus - 10.0).abs() < 1e-6);
    }

    #[test]
    fn pickup_loot_pushes_items_and_increments_rune_counts() {
        let mut inventory = Inventory::default();
        let mut runes = RuneInventory::default();

        pickup_loot(
            &mut inventory,
            &mut runes,
            DroppedLoot::Item(item("sword", 0)),
        );
        pickup_loot(
            &mut inventory,
            &mut runes,
            DroppedLoot::Rune("crit_shard".to_string()),
        );
        pickup_loot(
            &mut inventory,
            &mut runes,
            DroppedLoot::Rune("crit_shard".to_string()),
        );

        assert_eq!(inventory.0.len(), 1);
        assert_eq!(runes.0["crit_shard"], 2);
    }

    #[test]
    fn roll_loot_returns_none_when_drop_chance_fails() {
        let table = LootTable {
            drop_chance: 0.0,
            entries: vec![LootEntry {
                kind: LootKind::Item("sword".to_string()),
                weight: 1.0,
            }],
        };
        let items = ItemLibrary::default();
        let mut rng = rand::rng();

        assert_eq!(roll_loot(&table, &items, &mut rng), None);
    }

    #[test]
    fn roll_loot_returns_none_for_an_empty_table() {
        let table = LootTable::default();
        let items = ItemLibrary::default();
        let mut rng = rand::rng();

        assert_eq!(roll_loot(&table, &items, &mut rng), None);
    }

    #[test]
    fn roll_loot_sizes_a_rolled_items_sockets_from_its_template() {
        let table = LootTable {
            drop_chance: 1.0,
            entries: vec![LootEntry {
                kind: LootKind::Item("sword".to_string()),
                weight: 1.0,
            }],
        };
        let mut items = ItemLibrary::default();
        items.0.insert(
            "sword".to_string(),
            ItemDefinition {
                slot: EquipSlot::Weapon,
                socket_count: 3,
            },
        );
        let mut rng = rand::rng();

        let loot = roll_loot(&table, &items, &mut rng).unwrap();

        match loot {
            DroppedLoot::Item(item) => assert_eq!(item.sockets, vec![None, None, None]),
            DroppedLoot::Rune(_) => panic!("expected an item"),
        }
    }

    #[test]
    fn roll_loot_always_produces_a_rune_from_a_single_rune_entry_table() {
        let table = LootTable {
            drop_chance: 1.0,
            entries: vec![LootEntry {
                kind: LootKind::Rune("crit_shard".to_string()),
                weight: 1.0,
            }],
        };
        let items = ItemLibrary::default();
        let mut rng = rand::rng();

        assert_eq!(
            roll_loot(&table, &items, &mut rng),
            Some(DroppedLoot::Rune("crit_shard".to_string()))
        );
    }
}
