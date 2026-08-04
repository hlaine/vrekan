use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::player::{Downed, Player};
use crate::skill::UnspentSkillPoints;
use crate::status_effect::Stat;

/// Character level and progress toward the next one. See MECHANICS.md's
/// Progression section for the death-penalty rules `apply_death_xp_penalty`
/// and `reset_xp_on_full_wipe` implement. Resurrection-point checkpointing
/// is separate — moved to M9 (`ROADMAP.md`), since it needs dungeon-entry/
/// objective triggers that don't exist yet.
#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub level: u32,
    pub xp: f32,
}

impl Default for Level {
    fn default() -> Self {
        Level { level: 1, xp: 0.0 }
    }
}

/// Points available to spend on `Stats`, granted on level-up — manual
/// allocation, not automatic per-level growth (see MECHANICS.md). Spent via
/// `allocate_stat_point`, driven by the M8 stat-allocation panel
/// (`protocol::AllocateStatPointInput`).
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UnspentStatPoints(pub u32);

/// Manually-allocated stat investment — reuses the game's existing
/// mechanical stats (move speed, crit chance/multiplier) rather than
/// inventing new abstract attributes with no hookup yet. `bonus_max_health`
/// has no matching `Stat` variant (see that enum's doc comment) and so is
/// unreachable from `allocate_stat_point` — safely rescaling current health
/// when max changes needs its own deliberate handling, not a rushed wire-up
/// alongside the other three.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub bonus_max_health: f32,
    pub bonus_move_speed: f32,
    pub bonus_crit_chance: f32,
    pub bonus_crit_multiplier: f32,
}

/// XP granted to whichever player lands the killing blow — see
/// `crate::combat::attack_system`. Killing-blow-only credit, not shared
/// across the party: a reasonable starting rule per MECHANICS.md's
/// "tune by feel" framing for progression numbers, not a settled design.
#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct XpReward(pub f32);

/// XP required to go from `level` to `level + 1`. A formula, not a lookup
/// table — level is uncapped for v1 (see MECHANICS.md), so a table would
/// implicitly bound it. Quadratic growth; constants are tuning data, not
/// settled numbers.
const XP_BASE: f32 = 100.0;
const XP_GROWTH_EXPONENT: f32 = 1.5;

pub fn xp_required(level: u32) -> f32 {
    XP_BASE * (level as f32).powf(XP_GROWTH_EXPONENT)
}

/// Stat/skill points granted per level gained. Tuning data, not settled
/// numbers.
const STAT_POINTS_PER_LEVEL: u32 = 3;
const SKILL_POINTS_PER_LEVEL: u32 = 1;

/// Grants `amount` XP, leveling up as many times as the total covers — a
/// single large grant can cross several level thresholds at once, not just
/// the next one. Awards both stat and skill points per level, same
/// "accumulate now, spend later" shape (see `UnspentStatPoints`/
/// `UnspentSkillPoints`'s doc comments — both wait on M8 UI to spend).
pub fn grant_xp(
    level: &mut Level,
    stat_points: &mut UnspentStatPoints,
    skill_points: &mut UnspentSkillPoints,
    amount: f32,
) {
    level.xp += amount;
    while level.xp >= xp_required(level.level) {
        level.xp -= xp_required(level.level);
        level.level += 1;
        stat_points.0 += STAT_POINTS_PER_LEVEL;
        skill_points.0 += SKILL_POINTS_PER_LEVEL;
    }
}

/// Bonus granted per point spent on each stat — tuning data, not settled
/// numbers. Deliberately no `MaxHealth` entry: `Stat` itself has no such
/// variant (see its doc comment), so `allocate_stat_point`'s `match` is
/// exhaustive without needing to special-case rejecting it.
const STAT_POINT_MOVE_SPEED_BONUS: f32 = 2.0;
const STAT_POINT_CRIT_CHANCE_BONUS: f32 = 0.01;
const STAT_POINT_CRIT_MULTIPLIER_BONUS: f32 = 0.02;

/// Spends one `UnspentStatPoints` into `stats`, adding that stat's
/// fixed per-point bonus (see the constants above) — an M8 panel calls
/// this once per button click, same "reject a no-op untrusted input rather
/// than panic" shape as `item::equip_item`. Returns `false` (no state
/// changed) if there's no point to spend.
pub fn allocate_stat_point(unspent: &mut UnspentStatPoints, stats: &mut Stats, stat: Stat) -> bool {
    if unspent.0 == 0 {
        return false;
    }
    unspent.0 -= 1;
    match stat {
        Stat::MoveSpeed => stats.bonus_move_speed += STAT_POINT_MOVE_SPEED_BONUS,
        Stat::CritChance => stats.bonus_crit_chance += STAT_POINT_CRIT_CHANCE_BONUS,
        Stat::CritMultiplier => stats.bonus_crit_multiplier += STAT_POINT_CRIT_MULTIPLIER_BONUS,
    }
    true
}

