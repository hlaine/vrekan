use bevy_ecs::prelude::*;

use crate::combat::{
    is_within_attack_arc, resolve_melee_hit, AttackRequested, AttackerProgress, CombatStats,
    DamageType, Health, Resistances, MELEE_ARC_HALF_ANGLE_RADIANS,
};
use crate::item::{Equipment, ItemLibrary, RuneLibrary};
use crate::movement::{Facing, Position};
use crate::player::{Downed, Player};
use crate::progression::{Level, Stats, UnspentStatPoints, XpReward};
use crate::skill::{Od, UnspentSkillPoints};
use crate::status_effect::{
    ActiveEffects, EffectDefinition, EffectKind, EffectTarget, Stat, Stunned,
};
use crate::DeltaSeconds;

/// The mechanical shape of a weapon's attack: what an equipped `Weapon`-slot
/// item derives into for the player attack pipeline below to consume (see
/// `content::item::WeaponTemplate`), or `unarmed_weapon_stats` for an empty
/// slot. Distinct from `combat::MeleeAttack` (the enemy-only, server-spawned
/// component `combat::attack_system` still resolves) — see MECHANICS.md's
/// Weapons & attack timing section.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponStats {
    pub damage: f32,
    pub damage_type: DamageType,
    pub range: f32,
    pub attack_duration: f32,
    pub recovery: f32,
}

/// Baseline attack profile for an empty `Weapon` slot — MECHANICS.md: "An
/// empty `Weapon` slot falls back to a baseline unarmed profile rather than
/// soft-locking the player." Tuning data, not final numbers; a single named
/// function rather than inline defaults duplicated wherever an unequipped
/// attack is resolved.
pub fn unarmed_weapon_stats() -> WeaponStats {
    WeaponStats {
        damage: 5.0,
        damage_type: DamageType("primal".to_string()),
        range: 40.0,
        attack_duration: 0.2,
        recovery: 0.5,
    }
}

/// The player's "fury" self-buff — a bonus crit chance on any landed hit,
/// applied unconditionally (chance: 1.0) to the attacker. Shipped since M4
/// as a hardcoded field on the player's old `MeleeAttack` component (see
/// `server/src/main.rs`'s prior `PLAYER_FURY_*` constants); moved here now
/// that players no longer carry `MeleeAttack` at all — this is a player
/// mechanic, not a per-weapon one, so it lives in the shared attack
/// resolution rather than becoming a field on `WeaponStats`.
pub fn player_fury_effect() -> EffectDefinition {
    EffectDefinition {
        id: "fury".to_string(),
        kind: EffectKind::StatModifier {
            stat: Stat::CritChance,
        },
        duration: 3.0,
        magnitude: 0.05,
        stack_mode: crate::status_effect::StackMode::Independent,
        applies_to: EffectTarget::Attacker,
        chance: 1.0,
    }
}

/// Derives the effective `WeaponStats` for `equipment`'s equipped weapon —
/// looked up through `items` (an unknown/missing weapon template falls back
/// to unarmed, same "missing reference is a no-op" convention
/// `Equipment::resistance_bonus` uses) — or `unarmed_weapon_stats()` if no
/// weapon is equipped at all. Computed fresh at point of use every call,
/// never cached, per MECHANICS.md's "Effective combat values are always
/// computed fresh" section — equipping a different weapon takes effect on
/// the very next attack.
pub fn effective_weapon_stats(equipment: Option<&Equipment>, items: &ItemLibrary) -> WeaponStats {
    equipment
        .and_then(|equipment| equipment.weapon.as_ref())
        .and_then(|item| items.0.get(&item.template_key))
        .and_then(|definition| definition.weapon.clone())
        .unwrap_or_else(unarmed_weapon_stats)
}

