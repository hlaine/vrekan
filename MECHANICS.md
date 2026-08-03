# MECHANICS.md

Mechanic **shapes** for combat, progression, and world systems — structural
decisions that affect data schema and code, not final tuning numbers. Exact
damage values, XP constants, currency amounts, and prices are tuning data
(RON files), meant to change after playtesting. Don't treat any numbers
implied here as final; treat the formulas and data shapes as the thing to
build against.

See `DESIGN.md` for the damage-type list, enemy tiering, and camera/movement
system this plugs into.

## Combat

**Damage formula:**
```
final_damage = base_damage * (crit_roll ? crit_multiplier : 1.0) * (1 - resistance)
```
- Crit is resolved against base damage first, resistance applied after.
- `resistance` is a fraction per `DamageType`, looked up from the target's
  `Resistances` map (0.5 = 50% reduction, negative = weakness/bonus damage
  taken) — see `DESIGN.md`'s damage-type system.
- **Clamp `resistance` to `[-1.0, 1.0]`** (-100% to 100%) at the point damage
  is applied, regardless of what a content file specifies — a safeguard
  against a bad RON value producing negative damage or absurd amplification,
  enforced in `game_core`, not just by convention in content files.
- Crit chance and crit multiplier are v1 stats (character-level fields, not
  deferred).

**Status effects: generic, data-driven system.** Any attack can attach any
effect via data — no hardcoded per-effect combat logic. An effect definition
includes at minimum: type (damage-over-time like bleed, crowd-control like
stun, or a stat-modifying buff like fury), duration, magnitude, and whether
it stacks (and how — refresh duration, add magnitude, or independent stacks,
should be a per-effect data field, not a single global rule). This is the
same content-first principle as enemy/item templates: adding a new effect
should mean a new data entry, not new engine code for that specific effect.

**Facing direction.** Every player *and enemy* has a facing direction
derived from their last non-zero movement (WASD for players, the AI's own
chase movement for enemies), holding the last facing while stationary or
attacking — server-authoritative and replicated, not client-inferred,
consistent with movement itself being server-resolved (see `DESIGN.md`'s
Camera & movement section). This is what "aimed" means in `DESIGN.md`'s
Core loop: no independent mouse-look for v1. Enemies use the exact same
movement-derived mechanism as players, not target-tracking — an enemy
stopped mid-attack keeps facing wherever it was last moving (typically
still toward its target, since it just chased them there), rather than
snapping to face its target directly.

**Attack range/shape stays pure math, not physics colliders.** Melee (and
later ranged) hit detection is a distance check — and, once directional
attacks exist, an angle-from-facing check for a cone/arc rather than an
omnidirectional radius — resolved directly in `game_core`, not a spawned
physics hitbox/sensor entity. This keeps combat resolution unit-testable
without a running Bevy app (see `CLAUDE.md`). Reserve real physics colliders
for hit detection only if a shape genuinely can't be expressed as
distance/angle math.

**Ranges are content data, eventually.** Each weapon/attack's range (and,
once directional attacks exist, its facing-cone angle) belongs in RON
content alongside other weapon/item stats (M7) — the same data-not-engine-
code principle as damage types and enemy templates, not a hardcoded
per-weapon Rust constant. Not yet built: today `MeleeAttack::range` is
still a plain per-entity field set in code (`server`'s constants for the
player, enemy templates for enemies).

## Resources ("od")

A single resource pool — "od" and "fury/rage" are the same pool, just
flavor-named differently depending on context (e.g. UI copy might call it
"fury" narratively while the underlying component is `Od`). Generation is
dual: slow passive regeneration over time, plus bonus gains from specific
actions and from landing hits in combat. Power attacks consume it. Model
this as one `Od` component (`current`/`max`/`regen_rate`) with discrete
gain events, not two separate pools — not named `Resource`, since that name
is already Bevy's own ECS-resource derive macro.

## Progression

**XP curve:** quadratic/exponential family — cost per level increases, exact
constants are tuning data set later. Implement as a formula
(`xp_required(level) = f(level)`), not a lookup table, so it isn't implicitly
bounded by table size — matters because of the uncapped level decision below.

