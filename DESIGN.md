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
click-to-move. Player roams an open overworld with continuously respawning
enemies, and enters explicit dungeon instances to complete objectives. The core
drive is grinding loot, levels, and upgrades to take on progressively stronger
enemies. No trading — the only way to give another player an item is to drop it
for them in co-op.

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

1. Human missionaries / militia
2. Priests, knights, bishops — introduce "christian" spell types
3. Divine creatures — furious angels, archangels
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
- Trading (only direct item drop between co-op players)
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
