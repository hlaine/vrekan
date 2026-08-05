use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// A player's individual currency balance — not shared/pooled across the
/// party, consistent with no player-to-player trading (see `DESIGN.md`,
/// `MECHANICS.md`'s Economy section). This is the shared foundation for
/// both the vendor economy and forging cost (M7 part 2); neither spend
/// side is wired up yet — only the balance and the loot source that fills
/// it (see `item::LootKind::Currency`) exist so far.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Currency(pub u32);