**Level cap:** uncapped for v1. Consistent with `DESIGN.md`'s open "completable
vs. infinite" question — don't let the leveling system implicitly answer that
question by hardcoding a cap. Data structures (stat storage, save format)
should not assume an upper bound on level.

**Stat growth on level up:** manual allocation — leveling grants stat points
the player spends themselves, not automatic per-level stat increases.
Requires a `Stats` component with adjustable fields (not a hardcoded
per-level table) and a level-up/stat-allocation UI panel (M8).

**XP penalty on death:**
- Individual death → partial loss of in-level XP progress. The level itself
  never decreases — the loss floors at the start of the current level, it
  never pushes a player back into the previous level.
- Full party wipe → in-level progress resets to zero (still floors at the
  current level, never drops it).

## Death, downed state, and revive

- On taking lethal damage, a player enters a **downed** state — not an
  immediate respawn. While downed: safe from further attacks (out of combat
  entirely, enemies cannot interact with a downed player), and waits
  indefinitely for a teammate — no timer, no forced auto-action.
- **Revive**: an ally walks to the downed player's position and holds/presses
  an action button to revive them in place.
- **Full wipe**: if all party members are downed simultaneously, there's no
  one left to revive anyone — this state *is* the full-wipe trigger. It
  resolves via auto-respawn at the current resurrection point (not by
  waiting indefinitely, since indefinite waiting only makes sense when at
  least one player is upright).
- **Resurrection point**: updates automatically at fixed checkpoints (dungeon
  entry, reaching an objective) — not manually set by the player.

## Economy: vendors

NPC vendors support a real buy/sell loop — not a barter/quest-reward-only
system. Currency is **individual per player, not shared or pooled** across
the party (consistent with no player-to-player trading — see `DESIGN.md`).
Players can both buy from and sell items to vendors. This is a distinct
system from the loot/forging pipeline in `DESIGN.md`/M7, sharing the same
underlying `Item` data but adding a currency field to `PlayerState` and
vendor-specific content templates (inventory, prices — tuning data, RON).

## NPCs

- **Special characters (objective-givers)**: one-way text only — an NPC
  displays a line and grants an objective. No player dialogue choices, no
  branching, consistent with `DESIGN.md`'s no-cutscenes stance. Implemented
  as flavor text + objective-trigger data, not a dialogue-tree system.
- **Neutral characters**: unkillable, non-combat entities — not "high
  defense," but categorically outside the damage/targeting system entirely.
  Needs an explicit marker (e.g. a `Neutral` or `Untargetable` component)
  distinct from normal enemy/player combat eligibility.
- **Vendors** are a specific case of NPC — see Economy above.

## Enemy visual variants

Visual variants (e.g. "missionary-type1", "missionary-type2") share a base
content template and differ only by a swappable sprite/skin field — not
independent templates duplicating stats, AI, and loot tables for what's
mechanically the same enemy. The `content` crate's enemy schema needs a
variant list (sprite reference per variant) rather than requiring a full new
template per visual reskin.

## Bosses

Bosses get **dedicated mechanics**, not just stronger stats on the standard
enemy template:
- **Phases**: distinct behavior/ability sets triggered by health thresholds
  or timers, not a single flat stat block for the whole fight.
- **Enrage timer**: a time limit that changes boss behavior (typically makes
  the fight harder) if the encounter runs too long.
- **Arena bounds**: a defined boundary for the encounter, separate from the
  general open-world/dungeon movement and leash system in `DESIGN.md`.

This likely needs its own small set of components (`BossPhase`,
`EnrageTimer`, `ArenaBounds`) layered on top of the standard enemy
components, not a fork of the base enemy system.

## Open questions (revisit after relevant milestones are playable)

- Actual damage/resistance/crit numbers, XP curve constants, currency
  amounts and vendor prices — tune by feel once the relevant systems are
  playable, not derived here.
- Exact revive interaction range/timing — a reasonable starting assumption,
  not a settled decision.
- Whether the crit-before-resistance ordering feels right in practice.
- Stacking rules for status effects may need per-effect-type refinement once
  a few real effects exist and can be tested together.
- Whether directional melee arcs (cones, once built) should apply to
  AI-driven enemy attacks too, or stay a player-only refinement over the
  current omnidirectional radius check — enemies already have a facing
  direction now (see Combat section above), so this is purely about
  whether their own attacks should be gated by it.