/// Sum of every source of attack-speed bonus — active effects + equipped
/// gear/runes + Dexterity-derived level points (`Stats::bonus_attack_speed`)
/// — the same 4-way shape (minus a "base," since there's no baseline
/// attack-speed scalar to start from) `resolve_melee_hit` already uses for
/// crit chance. Computed fresh at point of use, never cached — see
/// MECHANICS.md's "Effective combat values are always computed fresh"
/// section. Clamped to non-negative: every current source is additive and
/// non-negative, but `effective_recovery` divides by `1.0 + this value`, so
/// this is a defensive floor against a future negative-magnitude source
/// (e.g. a slow debuff) rather than a case reachable today.
pub fn effective_attack_speed_bonus(
    attacker_effects: &ActiveEffects,
    attacker_equipment: Option<&Equipment>,
    attacker_level_stats: Option<&Stats>,
    runes: &RuneLibrary,
) -> f32 {
    let equipment_bonus = attacker_equipment
        .map(|equipment| equipment.stat_bonus(Stat::AttackSpeed, runes))
        .unwrap_or(0.0);
    let level_bonus = attacker_level_stats
        .map(|stats| stats.bonus_attack_speed)
        .unwrap_or(0.0);
    (attacker_effects.stat_bonus(Stat::AttackSpeed) + equipment_bonus + level_bonus).max(0.0)
}

/// MECHANICS.md's Weapons & attack timing formula: attack speed compresses
/// `base_recovery` only, never windup (see `start_player_windups`, which
/// still sets windup straight from `weapon.attack_duration`). Dividing by
/// `1.0 + attack_speed_bonus` rather than subtracting keeps the result
/// strictly positive for any non-negative bonus, with no separate clamp
/// needed against hitting zero or negative recovery time.
pub fn effective_recovery(base_recovery: f32, attack_speed_bonus: f32) -> f32 {
    base_recovery / (1.0 + attack_speed_bonus.max(0.0))
}

/// Finds the nearest valid attack target for a player `attacker` within
/// `range` and `attacker_facing`'s frontal cone — used to start a player's
/// attack windup (see `start_player_windups`). Mirrors
/// `combat::attack_system`'s own nearest-target selection (no-PvP: a player
/// never selects another player) but is kept as a separate function rather
/// than shared, so player-attack-timing changes can't regress
/// `attack_system`'s already-tested enemy resolution.
pub fn find_attack_target<'a>(
    attacker: Entity,
    attacker_pos: &Position,
    attacker_facing: &Facing,
    range: f32,
    targets: impl Iterator<Item = (Entity, &'a Position, bool)>,
) -> Option<Entity> {
    targets
        .filter(|(entity, _, _)| *entity != attacker)
        .filter(|(_, _, target_is_player)| !*target_is_player)
        .filter(|(_, pos, _)| attacker_pos.distance(pos) <= range)
        .filter(|(_, pos, _)| {
            is_within_attack_arc(
                attacker_pos,
                attacker_facing,
                pos,
                MELEE_ARC_HALF_ANGLE_RADIANS,
            )
        })
        .min_by(|(_, a, _), (_, b, _)| {
            attacker_pos
                .distance(a)
                .total_cmp(&attacker_pos.distance(b))
        })
        .map(|(entity, _, _)| entity)
}

/// Phased attack state for player weapon attacks — windup, then recovery,
/// replacing `AttackTimer`'s flat cooldown for players specifically (see
/// MECHANICS.md's Weapons & attack timing section). Player-only for M8.6:
/// enemies keep `combat::AttackTimer`'s flat-cooldown resolution unchanged,
/// a decision made explicitly rather than assumed (see `DECISIONS.md`'s
/// M8.6 planning entry). Not replicated — server-only resolution state,
/// mirroring `ActiveEffects`; a small replicated summary for the HUD's
/// windup-vs-recovery display is added once this is wired into `client`.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq)]
pub enum AttackPhase {
    #[default]
    Idle,
    /// `target` is locked in when windup starts and re-validated (not
    /// re-selected) once `remaining` reaches zero — see MECHANICS.md.
    Windup {
        remaining: f32,
        target: Entity,
    },
    Recovery {
        remaining: f32,
    },
}

/// What `tick_attack_phase` wants the caller to do this tick — `None` means
/// "nothing new," covering both `Idle` and an in-progress windup/recovery
/// that hasn't crossed a boundary yet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AttackPhaseEvent {
    /// Windup just completed: resolve the hit against `target` now (the
    /// caller must still re-validate it — see `AttackPhase::Windup`'s doc
    /// comment). The phase has already moved on to `Recovery`.
    HitReady { target: Entity },
    /// An in-progress windup was cancelled outright (the attacker went
    /// `Stunned`/`Downed` before it resolved) — no hit, no recovery, back to
    /// `Idle` immediately. See MECHANICS.md.
    Cancelled,
}

