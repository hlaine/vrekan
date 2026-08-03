use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// Character level and progress toward the next one. XP-on-death penalties
/// and resurrection-point checkpointing (see MECHANICS.md's Progression
/// section) aren't implemented yet — this is XP/level tracking only.
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
/// allocation, not automatic per-level growth (see MECHANICS.md). No
/// spending mechanic exists yet; that needs an M8 UI panel, so these just
/// accumulate for now.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UnspentStatPoints(pub u32);

/// Manually-allocated stat investment — reuses the game's existing
/// mechanical stats (max health, move speed, crit chance/multiplier)
/// rather than inventing new abstract attributes with no hookup yet.
/// Always zero until an M8 spending UI exists to produce a nonzero value,
/// so these bonuses have no gameplay effect yet — not a gap to patch
/// around, just not needed before then.
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

/// Stat points granted per level gained. Tuning data, not a settled number.
const STAT_POINTS_PER_LEVEL: u32 = 3;

/// Grants `amount` XP, leveling up as many times as the total covers — a
/// single large grant can cross several level thresholds at once, not just
/// the next one.
pub fn grant_xp(level: &mut Level, points: &mut UnspentStatPoints, amount: f32) {
    level.xp += amount;
    while level.xp >= xp_required(level.level) {
        level.xp -= xp_required(level.level);
        level.level += 1;
        points.0 += STAT_POINTS_PER_LEVEL;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xp_required_grows_with_level() {
        assert!(xp_required(2) > xp_required(1));
        assert!(xp_required(10) > xp_required(2));
    }

    #[test]
    fn grant_xp_accumulates_without_leveling_when_insufficient() {
        let mut level = Level::default();
        let mut points = UnspentStatPoints::default();

        grant_xp(&mut level, &mut points, 10.0);

        assert_eq!(level.level, 1);
        assert_eq!(level.xp, 10.0);
        assert_eq!(points.0, 0);
    }

    #[test]
    fn grant_xp_levels_up_once_and_carries_over_remainder() {
        let mut level = Level::default();
        let mut points = UnspentStatPoints::default();
        let required = xp_required(1);

        grant_xp(&mut level, &mut points, required + 25.0);

        assert_eq!(level.level, 2);
        assert!((level.xp - 25.0).abs() < 1e-4);
        assert_eq!(points.0, STAT_POINTS_PER_LEVEL);
    }

    #[test]
    fn grant_xp_handles_multiple_level_ups_in_one_grant() {
        let mut level = Level::default();
        let mut points = UnspentStatPoints::default();
        let huge_amount = xp_required(1) + xp_required(2) + xp_required(3) + 5.0;

        grant_xp(&mut level, &mut points, huge_amount);

        assert_eq!(level.level, 4);
        assert!((level.xp - 5.0).abs() < 1e-4);
        assert_eq!(points.0, STAT_POINTS_PER_LEVEL * 3);
    }
}