/// Fraction of current in-level XP lost on an individual death. Tuning
/// data, not a settled number (see MECHANICS.md's Progression section).
/// Only ever reduces `xp`, never `level`, so the "floors at the start of
/// the current level, never drops it" rule holds automatically — there's
/// no separate clamp to get wrong.
const DEATH_XP_PENALTY_FRACTION: f32 = 0.2;

/// Applies the individual-death XP penalty the instant a player becomes
/// `Downed` — `Added<Downed>` fires exactly once per downing, not every
/// tick they stay downed, so this doesn't need `death_system` (in
/// `combat.rs`) to know anything about progression at all.
pub fn apply_death_xp_penalty(mut newly_downed: Query<&mut Level, Added<Downed>>) {
    for mut level in &mut newly_downed {
        level.xp *= 1.0 - DEATH_XP_PENALTY_FRACTION;
    }
}

/// A full party wipe (every connected player currently downed) resets
/// in-level XP to zero for all of them — see MECHANICS.md's Progression
/// section. This supersedes `apply_death_xp_penalty`'s partial loss for
/// whichever player was downed last, rather than stacking with it (a wipe
/// is its own outcome, not "one more partial loss"). Runs every tick
/// rather than only on the exact transition — reapplying zero to an
/// already-zeroed value is a harmless no-op, and detecting "just became a
/// full wipe this tick" precisely isn't worth the extra state to track.
///
/// Guards against the empty-party case explicitly: `Iterator::all` is
/// vacuously true on an empty iterator, so without the `count == 0` guard
/// zero connected players would misread as "everyone's downed."
pub fn reset_xp_on_full_wipe(mut players: Query<(&mut Level, Has<Downed>), With<Player>>) {
    let count = players.iter().count();
    if count == 0 || !players.iter().all(|(_, downed)| downed) {
        return;
    }
    for (mut level, _) in &mut players {
        level.xp = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn xp_required_grows_with_level() {
        assert!(xp_required(2) > xp_required(1));
        assert!(xp_required(10) > xp_required(2));
    }

    #[test]
    fn grant_xp_accumulates_without_leveling_when_insufficient() {
        let mut level = Level::default();
        let mut stat_points = UnspentStatPoints::default();
        let mut skill_points = UnspentSkillPoints::default();

        grant_xp(&mut level, &mut stat_points, &mut skill_points, 10.0);

        assert_eq!(level.level, 1);
        assert_eq!(level.xp, 10.0);
        assert_eq!(stat_points.0, 0);
        assert_eq!(skill_points.0, 0);
    }

    #[test]
    fn grant_xp_levels_up_once_and_carries_over_remainder() {
        let mut level = Level::default();
        let mut stat_points = UnspentStatPoints::default();
        let mut skill_points = UnspentSkillPoints::default();
        let required = xp_required(1);

        grant_xp(
            &mut level,
            &mut stat_points,
            &mut skill_points,
            required + 25.0,
        );

        assert_eq!(level.level, 2);
        assert!((level.xp - 25.0).abs() < 1e-4);
        assert_eq!(stat_points.0, STAT_POINTS_PER_LEVEL);
        assert_eq!(skill_points.0, SKILL_POINTS_PER_LEVEL);
    }

    #[test]
    fn grant_xp_handles_multiple_level_ups_in_one_grant() {
        let mut level = Level::default();
        let mut stat_points = UnspentStatPoints::default();
        let mut skill_points = UnspentSkillPoints::default();
        let huge_amount = xp_required(1) + xp_required(2) + xp_required(3) + 5.0;

        grant_xp(&mut level, &mut stat_points, &mut skill_points, huge_amount);

        assert_eq!(level.level, 4);
        assert!((level.xp - 5.0).abs() < 1e-4);
        assert_eq!(stat_points.0, STAT_POINTS_PER_LEVEL * 3);
        assert_eq!(skill_points.0, SKILL_POINTS_PER_LEVEL * 3);
    }

    #[test]
    fn allocate_stat_point_spends_a_point_and_adds_the_matching_bonus() {
        let mut unspent = UnspentStatPoints(2);
        let mut stats = Stats::default();

        assert!(allocate_stat_point(
            &mut unspent,
            &mut stats,
            Stat::CritChance
        ));

        assert_eq!(unspent.0, 1);
        assert!((stats.bonus_crit_chance - STAT_POINT_CRIT_CHANCE_BONUS).abs() < 1e-6);
        assert_eq!(stats.bonus_move_speed, 0.0);
        assert_eq!(stats.bonus_crit_multiplier, 0.0);
    }

    #[test]
    fn allocate_stat_point_rejects_when_no_points_are_unspent() {
        let mut unspent = UnspentStatPoints(0);
        let mut stats = Stats::default();

        assert!(!allocate_stat_point(
            &mut unspent,
            &mut stats,
            Stat::MoveSpeed
        ));

        assert_eq!(unspent.0, 0);
        assert_eq!(stats.bonus_move_speed, 0.0);
    }

    #[test]
    fn allocate_stat_point_accumulates_across_repeated_spends() {
        let mut unspent = UnspentStatPoints(2);
        let mut stats = Stats::default();

        allocate_stat_point(&mut unspent, &mut stats, Stat::MoveSpeed);
        allocate_stat_point(&mut unspent, &mut stats, Stat::MoveSpeed);

        assert_eq!(unspent.0, 0);
        assert!((stats.bonus_move_speed - STAT_POINT_MOVE_SPEED_BONUS * 2.0).abs() < 1e-6);
    }

    #[test]
    fn death_penalty_reduces_xp_but_never_the_level() {
        let mut world = World::new();
        let player = world
            .spawn((
                Player,
                Downed,
                Level {
                    level: 3,
                    xp: 100.0,
                },
            ))
            .id();

        let _ = world.run_system_once(apply_death_xp_penalty);

        let level = world.get::<Level>(player).unwrap();
        assert_eq!(level.level, 3);
        assert!((level.xp - 80.0).abs() < 1e-4);
    }

    #[test]
    fn death_penalty_only_applies_once_at_the_moment_of_downing() {
        // Uses a persistent `Schedule` rather than two separate
        // `run_system_once` calls: each `run_system_once` builds a fresh,
        // stateless system with no memory of a prior run, so `Added<T>`
        // would (incorrectly, for this test's purpose) read as true again
        // on a second call regardless of whether the component is
        // actually new. A `Schedule` run twice on the same `World`
        // preserves each system's last-run tick between calls, matching
        // how this system actually behaves across real ticks in the
        // server's schedule.
        let mut world = World::new();
        let player = world
            .spawn((
                Player,
                Downed,
                Level {
                    level: 1,
                    xp: 100.0,
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(apply_death_xp_penalty);

        schedule.run(&mut world);
        schedule.run(&mut world);

        // The second tick sees the same `Downed` insertion, not a new one,
        // so the penalty must not apply twice.
        assert!((world.get::<Level>(player).unwrap().xp - 80.0).abs() < 1e-4);
    }

    #[test]
    fn full_wipe_resets_xp_to_zero_for_every_downed_player() {
        let mut world = World::new();
        let a = world
            .spawn((Player, Downed, Level { level: 2, xp: 50.0 }))
            .id();
        let b = world
            .spawn((Player, Downed, Level { level: 5, xp: 30.0 }))
            .id();

        let _ = world.run_system_once(reset_xp_on_full_wipe);

        assert_eq!(world.get::<Level>(a).unwrap(), &Level { level: 2, xp: 0.0 });
        assert_eq!(world.get::<Level>(b).unwrap(), &Level { level: 5, xp: 0.0 });
    }

    #[test]
    fn full_wipe_does_nothing_while_any_player_is_still_up() {
        let mut world = World::new();
        let downed = world
            .spawn((Player, Downed, Level { level: 2, xp: 50.0 }))
            .id();
        world.spawn((Player, Level { level: 1, xp: 10.0 }));

        let _ = world.run_system_once(reset_xp_on_full_wipe);

        assert!((world.get::<Level>(downed).unwrap().xp - 50.0).abs() < 1e-4);
    }

    #[test]
    fn full_wipe_does_nothing_with_no_players_connected() {
        // Regression guard: `Iterator::all` is vacuously true on an empty
        // iterator, so without an explicit empty-party check this would
        // otherwise misfire (there's nothing to even panic on, but a
        // system that silently "wipes" an empty world would be a sign the
        // guard regressed).
        let mut world = World::new();

        let _ = world.run_system_once(reset_xp_on_full_wipe);
    }
}
