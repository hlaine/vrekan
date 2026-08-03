# ROADMAP.md

Milestone sequence for v1. Living document — check items off, adjust order if
reality demands it, don't treat this as a rigid contract. Each milestone should
be small enough to review in a sitting; break a milestone into individual
Claude Code tasks as you start it, not all upfront.

## M0 — Workspace scaffold
- [x] Cargo workspace with all crates from `CLAUDE.md`, compiling but empty
- [x] CI (fmt check, clippy deny-warnings, test) passing on a trivial commit
- [x] `LICENSE`, `.gitignore`

## M1 — Single-player core loop (no networking yet)
- [x] Player entity: movement (WASD), basic melee attack
- [x] One enemy type, hardcoded, with health and a simple AI pattern
- [x] `apply_damage` and death handling in `game_core`, unit tested
- [x] Runs and is playable locally, single client, no server

## M2 — Data-driven content
- [x] RON schema + loader for enemy templates in `content`
- [x] Convert the M1 hardcoded enemy into a data-driven template
- [x] Add one more enemy type purely via a new `.ron` file, no engine changes
  — this is the test that the extensibility goal actually works

## M3 — Basic networking
- [x] Headless `server` binary, `MinimalPlugins`
- [x] bevy_replicon wired up: position replication for a moving entity
- [x] Two clients connect to one local server, see each other move

## M3.5 — Shared camera, leash, and map collision
- [x] Camera driven by party centroid (both clients), zoom scales with party
  spread — see `DESIGN.md`'s Camera & movement section
- [x] Hard leash: players can't exceed the camera's max spread, plus a
  lightweight visual indicator at the limit (not the real HUD, that's M8)
- [x] Map collision: freeform polygon colliders authored in Tiled, a first
  test map (`assets/maps/valley.tmx`) with a valley-style narrow pass
  between blocking mountain shapes. `avian2d` is the physics backend
  (server-only, `RigidBody::Dynamic` players against `RigidBody::Static`
  colliders — `Kinematic` bodies aren't affected by collisions in avian2d,
  so this isn't the naive choice it looks like). Client renders the map via
  `bevy_ecs_tiled`; the server parses the same file with the plain `tiled`
  crate instead, since `bevy_ecs_tiled` hard-depends on `bevy_render`
  regardless of features (confirmed empirically — not just a Cargo-feature
  toggle away) — see `CLAUDE.md`'s feature-unification note and
  `server/src/main.rs`'s `spawn_map_colliders` doc comment for the anchor
  convention keeping the two in sync. Both `tiled` and `avian2d` are new
  external dependencies per `CLAUDE.md`'s review rule — flagged and
  confirmed.
- [x] First real camera system — M1 only ever spawned a static `Camera2d` at
  the origin, so this isn't a replacement of prior follow-camera behavior

## M4 — Combat & damage type system, networked
- [ ] `DamageType` + resistance system from `game_core`, server-authoritative
  — see `MECHANICS.md` for the damage/crit formula shape
- [ ] Combat (melee attack from M1) works correctly across client/server
- [ ] Generic status-effect system (stun, bleed, buffs — stackable,
  duration-based, attachable to any attack via data)
- [ ] Enemy death; player death → "downed" state (not respawn) — see
  `MECHANICS.md` for the downed/revive/wipe rules
- [ ] Ally-revive: teammate walks to a downed player and presses an action
  button to revive them

## M5 — Progression: leveling & stats
- [ ] XP, character level, manual stat point allocation on level up — see
  `MECHANICS.md` for the formula shape
- [ ] XP penalty on individual death; full-party-wipe resets in-level
  progress to zero (level itself never drops) — see `MECHANICS.md`
- [ ] Resurrection-point checkpointing (auto-updates at dungeon entry /
  objectives); full-wipe auto-respawn there
- [ ] Server-authoritative, persisted per character (save format TBD)

## M6 — Skills
- [ ] Skill acquisition and upgrade, data-driven like enemies/items
- [ ] At least 2-3 skills with distinct mechanical behavior
- [ ] "Öd" resource (regen + combat/action-generated), power attacks that
  consume it — see `MECHANICS.md`

## M7 — Items & forging
- [ ] Item drops, pickup, equip
- [ ] Affix/forging system (the "custom system" from `DESIGN.md`)
- [ ] Loot tables tied to enemy tiers
- [ ] Vendor buy/sell economy, individual currency per player — see
  `MECHANICS.md`
- [ ] Enemy visual-variant data shape (shared base template + swappable
  sprite field) — see `MECHANICS.md`

## M8 — UI: HUD & menus
- [ ] egui HUD: health/resource (öd), cooldowns, minimap, party status,
  downed-state indicator
- [ ] Inventory, skill tree, forging UI, vendor/shop UI — built alongside
  M6/M7, not deferred wholesale to the end
- [ ] Level-up / stat-allocation panel (manual stat points — see
  `MECHANICS.md`)
- [ ] Player skin preset selection; equipped armor/helmet renders visually
  (full per-item outfit changes — see `MECHANICS.md`)

## M9 — Objectives & first dungeon content
- [ ] Hand-authored dungeon instance, entered explicitly from the overworld
- [ ] One full objective sequence, tier-1 enemies only
- [ ] Special-character dialog: one-way objective/flavor text, no branching
  — see `MECHANICS.md`
- [ ] Neutral (unkillable) character type
- [ ] Boss mechanics: phases, enrage timer, arena bounds — see
  `MECHANICS.md`; first boss encounter as part of this dungeon

## M10 — Polish pass
- [ ] Splash/title screen
- [ ] Placeholder audio (CC0 SFX + music) wired in
- [ ] Second co-op playtest end-to-end: two players, overworld + dungeon + loot

---

Beyond M10: broaden enemy tiers (M9's structure repeats), expand skill/item
pools, revisit the "completable vs. infinite" open question from `DESIGN.md`.
