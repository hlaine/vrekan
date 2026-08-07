# MECHANICS.md

Mechanic **shapes** for combat, progression, and world systems — structural
decisions that affect data schema and code, not final tuning numbers. Exact
damage values, XP constants, currency amounts, and prices are tuning data
(RON files), meant to change after playtesting. Don't treat any numbers
implied here as final; treat the formulas and data shapes as the thing to
build against.

See `DESIGN.md` for the damage-type list, enemy tiering, and camera/movement
system this plugs into. See `ROADMAP.md` for which milestone builds which
piece of what's described here — this file describes target shapes, some
already built, some not yet.

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
- `base_damage` and `resistance` are each themselves a **sum of several
  layered contributions** (weapon, attributes, equipment, socketed runes,
  active effects), not a single stored number — see "Effective combat
  values are always computed fresh" below for the shape this takes.

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
chase movement for enemies), holding the last facing while stationary —
server-authoritative and replicated, not client-inferred, consistent with
movement itself being server-resolved (see `DESIGN.md`'s Camera & movement
section). This is what "aimed" means in `DESIGN.md`'s Core loop: no
independent mouse-look for v1.

**Exception, added in M8.6 alongside directional melee arcs:** an enemy
snaps its facing directly toward the player it's attacking at the instant
it issues that attack (`game_core::enemy::ai_system`), overriding whatever
movement-derived facing is left over from its chase. Before directional
(cone-gated) attacks existed, an enemy's stale facing while stationary and
attacking was harmless — omnidirectional hit detection didn't care which
way it faced. Once attacks became cone-gated (see Weapons & attack timing
below), a stale facing would make an enemy whiff a target that circled
around it after the chase ended, which reads as a bug, not a readable
telegraph. Players have no equivalent exception: a player's facing is
still purely WASD-derived, since directly controlling facing is already
the player's own input, not something that needs to be pinned to a locked
attack target the way an AI's is.

**Attack range/shape stays pure math, not physics colliders.** Melee (and
later ranged) hit detection is a distance check — and, once directional
attacks exist (see Weapons & attack timing below), an angle-from-facing
check for a cone/arc rather than an omnidirectional radius — resolved
directly in `game_core`, not a spawned physics hitbox/sensor entity. This
keeps combat resolution unit-testable without a running Bevy app (see
`CLAUDE.md`). Reserve real physics colliders for hit detection only if a
shape genuinely can't be expressed as distance/angle math.

### Weapons & attack timing

Each weapon (`ItemDefinition`, `Weapon` slot) carries `damage`,
`damage_type`, `range`, `attack_duration` (windup before the hit applies),
and `recovery` (lockout after the hit lands before the next attack can be
requested) — content data, not a hardcoded per-entity Rust constant. An
empty `Weapon` slot falls back to a baseline unarmed profile rather than
soft-locking the player.

**Attack timing is phased, not a single flat cooldown**: an `AttackRequested`
enters a windup phase, damage resolves once `attack_duration` elapses, then
`recovery` locks out the next attack. A windup in progress is cancelled
outright (no pending hit) if the attacker is stunned, downed, or dies before
it resolves — consistent with `Downed`/`Stunned` already excluding attacking
entirely.

**Attack speed compresses recovery only, never windup.** Windup is the
telegraph a player reads to time a dodge against — shrinking it as
Dexterity/attack-speed investment grows would erode the read-and-react loop
`DESIGN.md`'s dodge-focused combat depends on. Recovery shrinking still
makes investment feel rewarding (back in action faster) without touching
that telegraph. Formula (avoids needing a manual clamp against hitting
zero/negative time):
```
effective_recovery = base_recovery / (1.0 + attack_speed_bonus)
```

**A player is rooted (cannot move) during windup, free to move during
recovery** — a starting assumption reinforcing the telegraph-read design
above, not yet confirmed live.

**Directional (cone) melee arcs**, gated by `Facing`, resolve the previous
open question about whether to move off the omnidirectional radius check —
both players and enemies are cone-gated (`game_core::combat::
is_within_attack_arc`, a single fixed half-angle shared by every weapon for
now, not yet a per-weapon content field), with enemies additionally
snapping facing to their target at the moment of attack so the cone-gate
doesn't make them whiff — see the Facing section above.

**Armor and Helmet items grant a flat resistance % per `DamageType`** —
using the same `DamageType`-keyed shape `Resistances` already uses, not the
fixed `Stat` enum, since damage types are deliberately open-ended content.
Players gain a base (empty-by-default) `Resistances` component, matching
enemies.

### Effective combat values are always computed fresh, never cached

