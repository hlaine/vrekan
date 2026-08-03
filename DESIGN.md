# DESIGN.md

Game design context. Living document — expand a section at a time as systems are
actually built, rather than speculatively ahead of time. See `CLAUDE.md` for
engineering conventions.

## Vision

An endgame-focused, loot-grind action RPG (ARPG): no cutscenes, no branching
dialogue. A pagan farmer fights an invading Christian order, escalating from
human missionaries through church hierarchy to divine beings, toward a
gnostic-flavored confrontation with the invading god's creation itself. Story
is conveyed through objectives and light environmental/UI text, not scripted
scenes.

**Tone: dark and grim, not high-fantasy or whimsical.** The pagan side reads
as raw, aggressive resistance — a desperate, visceral defense of land and
belief — rather than a heroic-fantasy crusade. This is a fictional/mythic
framing of the conflict, not a literal historical or theological statement.
Where tone is relevant (art direction, audio, enemy/ability flavor text and
naming), lean into grounded and brutal rather than ornate or fantastical —
but don't force tone language into places it has no bearing, like damage-type
resistance numbers or networking code.

## Core loop

Real-time, direct movement (WASD/stick) with aimed abilities — dodge-focused, not
click-to-move. "Aimed" means facing-based, not independent mouse-look: a
character's facing direction is derived from their last movement input, and
directional attacks are checked against that facing — see `MECHANICS.md`'s
Combat section. Player roams an open overworld with continuously respawning
enemies, and enters explicit dungeon instances to complete objectives. The core
drive is grinding loot, levels, and upgrades to take on progressively stronger
enemies. Player-to-player trading is out of scope — the only way to give
another player an item directly is to drop it for them in co-op. NPC vendors
(buy/sell, individual currency per player) are a separate, in-scope system —
see `MECHANICS.md`.

## World & session structure

- **Per-party instance.** Each co-op group gets its own private overworld +
  dungeon instances — a self-contained session per party, not a persistent
  shared world.
- **No durable world state.** The overworld doesn't need to remember what
  happened in it; enemies simply respawn. Nothing about the world itself needs to
  be saved between sessions.
- **Dungeons are entered explicitly** for specific objectives/tasks — self-
  contained instances, not part of the continuous overworld.
- **Only the player's character persists** across sessions: level, stats, items,
  skills. Save architecture specifics are an open question (see below).

## Camera & movement

- **Shared camera per client.** Each client's camera is driven by *all* party
  members' positions (already replicated via networking from M3), not just
  the local player — centered on the party's midpoint, zooming out as the
  party spreads apart and back in as they regroup. Both clients see a
  consistent, synchronized-feeling view without needing any new networking
  beyond what's already replicated.
- **Hard leash, server-authoritative.** Players cannot move further apart than
  the camera's max zoom allows — an invisible boundary enforced in the
  server's movement resolution (`game_core`), not a client-side cosmetic
  clamp. A client-side-only leash would be both cheatable and a desync risk;
  the server is the source of truth here exactly as it is for all other
  simulation state. The client adds a lightweight visual indicator when at
  the limit.
- **Movement/collision is purely 2D.** Elevation (mountains, hills) is
  cosmetic only — no climbing, no verticality in gameplay. What determines
  passability is the collision layer, not visual height; terrain that reads
  as elevated can still be walkable or blocking independent of how it looks.
- **Collision authoring: freeform polygon colliders**, not a rigid tile grid
  — chosen for natural-looking terrain silhouettes (a valley should read as
  a genuine narrow pass between mountains, not a staircase of grid tiles).
  Authored in Tiled alongside the visual tileset.
