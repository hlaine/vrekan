pub mod combat;
pub mod economy;
pub mod enemy;
pub mod interact;
pub mod item;
pub mod movement;
pub mod player;
pub mod progression;
pub mod revive;
pub mod skill;
pub mod status_effect;
pub mod weapon_attack;

pub use combat::{
    is_within_attack_arc, AttackRequested, AttackTimer, CombatStats, DamageType, Health,
    MeleeAttack, Resistances, MELEE_ARC_HALF_ANGLE_RADIANS,
};
pub use economy::{
    buy_item, sell_item, socketed_item_sell_value, Currency, VendorLibrary, VendorListing,
};
pub use enemy::{Aggro, Enemy, EnemyKind};
pub use interact::{
    interact_or_pickup_system, nearest_interactable_in_range, nearest_interactable_with_panel,
    InteractOrPickupRequested, Interactable, InteractableDefinition, InteractableLibrary,
    FORGING_PANEL_ID, PICKUP_RANGE, VENDOR_PANEL_ID,
};
pub use item::{
    equip_item, pickup_loot, roll_loot, socket_rune, unequip_item, unsocket_rune, DroppedLoot,
    EquipSlot, Equipment, Inventory, Item, ItemDefinition, ItemDrop, ItemLibrary, LootEntry,
    LootKind, LootTable, RuneDefinition, RuneInventory, RuneLibrary,
};
pub use movement::{leash_system, Facing, MoveSpeed, Position, Velocity, LEASH_DISTANCE};
pub use player::{Downed, Player};
pub use progression::{
    allocate_stat_point, apply_death_xp_penalty, grant_xp, reset_xp_on_full_wipe, xp_required,
    Level, Stats, UnspentStatPoints, XpReward,
};
pub use revive::{revive_system, Reviving};
pub use skill::{
    learn_skill, skill_cast_system, tick_od_regen, tick_skill_cooldowns, KnownSkills, Od,
    SkillCastRequested, SkillCooldowns, SkillDefinition, SkillKind, SkillLibrary,
    UnspentSkillPoints,
};
pub use status_effect::{
    tick_status_effects, ActiveEffects, EffectDefinition, EffectKind, EffectTarget, StackMode,
    Stat, Stunned,
};
pub use weapon_attack::{
    effective_weapon_stats, find_attack_target, start_player_windups, tick_player_attack_phases,
    unarmed_weapon_stats, AttackPhase, AttackPhaseEvent, WeaponStats,
};

use bevy_ecs::prelude::*;

/// Seconds elapsed since the last tick. Kept as a plain resource (rather than
/// depending on `bevy_time`) so `game_core` stays free of any dependency
/// beyond `bevy_ecs` — the client (and later the server) is responsible for
/// updating this from its own clock each frame/tick.
#[derive(Resource, Default)]
pub struct DeltaSeconds(pub f32);
