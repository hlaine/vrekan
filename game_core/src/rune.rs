use std::collections::HashSet;

use bevy_ecs::prelude::*;
use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};

/// Rune types this character has ever picked up, independent of whether
/// they've since learned it — see `item::pickup_loot`. Grows monotonically,
/// never shrinks. Replicated/persisted like `KnownRunes`, see
/// MECHANICS.md's Runes section.
#[derive(Component, Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredRunes(pub HashSet<String>);

/// Rune types permanently learned — required (alongside physical stock in
/// `item::RuneInventory`) to socket a rune, regardless of how many physical
/// copies are on hand (see `item::socket_rune`). Mirrors `skill::KnownSkills`'s
/// shape, minus a per-entry level: a rune is either known or not, no ranks.
#[derive(Component, Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnownRunes(pub HashSet<String>);

/// Casts available to spend at a rune-casting panel, granted per level-up
/// (see `rune_casts_granted`) — see MECHANICS.md's Progression section.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UnspentRuneCasts(pub u32);

/// How many candidates a single rune cast offers — MECHANICS.md: "samples 3
/// random candidates ... (fewer if fewer are available)".
pub const RUNE_CAST_OFFER_SIZE: usize = 3;

/// Server-sampled candidates from a rune cast (`request_rune_cast`), awaiting
/// the player's pick (`select_rune_cast`). Replicated so the client can
/// render the offered choices; an empty vec means "no offer pending."
/// Deliberately **not** persisted (no `CharacterSave` field) — an
/// outstanding offer is simply lost on disconnect, the same "server-only
/// resolution state" treatment `weapon_attack::AttackPhase` gets, just
/// replicated here since the client needs to actually show it.
#[derive(Component, Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuneCastOffer(pub Vec<String>);

/// Rune casts granted for a single level-up, gated by `intelligence` — see
/// MECHANICS.md's Attributes section. Tuning data, not a settled curve (see
/// MECHANICS.md's Open questions): always at least the base amount, plus one
/// more every `RUNE_CASTS_INTELLIGENCE_DIVISOR` points invested.
const RUNE_CASTS_BASE_PER_LEVEL: u32 = 1;
const RUNE_CASTS_INTELLIGENCE_DIVISOR: u32 = 5;

pub fn rune_casts_granted(intelligence: u32) -> u32 {
    RUNE_CASTS_BASE_PER_LEVEL + intelligence / RUNE_CASTS_INTELLIGENCE_DIVISOR
}

/// Spends one `UnspentRuneCasts` and samples up to `RUNE_CAST_OFFER_SIZE`
/// candidates (without replacement) from `discovered - known` into `offer`
/// — see MECHANICS.md's "Learning, via casting" section. Rejects (returns
/// `false`, no state changed) if there's no unspent cast to spend, or
/// nothing eligible to offer (every discovered rune is already known) — an
/// empty offer would spend a point for nothing shown, the same
/// reject-a-no-op shape `item::socket_rune` already uses for untrusted
/// input.
pub fn request_rune_cast(
    offer: &mut RuneCastOffer,
    unspent: &mut UnspentRuneCasts,
    discovered: &DiscoveredRunes,
    known: &KnownRunes,
    rng: &mut impl Rng,
) -> bool {
    if unspent.0 == 0 {
        return false;
    }
    // Sorted first so the pre-shuffle order is deterministic (`HashSet`
    // iteration order isn't), then shuffled for the actual random pick.
    let mut candidates: Vec<&String> = discovered.0.difference(&known.0).collect();
    if candidates.is_empty() {
        return false;
    }
    candidates.sort();
    candidates.shuffle(rng);
    candidates.truncate(RUNE_CAST_OFFER_SIZE);

    unspent.0 -= 1;
    offer.0 = candidates.into_iter().cloned().collect();
    true
}