/// Advances `phase` by `dt`, returning the new phase and, when relevant,
/// what the caller needs to react to. `incapacitated` (the attacker is
/// currently `Stunned` or `Downed`) only affects an in-progress `Windup` —
/// cancelling it outright per MECHANICS.md; it deliberately does *not*
/// interrupt `Recovery`, which is just a lockout timer winding down, not an
/// action that can be cancelled. `recovery` is the duration to enter once a
/// windup completes, supplied by the caller (from the attack's effective
/// weapon stats) rather than looked up here, so this function stays free of
/// any `content`/`Equipment` dependency.
pub fn tick_attack_phase(
    phase: AttackPhase,
    dt: f32,
    incapacitated: bool,
    recovery: f32,
) -> (AttackPhase, Option<AttackPhaseEvent>) {
    match phase {
        AttackPhase::Idle => (AttackPhase::Idle, None),
        AttackPhase::Windup { remaining, target } => {
            if incapacitated {
                return (AttackPhase::Idle, Some(AttackPhaseEvent::Cancelled));
            }
            let remaining = remaining - dt;
            if remaining <= 0.0 {
                (
                    AttackPhase::Recovery {
                        remaining: recovery.max(0.0),
                    },
                    Some(AttackPhaseEvent::HitReady { target }),
                )
            } else {
                (AttackPhase::Windup { remaining, target }, None)
            }
        }
        AttackPhase::Recovery { remaining } => {
            let remaining = remaining - dt;
            if remaining <= 0.0 {
                (AttackPhase::Idle, None)
            } else {
                (AttackPhase::Recovery { remaining }, None)
            }
        }
    }
}

type PlayerAttackers<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Position,
        &'static Facing,
        &'static mut AttackPhase,
        Option<&'static Equipment>,
    ),
    (With<Player>, Without<Downed>, Without<Stunned>),
>;

type AttackTargets<'w, 's> =
    Query<'w, 's, (Entity, &'static Position, Has<Player>), (With<Health>, Without<Downed>)>;

/// Consumes `AttackRequested` events for player attackers (the same events
/// `combat::attack_system` reads for enemy attackers — `MessageReader`
/// tracks its own read cursor per system, so two systems reading the same
/// message stream is safe and doesn't steal events from each other): if the
/// player is `AttackPhase::Idle`, resolves the effective weapon stats and
/// finds a target within range and the frontal cone, then locks it in and
/// enters `Windup`. No target in range/arc, or already mid-attack, is a
/// silent no-op — same "nothing eligible, nothing happens" shape
/// `attack_system` already uses.
pub fn start_player_windups(
    mut events: MessageReader<AttackRequested>,
    mut attackers: PlayerAttackers,
    targets: AttackTargets,
    items: Res<ItemLibrary>,
) {
    for event in events.read() {
        let Ok((entity, pos, facing, mut phase, equipment)) = attackers.get_mut(event.attacker)
        else {
            continue;
        };
        if *phase != AttackPhase::Idle {
            tracing::debug!(?entity, ?phase, "attack input ignored: attacker not Idle");
            continue;
        }

        let weapon = effective_weapon_stats(equipment, &items);
        let target = find_attack_target(entity, pos, facing, weapon.range, targets.iter());
        let Some(target) = target else {
            tracing::debug!(
                ?entity,
                ?pos,
                ?facing,
                range = weapon.range,
                "no target in range/cone"
            );
            continue;
        };

        tracing::debug!(
            ?entity,
            ?target,
            attack_duration = weapon.attack_duration,
            damage = weapon.damage,
            "starting windup"
        );
        *phase = AttackPhase::Windup {
            remaining: weapon.attack_duration.max(0.0),
            target,
        };
    }
}

type TickingPlayers<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut AttackPhase,
        Has<Stunned>,
        Has<Downed>,
        &'static CombatStats,
        Option<&'static mut Level>,
        Option<&'static mut UnspentStatPoints>,
        Option<&'static mut UnspentSkillPoints>,
        Option<&'static mut Od>,
        Option<&'static Equipment>,
        Option<&'static Stats>,
    ),
    With<Player>,