Attack timing, resistance, damage, crit stats, and rune magnitude are all
sums of contributions from multiple sources — base value, active effects,
equipped items, socketed runes, attribute-derived bonuses — recomputed at
point of use (see `attack_system`'s existing crit-stat computation for the
canonical shape). When a system introduces one of these formulas, treat it
as intentionally open to more terms later, not a finished two- or
three-input formula — several already gained inputs from work built after
them (resistance: base → armor equipment → Vitality → Algiz rune; rune
magnitude: base → Intelligence). Caching a computed value into a static
component instead of recomputing it is the single most common source of a
"stat exists but silently does nothing" bug in this codebase so far
(`Stats`' bonus fields sitting unread for a full milestone before M8 wired
them in) — prefer recompute-fresh by default.

### Combat feedback

**Critical hits get a short-lived marker**, not a floating damage number —
`resolve_damage` returns whether the hit crit (not just the damage amount,
which is what it returns today) so a `RecentCrit`-style marker can be
inserted on the target for a fraction of a second and replicated, driving a
light/particle flourish. Deliberately no floating numbers or per-hit
damage text at all — reads as clutter against `DESIGN.md`'s "no
hand-holding" tone; a crit is the one moment worth calling out.

**Critically-low health gets a persistent diegetic tell instead of a
floating bar** — below a low-health threshold (tuning data), a continuous
effect (e.g. a bleeding-out visual) persists until death; nothing shown
above the threshold. Purely a function of the already-replicated
`Health.current / Health.max` ratio, no new replication needed for this
part.

Both of the above apply to **any entity with `Health` and a combat-participant
marker (`Player` or enemy), explicitly excluding destructible objects** (see
Dynamic objects below) — a crate breaking shouldn't visually "bleed."

## Attributes

Four primary attributes — **Might, Dexterity, Vitality, Intelligence** —
are the target of manually-allocated level-up points (see Progression
below), replacing direct-to-secondary-stat spending. Each derives one clear
cluster of secondary effects rather than being a free-floating number:

- **Might** → % bonus weapon damage.
- **Dexterity** → % attack-speed bonus (recovery only, see Weapons & attack
  timing above) + bonus crit chance.
- **Vitality** → bonus max health + a small flat resistance bonus, stacking
  with armor's own resistance contribution.
- **Intelligence** ("rune knowledge" in narrative/UI copy) → gates and
  scales the rune system: how many `UnspentRuneCasts` are granted per
  level, the max number of runes combinable at once (2-9, see Runes below),
  and a multiplier on socketed rune magnitude. See Runes below for exactly
  where each hook lands.

Deliberately **not** attribute-derived: `MoveSpeed` and crit *multiplier*
stay separate, gear/rune-sourced stats. Move speed as a freely stackable
attribute is a known ARPG balance trap (degenerate at either extreme with
little interesting middle ground); keeping crit multiplier separate from
crit chance preserves build diversity (one attribute point shouldn't buy
both halves of the crit equation).

## Resources ("od")

A single resource pool — "od" and "fury/rage" are the same pool, just
flavor-named differently depending on context (e.g. UI copy might call it
"fury" narratively while the underlying component is `Od`). Generation is
dual: slow passive regeneration over time, plus bonus gains from specific
actions and from landing hits in combat. Power attacks consume it. Model
this as one `Od` component (`current`/`max`/`regen_rate`) with discrete
gain events, not two separate pools — not named `Resource`, since that name
is already Bevy's own ECS-resource derive macro. A future shapeshifting
system (see below) and at least one planned rune (Tiwaz — bonus damage at
an Od cost per hit) both spend from this same pool rather than introducing
a second one.

## Progression

**XP curve:** quadratic/exponential family — cost per level increases, exact
constants are tuning data set later. Implement as a formula
(`xp_required(level) = f(level)`), not a lookup table, so it isn't implicitly
bounded by table size — matters because of the uncapped level decision below.

**Level cap:** uncapped for v1. Consistent with `DESIGN.md`'s open "completable
vs. infinite" question — don't let the leveling system implicitly answer that
question by hardcoding a cap. Data structures (stat storage, save format)
should not assume an upper bound on level.

**Growth on level up: manual allocation across three independent point
types**, all spent by the player, none automatic:
- Attribute points, spent on Might/Dexterity/Vitality/Intelligence (see
  Attributes above) — supersedes spending directly on secondary stats.
- Skill points, unchanged from the existing skill-acquisition system.
- `UnspentRuneCasts` (see Runes below) — a level-up grant, gated in count
  by Intelligence, spent at a rune-casting panel rather than immediately
  producing a stat change itself.

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

## Dynamic objects

**Destructibles** (crates, barrels, etc.) are modeled as an enemy template
stripped down: `Health`, `LootTable`, optionally `Resistances`, no AI/attack/
`XpReward` components. `death_system`'s existing drop-and-despawn path
handles them unmodified. They participate in `attack_system`'s ordinary
nearest-target search on equal footing with enemies — a crate can be
auto-targeted ahead of a nearby enemy; this is an accepted tradeoff (crate
placement becomes a real spatial/tactical consideration), not a bug to fix.
Excluded from the crit/bleeding combat-feedback visuals above.