/// Confirms `rune_id` from a pending `offer`: adds it to `known` and clears
/// the offer — see `request_rune_cast`. Rejects (returns `false`, no state
/// changed) if `rune_id` isn't among the offered candidates, including the
/// no-offer-pending case (an empty `offer.0` never contains anything).
pub fn select_rune_cast(offer: &mut RuneCastOffer, known: &mut KnownRunes, rune_id: &str) -> bool {
    if !offer.0.iter().any(|candidate| candidate == rune_id) {
        return false;
    }
    known.0.insert(rune_id.to_string());
    offer.0.clear();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn rune_casts_granted_scales_with_intelligence() {
        assert_eq!(rune_casts_granted(0), 1);
        assert_eq!(rune_casts_granted(4), 1);
        assert_eq!(rune_casts_granted(5), 2);
        assert_eq!(rune_casts_granted(12), 3);
    }

    #[test]
    fn request_rune_cast_rejects_when_no_casts_are_unspent() {
        let mut offer = RuneCastOffer::default();
        let mut unspent = UnspentRuneCasts(0);
        let discovered = DiscoveredRunes(set(&["kenaz"]));
        let known = KnownRunes::default();
        let mut rng = rand::rng();

        assert!(!request_rune_cast(
            &mut offer,
            &mut unspent,
            &discovered,
            &known,
            &mut rng
        ));
        assert!(offer.0.is_empty());
    }

    #[test]
    fn request_rune_cast_rejects_when_nothing_discovered_is_still_unknown() {
        let mut offer = RuneCastOffer::default();
        let mut unspent = UnspentRuneCasts(3);
        let discovered = DiscoveredRunes(set(&["kenaz"]));
        let known = KnownRunes(set(&["kenaz"]));
        let mut rng = rand::rng();

        assert!(!request_rune_cast(
            &mut offer,
            &mut unspent,
            &discovered,
            &known,
            &mut rng
        ));
        assert_eq!(unspent.0, 3);
    }

    #[test]
    fn request_rune_cast_spends_a_point_and_offers_only_undiscovered_or_known_runes() {
        let mut offer = RuneCastOffer::default();
        let mut unspent = UnspentRuneCasts(2);
        let discovered = DiscoveredRunes(set(&["kenaz", "isaz", "thurisaz"]));
        let known = KnownRunes(set(&["thurisaz"]));
        let mut rng = rand::rng();

        assert!(request_rune_cast(
            &mut offer,
            &mut unspent,
            &discovered,
            &known,
            &mut rng
        ));

        assert_eq!(unspent.0, 1);
        assert_eq!(offer.0.len(), 2);
        assert!(offer.0.iter().all(|id| id == "kenaz" || id == "isaz"));
    }

    #[test]
    fn request_rune_cast_offers_at_most_the_offer_size_even_with_more_candidates() {
        let mut offer = RuneCastOffer::default();
        let mut unspent = UnspentRuneCasts(1);
        let discovered = DiscoveredRunes(set(&["a", "b", "c", "d", "e"]));
        let known = KnownRunes::default();
        let mut rng = rand::rng();

        assert!(request_rune_cast(
            &mut offer,
            &mut unspent,
            &discovered,
            &known,
            &mut rng
        ));

        assert_eq!(offer.0.len(), RUNE_CAST_OFFER_SIZE);
    }

    #[test]
    fn select_rune_cast_adds_the_chosen_candidate_and_clears_the_offer() {
        let mut offer = RuneCastOffer(vec!["kenaz".to_string(), "isaz".to_string()]);
        let mut known = KnownRunes::default();

        assert!(select_rune_cast(&mut offer, &mut known, "isaz"));

        assert!(known.0.contains("isaz"));
        assert!(offer.0.is_empty());
    }

    #[test]
    fn select_rune_cast_rejects_a_rune_not_in_the_offer() {
        let mut offer = RuneCastOffer(vec!["kenaz".to_string()]);
        let mut known = KnownRunes::default();

        assert!(!select_rune_cast(&mut offer, &mut known, "isaz"));

        assert!(known.0.is_empty());
        assert_eq!(offer.0, vec!["kenaz".to_string()]);
    }

    #[test]
    fn select_rune_cast_rejects_when_no_offer_is_pending() {
        let mut offer = RuneCastOffer::default();
        let mut known = KnownRunes::default();

        assert!(!select_rune_cast(&mut offer, &mut known, "kenaz"));
    }
}