- **Enemies are solid bodies, not passable.** A player can't walk through an
  enemy — same server-authoritative physics collision system as map terrain
  (`avian2d`). Bumping one transfers some momentum (normal dynamic-body
  physics, not a scripted "immovable object" special case), but an enemy's
  mass scales with its size, so a large/heavy enemy barely budges while a
  small one can be shoved more easily. This is what makes "defeat or lure
  away the enemy blocking this passage" an actual tactical choice rather
  than something a player can freely bulldoze through. Enemies also collide
  with each other (no stacking on the same spot when several chase one
  player) and with a downed player's body — still physically present even
  though out of combat, not incorporeal (see `MECHANICS.md`'s downed-state
  section).

## Multiplayer scope

- Co-op only. No PvP.
- Party size cap for v1: **2 players.** Build the sync/replication logic to
  generalize to more players rather than hardcoding for 2, so raising the cap
  later is a config change.

## Progression systems

Full loop is the v1 target — leveling, skills, and item forging/affixes are all
in scope. Recommended build sequence (since skills and items build on the same
underlying stat model):
1. Character leveling & core stats
2. Skills — acquired and upgradeable
3. Items — acquisition, forging/affixes, stat contribution

"In scope for v1" describes the target, not the order of implementation.

## Damage & faction system

A generic `DamageType` system with a resistance/weakness relationship between
types — not a hardcoded two-faction (pagan vs. christian) special case. This
keeps the mechanic extensible (new damage types can be added as content, not
engine changes) while still giving the theme real mechanical teeth: certain
enemy tiers deal types that map narratively onto "christian" magic (e.g.
holy/radiant), and player abilities map onto "pagan"/primal types with a
countering relationship to them.

## Enemy tiering

Roughly escalating tiers, doubling as the difficulty/content pacing structure.
Each tier shares underlying components (health, damage, loot table, AI pattern)
but escalates stats and introduces new abilities — this maps directly onto the
data-driven enemy template system in the `content` crate.

1. Converted pagans — villagers turned against their own people, ascending in
   power: **converted farmer, missionary**, converted housecarl. Converted
   farmer and missionary are the first two enemy types being built (M2).
2. Christians — the invading order's own personnel, ascending: sailor, monk,
   soldier, priest, knight, bishop, paladin. Priests/knights/bishops+
   introduce "christian" spell types.
3. Divine creatures — furious angels, archangels, and other divine beings.
   Specific roster not yet decided.
4. Final boss / collapse of the invading god's creation

## Objectives & dungeons

Hand-authored for v1: a fixed, designed set of objectives and dungeon layouts.
Procedural generation is an explicit stretch goal, not v1 scope — it's a
significant system on its own and shouldn't block getting the core loop working.

## Presentation: HUD, menus, splash, audio

- **HUD (v1):** health/resource bar, ability cooldowns, minimap, party member
  status (health/status for co-op teammates).
- **Menus (v1):** start/join game, inventory, settings/quit, skill tree, and
  forging UI. Menus never pause the simulation — see `CLAUDE.md`.
- **Splash screen:** simple static title/splash screen before the main menu.
  Not a priority for polish in v1 — functional placeholder is fine.
- **Audio (v1):** free/CC0 asset packs for SFX and music as placeholders —
  favor grounded, dark, aggressive tone over orchestral/whimsical fantasy
  packs, consistent with the Vision section. Revisit with custom or
  purpose-sourced audio later, once the core loop and content pipeline are
  proven — not worth investing in bespoke audio before then.

## Explicitly out of scope (v1)

- Cutscenes, branching dialogue, story choices
- Player-to-player trading (direct item drop between co-op players is still
  the only way to give someone an item person-to-person; NPC vendor buy/sell
  is a separate, in-scope system — see `MECHANICS.md`)
- PvP
- Persistent/shared world across parties
- Procedural dungeon generation

## Open questions (revisit later, not blocking)

- Is the game completable (a final boss = win state) or endless/infinite
  scaling? Doesn't block v1 systems work either way.
- Save architecture: server-authoritative vs. client-side, account/character
  identity model.
- Whether damage-type resistances are exposed as visible player-facing stats or
  stay mostly under the hood.