**Movable puzzle objects** are ordinary physics bodies (`RigidBody::Dynamic`
with mass, no `Health`/AI) — pushed exactly the way a player already pushes
an enemy via mass-scaled momentum transfer (see `DESIGN.md`'s Camera &
movement section). No new physics concept.

**`Unlockable`**: a generic gate entity whose collider toggles between
blocking and passable, driven by a `Vec<UnlockCondition>`. **All conditions
in the vector must hold simultaneously for the gate to open — AND
semantics, not OR.** Starts with one variant, `ObjectInZone { object, zone }`
(a pushed object's position overlapping a defined target area), with a
second variant, `HasKeyItem(String)` (see Stateful inventory below), added
once that system lands — the same entity type serves both a pushed-block
puzzle and a key-gated door, and a single gate can require more than one
condition at once (e.g. two different party members each holding their own
key — a genuinely co-op puzzle beat, free once both variants exist).
Puzzle *state* itself doesn't need to persist across sessions — dungeons are
already self-contained, stateless-between-entries instances per `DESIGN.md`.

## Runes: discovery, learning, and combination

Two layers, kept deliberately separate:
- **`RuneInventory`** (already built) — physical stack counts of runes
  currently held. Consumed by socketing, refunded by unsocketing, unchanged.
- **`KnownRunes`** — which rune *types* have been permanently learned, ever.
  Mirrors `KnownSkills`'s shape. Socketing and combining both require the
  rune type to be in `KnownRunes`, regardless of how many physical copies
  are on hand — the one new gate added to the otherwise-unchanged
  `socket_rune`/`unsocket_rune` functions.

**Discovery**: the first time a rune type is ever picked up, it's added to
`DiscoveredRunes`, independent of whether it's later learned. Picking up
further copies of an already-discovered-but-unlearned rune still adds to
`RuneInventory` (still "collectible") without granting knowledge.

**Learning, via casting**: a level-up grants `UnspentRuneCasts` (count
scales with Intelligence). Spending one at a blacksmith/sejdr's rune-casting
panel samples **3 random candidates from `DiscoveredRunes − KnownRunes`**
(fewer if fewer are available); the player picks one to add to `KnownRunes`.
Modeled on the divination framing of rune-casting rather than browsing a
full catalog — bounded choice, some variance even across characters, and it
keeps discovery meaningful (casting only ever offers runes actually
encountered in the world).

**Learning, via direct grant**: a sejdr encounter or cleared objective can
grant a *specific* rune directly into `KnownRunes`, bypassing casting
entirely — a new `Interactable` payload (`grants_rune: Option<String>`),
claimable once per player, alongside the existing effect-grant shape
runestones already use.

**Combining — two distinct paths, not one generic system:**
- **Repeated (same-rune) combining**: consume `combine_count` copies (2-9,
  capped by Intelligence) of the *same* known rune → 1 copy of a stronger
  version. `RuneTemplate` gains `tier: u32` and `upgrades_into: Option<String>`
  (explicit content-authored recipe, not formula-derived from tier number).
  Fully deterministic — the result is always shown plainly before
  confirming; no reason to hide simple arithmetic.
- **Bind-rune combining**: consume a specific *multiset of different* known
  runes (a curated `BindRecipe { inputs: Vec<String>, output: String }`,
  order-independent match) → 1 copy of a unique combined rune. Deliberately
  a curated, hand-authored set of meaningful combinations, not exhaustive
  coverage of every possible input multiset. **The result of a given
  recipe is hidden until first successfully attempted, revealed
  server-wide from that point on** (a single shared discovery state, not
  per-character — simpler, and matches the precedent that item/rune
  *definitions* are universal content, not per-character knowledge);
  unrecognized combinations simply fail with no reveal. Once revealed,
  shown plainly on future attempts, mirroring the repeated-rune case.

Both combine paths cost currency quadratically per tier/step (existing
stub, still tuning data) and require proximity to a panel-opening
`Interactable` (blacksmith or sejdr), reusing the existing
`is_near_interactable_with_panel` mechanism.

**Rune powers, mechanically:** a `RuneDefinition` can grant any combination
of: a flat bonus to an existing `Stat` (as today), a flat bonus to a
specific `DamageType` (summed into `base_damage` the same layered way
weapon/attribute contributions are, per "Effective combat values" above —
new `DamageType` strings, like a "flame" sub-type, cost nothing
architecturally), or a chance-per-hit `EffectDefinition` (full reuse of the
existing skill/melee effect system — no new engine concept for on-hit
effects like a chill or bleed). **One exception needs genuinely new
plumbing**: a *reactive* rune (triggers when the wearer is hit, not when
they land a hit — e.g. a thorns-style effect) has no existing hook, since
today effects only ever apply from the attacker's own effect list. This
needs a new check against the *target's* equipped runes inside
`attack_system`'s damage-application step, not just reuse of what's there.

**Socketed rune contributions (bonus damage, on-hit/reactive effects, and
magnitude) flow into the attacker's effective attack profile at the same
point weapon stats do** — the effective-attack computation reads socketed
runes on the equipped weapon/armor, not just the base item template.

**Rune magnitude scales with Intelligence**: `effective_magnitude =
base_magnitude * (1.0 + intelligence_bonus)`, computed at the same point
`Equipment::stat_bonus` already sums rune contributions — the primary
mechanical hook for the "rune knowledge" attribute.

**Starting rune roster** (loosely drawing on commonly-cited Elder Futhark
meanings, adapted freely — see `DESIGN.md`'s note that this is a fictional/
mythic framing, not a claim to religious authority): Kenaz (flame
bonus-damage), Isaz (on-hit chill effect), Thurisaz (reactive when-hit
stun — the new-plumbing case above), Algiz (flat resistance), Sowilo
(crit stat, tiered via repeated-combining), Tiwaz (bonus damage at an Od
cost per hit), Ansuz (Od regen), Fehu (bonus currency on kill).

## Stateful inventory: key items

**Keys and artifacts are one mechanism, not two** — a `KeyItemTemplate`
(id, display name/flavor text, `consumed_on_use: bool`). A key
(`consumed_on_use: true`) is removed from `KeyItems` after successfully
opening a matching gate; an artifact (`consumed_on_use: false`) is checked
but never removed, and can gate more than one thing. `KeyItems` is a
possession-by-presence set (`HashSet<String>`), the same shape as
`KnownRunes`/`KnownSkills`/`DiscoveredRunes`, replicated and persisted in
`CharacterSave`.

A destructible or enemy can drop a key item directly (`LootKind` gains a
matching variant), or one can be granted via an `Interactable`, mirroring
how runes are grantable both ways.

No new persistence category is needed for "is this gate open" — since
`DESIGN.md` already keeps the overworld itself stateless between sessions,
a gate simply re-checks the party's current possession set every time the
area loads, rather than storing its own open/closed flag anywhere.

## Shapeshifting (future system, not yet scoped into a milestone)

A skill-driven form change, spending Od (not a separate pool — see the
Resources section above). Server-authoritative: alters combat-relevant
stats (damage/damage_type, resistances, move speed) for the shapeshift's
duration, not just a client visual swap — same authority boundary as all
other combat state. Likely lands as a new `SkillKind::Shapeshift` variant
plus a replicated form marker, once the effective-stat computation pattern
(attack stats and derived attributes computed fresh at point of use, not
cached into a static component) is in place to layer onto. Visual
rendering blocked on actual creature-form art, same as the already-deferred
per-item armor visuals.

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
- **Sejdrs** are a specific case of NPC dedicated to rune combination and
  the rune-casting panel — see Runes above. Blacksmiths also gain both
  abilities per that section; a sejdr isn't a blacksmith reskin, it's a
  distinct NPC type that happens to share those capabilities.

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
components, not a fork of the base enemy system. The crit-marker and
critical-health combat-feedback visuals (see Combat feedback above) apply
to bosses automatically, being entity-agnostic — only the dedicated
top-of-screen boss health bar is boss-specific, unbuilt UI.

## Open questions (revisit after relevant milestones are playable)

- Actual damage/resistance/crit numbers, XP curve constants, currency
  amounts and vendor prices — tune by feel once the relevant systems are
  playable, not derived here.
- Exact revive interaction range/timing — a reasonable starting assumption,
  not a settled decision.
- Whether the crit-before-resistance ordering feels right in practice.
- Stacking rules for status effects may need per-effect-type refinement once
  a few real effects exist and can be tested together.
- Whether being rooted during windup (vs. free to move) actually feels
  right once playable — a starting assumption, not confirmed live.
- The shared fixed melee cone half-angle (~50 degrees, see
  `game_core::combat::MELEE_ARC_HALF_ANGLE_RADIANS`) is tuning data, not a
  per-weapon field yet — whether different weapon archetypes should get
  different arc widths is unresolved.
- Exact attack-speed-bonus and rune-magnitude-bonus curves (how Dexterity/
  Intelligence points convert to their respective bonus fractions) — tuning
  data, not derived here.
- Exact combine-count-per-Intelligence curve (how many of the 2-9 range is
  unlocked at what Intelligence value).
- Whether bind-recipe reveal should ever become per-character instead of
  server-wide, if playtesting suggests shared discovery undercuts the
  sense of personal discovery.