>;

/// Advances every player's `AttackPhase` by one tick (via `tick_attack_phase`,
/// using the *current* effective weapon's `recovery`, compressed by
/// `effective_attack_speed_bonus`/`effective_recovery` (M8.7) — both
/// re-derived fresh each tick like everything else here, so switching
/// weapons or gaining an attack-speed buff mid-recovery takes effect
/// immediately rather than honoring a stale duration) and resolves the hit
/// through `combat::resolve_melee_hit` whenever a windup completes.
/// `Stunned`/`Downed` cancel an in-progress windup outright (see
/// `tick_attack_phase`); the player's "fury" self-buff (`player_fury_effect`)
/// applies on every landed hit, same as it did when it lived on the old
/// `MeleeAttack` component.
// Same false-positive shape `resolve_melee_hit`/`attack_system` already
// carry this exact allow for — a flat Bevy system parameter list is the
// idiomatic shape here, not a sign this needs bundling into a struct.
#[allow(clippy::too_many_arguments)]
pub fn tick_player_attack_phases(
    mut commands: Commands,
    delta: Res<DeltaSeconds>,
    mut players: TickingPlayers,
    targets_equipment: Query<&Equipment>,
    targets_level_stats: Query<&Stats>,
    mut healths: Query<(&mut Health, Option<&Resistances>, Option<&XpReward>)>,
    mut all_effects: Query<&mut ActiveEffects>,
    items: Res<ItemLibrary>,
    runes: Res<RuneLibrary>,
) {
    let dt = delta.0;
    let mut rng = rand::rng();
    let fury = player_fury_effect();
    let no_effects = ActiveEffects::default();
    for (
        entity,
        mut phase,
        stunned,
        downed,
        stats,
        level,
        stat_points,
        skill_points,
        od,
        equipment,
        level_stats,
    ) in &mut players
    {
        let weapon = effective_weapon_stats(equipment, &items);
        let attacker_effects = all_effects.get(entity).unwrap_or(&no_effects);
        let attack_speed_bonus =
            effective_attack_speed_bonus(attacker_effects, equipment, level_stats, &runes);
        let recovery = effective_recovery(weapon.recovery.max(0.0), attack_speed_bonus);
        let (new_phase, event) = tick_attack_phase(*phase, dt, stunned || downed, recovery);
        *phase = new_phase;

        let Some(AttackPhaseEvent::HitReady { target }) = event else {
            continue;
        };
        tracing::debug!(?entity, ?target, "windup complete, resolving hit");

        let progress = AttackerProgress {
            level: level.map(|level| level.into_inner()),
            stat_points: stat_points.map(|points| points.into_inner()),
            skill_points: skill_points.map(|points| points.into_inner()),
            od: od.map(|od| od.into_inner()),
        };
        let hit = resolve_melee_hit(
            entity,
            target,
            weapon.damage,
            &weapon.damage_type,
            std::slice::from_ref(&fury),
            stats,
            equipment,
            level_stats,
            progress,
            targets_equipment.get(target).ok(),
            targets_level_stats.get(target).ok(),
            &items,
            &mut healths,
            &mut all_effects,
            &runes,
            &mut commands,
            &mut rng,
        );
        tracing::debug!(?entity, hit, damage = weapon.damage, "hit resolved");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{EquipSlot, Item, ItemDefinition};
    use bevy_ecs::system::RunSystemOnce;

    fn dummy_entity() -> Entity {
        World::new().spawn_empty().id()
    }

    #[test]
    fn effective_recovery_at_zero_attack_speed_bonus_returns_base_recovery() {
        assert!((effective_recovery(0.5, 0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn effective_recovery_shrinks_as_attack_speed_bonus_grows() {
        // 1.0 bonus halves recovery: divide by (1.0 + 1.0).
        assert!((effective_recovery(0.5, 1.0) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn effective_recovery_never_reaches_zero_at_an_extreme_bonus() {
        let recovery = effective_recovery(0.5, 1_000_000.0);

        assert!(recovery > 0.0);
        assert!(recovery < 0.001);
    }

    #[test]
    fn effective_attack_speed_bonus_sums_dexterity_level_stats_equipment_and_effects() {
        let mut world = World::new();
        let attacker = world
            .spawn((
                ActiveEffects::default(),
                Stats {
                    bonus_attack_speed: 0.3,
                    ..Default::default()
                },
            ))
            .id();
        let mut effects = world.get_mut::<ActiveEffects>(attacker).unwrap();
        effects.apply(EffectDefinition {
            id: "haste".to_string(),
            kind: EffectKind::StatModifier {
                stat: Stat::AttackSpeed,
            },
            duration: 5.0,
            magnitude: 0.2,
            stack_mode: crate::status_effect::StackMode::Independent,
            applies_to: EffectTarget::Attacker,
            chance: 1.0,
        });
        let effects = world.get::<ActiveEffects>(attacker).unwrap();
        let stats = world.get::<Stats>(attacker).unwrap();
        let runes = RuneLibrary::default();

        let bonus = effective_attack_speed_bonus(effects, None, Some(stats), &runes);

        assert!((bonus - 0.5).abs() < 1e-6);
    }

    #[test]
    fn tick_attack_phase_idle_stays_idle() {
        let (phase, event) = tick_attack_phase(AttackPhase::Idle, 1.0, false, 0.5);

        assert_eq!(phase, AttackPhase::Idle);
        assert_eq!(event, None);
    }

    #[test]
    fn tick_attack_phase_windup_counts_down_without_completing() {
        let target = dummy_entity();
        let phase = AttackPhase::Windup {
            remaining: 0.5,
            target,
        };

        let (phase, event) = tick_attack_phase(phase, 0.2, false, 0.3);

        assert_eq!(
            phase,
            AttackPhase::Windup {
                remaining: 0.3,
                target
            }
        );
        assert_eq!(event, None);
    }

    #[test]
    fn tick_attack_phase_windup_completion_enters_recovery_and_reports_hit_ready() {
        let target = dummy_entity();
        let phase = AttackPhase::Windup {
            remaining: 0.2,
            target,
        };

        let (phase, event) = tick_attack_phase(phase, 0.3, false, 0.4);

        assert_eq!(phase, AttackPhase::Recovery { remaining: 0.4 });
        assert_eq!(event, Some(AttackPhaseEvent::HitReady { target }));
    }

    #[test]
    fn tick_attack_phase_incapacitated_cancels_windup_regardless_of_remaining() {
        let target = dummy_entity();
        let phase = AttackPhase::Windup {
            remaining: 999.0,
            target,
        };

        let (phase, event) = tick_attack_phase(phase, 0.01, true, 0.4);

        assert_eq!(phase, AttackPhase::Idle);
        assert_eq!(event, Some(AttackPhaseEvent::Cancelled));
    }

    #[test]
    fn tick_attack_phase_recovery_counts_down_without_completing() {
        let phase = AttackPhase::Recovery { remaining: 0.5 };

        let (phase, event) = tick_attack_phase(phase, 0.2, false, 0.0);

        assert_eq!(phase, AttackPhase::Recovery { remaining: 0.3 });
        assert_eq!(event, None);
    }

    #[test]
    fn tick_attack_phase_recovery_completion_returns_to_idle() {
        let phase = AttackPhase::Recovery { remaining: 0.2 };

        let (phase, event) = tick_attack_phase(phase, 0.3, false, 0.0);

        assert_eq!(phase, AttackPhase::Idle);
        assert_eq!(event, None);
    }

    #[test]
    fn tick_attack_phase_recovery_ignores_incapacitated() {
        let phase = AttackPhase::Recovery { remaining: 0.5 };

        let (phase, event) = tick_attack_phase(phase, 0.2, true, 0.0);

        assert_eq!(phase, AttackPhase::Recovery { remaining: 0.3 });
        assert_eq!(event, None);
    }

    #[test]
    fn effective_weapon_stats_falls_back_to_unarmed_with_no_equipment() {
        let items = ItemLibrary::default();

        let stats = effective_weapon_stats(None, &items);

        assert_eq!(stats, unarmed_weapon_stats());
    }

    #[test]
    fn effective_weapon_stats_falls_back_to_unarmed_with_an_empty_weapon_slot() {
        let items = ItemLibrary::default();
        let equipment = Equipment::default();

        let stats = effective_weapon_stats(Some(&equipment), &items);

        assert_eq!(stats, unarmed_weapon_stats());
    }

    #[test]
    fn effective_weapon_stats_reads_the_equipped_weapons_template() {
        let mut items = ItemLibrary::default();
        let sword_stats = WeaponStats {
            damage: 20.0,
            damage_type: DamageType("primal".to_string()),
            range: 70.0,
            attack_duration: 0.4,
            recovery: 0.6,
        };
        items.0.insert(
            "steel_sword".to_string(),
            ItemDefinition {
                slot: EquipSlot::Weapon,
                socket_count: 1,
                sell_value: 10,
                weapon: Some(sword_stats.clone()),
                resistances: Resistances::default(),
            },
        );
        let equipment = Equipment {
            weapon: Some(Item {
                template_key: "steel_sword".to_string(),
                sockets: vec![None],
            }),
            ..Default::default()
        };

        let stats = effective_weapon_stats(Some(&equipment), &items);

        assert_eq!(stats, sword_stats);
    }

    #[test]
    fn effective_weapon_stats_falls_back_to_unarmed_for_an_unknown_template() {
        let items = ItemLibrary::default();
        let equipment = Equipment {
            weapon: Some(Item {
                template_key: "mystery".to_string(),
                sockets: vec![],
            }),
            ..Default::default()
        };

        let stats = effective_weapon_stats(Some(&equipment), &items);

        assert_eq!(stats, unarmed_weapon_stats());
    }

    #[test]
    fn find_attack_target_picks_the_nearest_in_range_target_within_the_cone() {
        let mut world = World::new();
        let attacker = world.spawn_empty().id();
        let attacker_pos = Position { x: 0.0, y: 0.0 };
        let facing = Facing { x: 1.0, y: 0.0 };
        let near = world.spawn_empty().id();
        let near_pos = Position { x: 2.0, y: 0.0 };
        let far = world.spawn_empty().id();
        let far_pos = Position { x: 4.0, y: 0.0 };

        let target = find_attack_target(
            attacker,
            &attacker_pos,
            &facing,
            10.0,
            vec![(near, &near_pos, false), (far, &far_pos, false)].into_iter(),
        );

        assert_eq!(target, Some(near));
    }

    #[test]
    fn find_attack_target_excludes_players_no_pvp() {
        let mut world = World::new();
        let attacker = world.spawn_empty().id();
        let attacker_pos = Position { x: 0.0, y: 0.0 };
        let facing = Facing { x: 1.0, y: 0.0 };
        let teammate = world.spawn_empty().id();
        let teammate_pos = Position { x: 1.0, y: 0.0 };

        let target = find_attack_target(
            attacker,
            &attacker_pos,
            &facing,
            10.0,
            vec![(teammate, &teammate_pos, true)].into_iter(),
        );

        assert_eq!(target, None);
    }

    #[test]
    fn find_attack_target_excludes_a_target_outside_the_facing_cone() {
        let mut world = World::new();
        let attacker = world.spawn_empty().id();
        let attacker_pos = Position { x: 0.0, y: 0.0 };
        let facing = Facing { x: 0.0, y: 1.0 };
        let behind = world.spawn_empty().id();
        let behind_pos = Position { x: 0.0, y: -1.0 };

        let target = find_attack_target(
            attacker,
            &attacker_pos,
            &facing,
            10.0,
            vec![(behind, &behind_pos, false)].into_iter(),
        );

        assert_eq!(target, None);
    }

    #[test]
    fn start_player_windups_locks_a_target_and_enters_windup() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        world.init_resource::<ItemLibrary>();

        let attacker = world
            .spawn((
                Player,
                Position { x: 0.0, y: 0.0 },
                Facing { x: 1.0, y: 0.0 },
                AttackPhase::Idle,
            ))
            .id();
        let enemy = world
            .spawn((Position { x: 2.0, y: 0.0 }, Health::new(30.0)))
            .id();

        world
            .resource_mut::<Messages<AttackRequested>>()
            .write(AttackRequested { attacker });

        let _ = world.run_system_once(start_player_windups);

        match world.get::<AttackPhase>(attacker).unwrap() {
            AttackPhase::Windup { target, .. } => assert_eq!(*target, enemy),
            other => panic!("expected Windup, got {other:?}"),
        }
    }

    #[test]
    fn start_player_windups_is_a_no_op_when_not_idle() {
        let mut world = World::new();
        world.init_resource::<Messages<AttackRequested>>();
        world.init_resource::<ItemLibrary>();

        let attacker = world
            .spawn((
                Player,
                Position { x: 0.0, y: 0.0 },
                Facing { x: 1.0, y: 0.0 },
                AttackPhase::Recovery { remaining: 0.2 },
            ))
            .id();
        world.spawn((Position { x: 2.0, y: 0.0 }, Health::new(30.0)));

        world
            .resource_mut::<Messages<AttackRequested>>()
            .write(AttackRequested { attacker });

        let _ = world.run_system_once(start_player_windups);

        assert_eq!(
            *world.get::<AttackPhase>(attacker).unwrap(),
            AttackPhase::Recovery { remaining: 0.2 }
        );
    }

    #[test]
    fn tick_player_attack_phases_resolves_a_hit_on_windup_completion() {
        let mut world = World::new();
        world.insert_resource(DeltaSeconds(1.0));
        world.init_resource::<ItemLibrary>();
        world.init_resource::<RuneLibrary>();

        let target = world
            .spawn((Health::new(30.0), ActiveEffects::default()))
            .id();
        let attacker = world
            .spawn((
                Player,
                AttackPhase::Windup {
                    remaining: 0.5,
                    target,
                },
                CombatStats {
                    crit_chance: 0.0,
                    crit_multiplier: 1.0,
                },
                ActiveEffects::default(),
            ))
            .id();

        let _ = world.run_system_once(tick_player_attack_phases);

        // unarmed_weapon_stats().damage == 5.0
        assert_eq!(world.get::<Health>(target).unwrap().current, 25.0);
        assert_eq!(
            *world.get::<AttackPhase>(attacker).unwrap(),
            AttackPhase::Recovery {
                remaining: unarmed_weapon_stats().recovery
            }
        );
    }

    #[test]
    fn tick_player_attack_phases_whiffs_when_the_target_vanished_during_windup() {
        let mut world = World::new();
        world.insert_resource(DeltaSeconds(1.0));
        world.init_resource::<ItemLibrary>();
        world.init_resource::<RuneLibrary>();

        let vanished_target = dummy_entity();
        let attacker = world
            .spawn((
                Player,
                AttackPhase::Windup {
                    remaining: 0.5,
                    target: vanished_target,
                },
                CombatStats {
                    crit_chance: 0.0,
                    crit_multiplier: 1.0,
                },
                ActiveEffects::default(),
            ))
            .id();

        let _ = world.run_system_once(tick_player_attack_phases);

        // No panic, and the phase still advances to Recovery even though
        // the hit itself whiffed.
        assert_eq!(
            *world.get::<AttackPhase>(attacker).unwrap(),
            AttackPhase::Recovery {
                remaining: unarmed_weapon_stats().recovery
            }
        );
    }

    #[test]
    fn tick_player_attack_phases_cancels_on_incapacitation_without_resolving() {
        let mut world = World::new();
        world.insert_resource(DeltaSeconds(1.0));
        world.init_resource::<ItemLibrary>();
        world.init_resource::<RuneLibrary>();

        let target = world
            .spawn((Health::new(30.0), ActiveEffects::default()))
            .id();
        let attacker = world
            .spawn((
                Player,
                Stunned,
                AttackPhase::Windup {
                    remaining: 999.0,
                    target,
                },
                CombatStats {
                    crit_chance: 0.0,
                    crit_multiplier: 1.0,
                },
                ActiveEffects::default(),
            ))
            .id();

        let _ = world.run_system_once(tick_player_attack_phases);

        assert_eq!(world.get::<Health>(target).unwrap().current, 30.0);
        assert_eq!(
            *world.get::<AttackPhase>(attacker).unwrap(),
            AttackPhase::Idle
        );
    }
}
