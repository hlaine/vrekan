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
- [ ] Headless `server` binary, `MinimalPlugins`
- [ ] bevy_replicon wired up: position replication for a moving entity
- [ ] Two clients connect to one local server, see each other move

## M4 — Combat & damage type system, networked
- [ ] `DamageType` + resistance system from `game_core`, server-authoritative
- [ ] Combat (melee attack from M1) works correctly across client/server
- [ ] Enemy death, respawn on the overworld

## M5 — Progression: leveling & stats
- [ ] XP, character level, stat growth
- [ ] Server-authoritative, persisted per character (save format TBD)

## M6 — Skills
- [ ] Skill acquisition and upgrade, data-driven like enemies/items
- [ ] At least 2-3 skills with distinct mechanical behavior

## M7 — Items & forging
- [ ] Item drops, pickup, equip
- [ ] Affix/forging system (the "custom system" from `DESIGN.md`)
- [ ] Loot tables tied to enemy tiers

## M8 — UI: HUD & menus
- [ ] egui HUD: health/resource, cooldowns, minimap, party status
- [ ] Inventory, skill tree, forging UI — built alongside M6/M7, not deferred
  wholesale to the end

## M9 — Objectives & first dungeon content
- [ ] Hand-authored dungeon instance, entered explicitly from the overworld
- [ ] One full objective sequence, tier-1 enemies only

## M10 — Polish pass
- [ ] Splash/title screen
- [ ] Placeholder audio (CC0 SFX + music) wired in
- [ ] Second co-op playtest end-to-end: two players, overworld + dungeon + loot

---

Beyond M10: broaden enemy tiers (M9's structure repeats), expand skill/item
pools, revisit the "completable vs. infinite" open question from `DESIGN.md`.
