# DECISIONS.md

Architecture decisions log — a running record of *why* the client/server
architecture looks the way it does, for the cases where the reasoning isn't
obvious from reading the code. Append new entries as decisions are made;
don't rewrite or delete old ones even if a later decision supersedes them —
add a new entry that references and supersedes the old one instead, so the
history of "we tried X, here's why we moved to Y" stays intact.

Each entry: **Context** (the problem/question), **Decision** (what we went
with), **Consequences** (the tradeoff, and what to watch for later). Keep
entries scoped to genuine architecture forks — crate/dependency choices,
client/server authority boundaries, wire format, or anything that was
non-obvious enough to require real investigation before deciding. Routine
implementation choices don't belong here.

---

## Movement authority: server-only, no client-side prediction (M3)

**Context:** M3 wired up basic position replication. The question of
whether the client should locally predict its own movement (and reconcile
against server corrections) versus simply rendering whatever `Position` the
server replicates back.

**Decision:** No client-side prediction. The client sends `MoveInput` and
renders whatever `game_core::Position` comes back from the server —
`game_core::Velocity` is not replicated at all, so the client has no local
copy of it to integrate against even for the locally-controlled player.

**Consequences:** Movement has one round-trip of input lag (visible mainly
on real network latency, negligible on localhost). This keeps the
client/server split simple for now — there's no reconciliation logic to get
subtly wrong. Revisit if/when real-network playtesting (not localhost)
makes the input lag feel bad; that would mean adding client-side prediction
plus a reconciliation step against server corrections, which is a genuine
architecture addition, not a tweak — treat it as its own milestone-sized
decision, not something to bolt on quietly.

---

## Hard leash is server-authoritative, not a client-side clamp (M3.5)

**Context:** The party leash (`DESIGN.md`'s Camera & movement section) needs
to cap how far apart two players can get. This could be enforced client-side
(cosmetic clamp on local rendering) or server-side (clamped on the
authoritative position).

**Decision:** Server-authoritative. `game_core::movement::leash_system`
clamps each player's distance from the party centroid to at most
`LEASH_DISTANCE / 2`, and only runs in the server's schedule. The client
only renders a cosmetic tint (`leash_indicator_system` in
`client/src/main.rs`) reflecting that state — it does not itself constrain
movement.

**Consequences:** A client-side-only leash would be both cheatable (a
modified client could ignore it) and a desync risk (client and server would
disagree about position without a correction mechanism, and M3 established
that the client has no reconciliation logic — see the entry above). Putting
it server-side keeps it consistent with every other piece of authoritative
state. `LEASH_DISTANCE` lives in `game_core::movement` specifically so the
client's camera-zoom calculation and the server's clamp read the exact same
constant and can't drift apart.

---

## Cargo feature unification across workspace builds (M3.5)

**Context:** `client` and `server` need different feature sets from shared
dependencies (`bevy_ecs_tiled`'s render features on the client only,
`avian2d`'s minimal feature set on the server only). The naive assumption
was that per-crate `features = [...]` declarations would isolate this.

**Decision:** They don't, in two distinct ways — both now load-bearing
assumptions anywhere a client/server dependency-feature split is attempted
again:

1. **Cross-workspace-member unification.** `cargo build --workspace` unifies
   a shared dependency's features across every member being built in that
   invocation — the server binary produced by a workspace-wide build
   includes the client's features too, not just its own. Documented in this
   file's "Dependencies & versioning" section of `CLAUDE.md`. **The server
   must always be built via `cargo build -p server --release` for
   deployment, never `--workspace`.**
2. **Same-crate unification via transitive optional deps.** This is the
   sharper trap: even a single crate's own dependency graph unifies
   features across every path that reaches a shared dependency. We verified
   this empirically — `server` declared `avian2d` directly with
   `default-features = false, features = ["2d", "f32", "parry-f32"]`, but
   `bevy_ecs_tiled`'s `"avian"` feature *also* pulls in `avian2d` (as its own
   optional dependency, declared with no `default-features = false`
   override on `bevy_ecs_tiled`'s end) — and that positive request for
   avian2d's defaults (including `debug-plugin` → `bevy/bevy_render`) wins
   over our explicit opt-out, regardless of `-p server`. Confirmed via
   `cargo tree -e features -p avian2d` showing the full default feature list
   despite our own declaration.

**Consequences:** `default-features = false` on a dependency is a *request*,
not a guarantee — it can be silently overridden by any other crate in the
graph (workspace member or transitive dependency) that asks for defaults on
the same crate. Before assuming a Cargo feature split isolates two binaries'
dependency footprints, verify with `cargo tree -e features -p <crate>`
against the exact build invocation that will actually ship, not just
`cargo build --workspace`. This is *why* the server ended up not using
`bevy_ecs_tiled` at all — see the next entry.

---

## `bevy_ecs_tiled` cannot be used on the headless server, in any configuration (M3.5)

**Context:** The original plan was for both `client` and `server` to depend
on `bevy_ecs_tiled` — client with rendering features, server with
`default-features = false` and just enough to parse maps and auto-spawn
avian2d colliders from object layers, keeping the server's `MinimalPlugins`
headless design (`CLAUDE.md`'s "Server is headless" rule) intact.

**Decision:** Ruled out entirely, not just its `"avian"` feature. Verified
via `cargo tree -i -p bevy_render` under a server-shaped dependency set: even
with `bevy_ecs_tiled`'s `default-features = false, features = []` (nothing
enabled beyond the bare crate), `bevy_render`/`bevy_core_pipeline`/
`bevy_sprite_render` still appeared. Root cause: `bevy_ecs_tiled` has a
**hard, non-optional dependency on `bevy_ecs_tilemap`**, and
`bevy_ecs_tilemap`'s own `Cargo.toml` requests
`bevy = { features = ["bevy_core_pipeline", "bevy_render", "bevy_asset", "bevy_sprite_render", "bevy_log"], default-features = false }`
**unconditionally** — not gated behind `bevy_ecs_tilemap`'s own `"render"`
Cargo feature at all. Toggling Cargo features cannot avoid this; it's
structural to the crate.

The server instead uses the plain `tiled` crate (a pure Rust TMX parser,
zero Bevy dependency — confirmed dependency-clean and functional in a real
headless `MinimalPlugins` + `avian2d` `PhysicsPlugins` app) to read the
map's "collision" object layer directly, and hand-spawns `avian2d` static
colliders from the parsed polygons. See `server/src/main.rs`'s
`spawn_map_colliders`.

**Consequences:** Client (`bevy_ecs_tiled`, full rendering) and server
(plain `tiled`, geometry only) now use two different code paths to read the
same `.tmx` file, which means their coordinate conventions could in
principle drift apart — see the next entry for how that's kept in sync. If
a future dependency looks like it needs `bevy_ecs_tilemap` on the server for
any reason, re-verify with `cargo tree -i -p bevy_render` first; this
finding was surprising enough that it's worth not assuming it's been fixed
in a later version without re-checking.

---

## Tiled coordinate convention: `TilemapAnchor::TopLeft` (M3.5)

**Context:** With the client and server parsing the same `.tmx` file
through two different libraries (previous entry), their conversion from
Tiled's pixel coordinates (origin top-left, Y increases downward) to
Bevy/`game_core` world coordinates (Y increases upward) needs to produce
*identical* results, or collision geometry silently drifts from what's
rendered.

**Decision:** Configure the client's map spawn with `TilemapAnchor::TopLeft`
(from `bevy_ecs_tilemap`, re-exported via `bevy_ecs_tiled::prelude`). Traced
`bevy_ecs_tilemap`'s anchor-offset math (`TilemapAnchor::as_offset`) by hand
for this case: with `TopLeft`, the map's outer top-left corner lands exactly
at world `(0, 0)`, and the half-tile offset baked into the "anchor is the
center of the bottom-left tile" default convention cancels out exactly.
This makes the conversion a single, exact, map-size-independent formula:

```
world_x = tiled_x
world_y = -tiled_y
```

The server's hand-rolled parser (previous entry) uses this exact formula
when converting `tiled::Object` polygon points into `avian2d::math::Vector`
world points. No other anchor option produces a fixed, size-independent
formula — all the others (`Center`, `BottomLeft`, etc.) bake in a
map-width/height-dependent constant, which would need to be recomputed
identically on both sides.

**Consequences:** Any new map must be spawned client-side with
`TilemapAnchor::TopLeft` — changing that anchor on the client without
updating the server's formula (or vice versa) will silently misalign
collision from rendering. This is called out in both
`client/src/main.rs`'s `MAP_PATH` comment and `server/src/main.rs`'s
`spawn_map_colliders` doc comment.

---

## avian2d player bodies: `RigidBody::Dynamic`, not `Kinematic` (M3.5)

**Context:** The obvious-looking choice for a player-controlled physics body
is `RigidBody::Kinematic` — "kinematic" reads as "moved by code, not by the
physics solver," which sounds exactly like what a directly-input-driven
player needs.

**Decision:** Use `RigidBody::Dynamic` instead, with `LinearVelocity` set
directly from input each tick (matching what a naive `Kinematic` setup would
have done), plus `LockedAxes::ROTATION_LOCKED` and `Friction::ZERO` so a
directly-velocity-driven circle doesn't spin or stick on glancing contact.

**Consequences — and why this was worth writing down:** avian2d's own doc
comment on `RigidBody::Kinematic` states it plainly ("not affected by any
external forces or collisions... the engine doesn't modify the values of a
kinematic body's components"), but this is easy to skim past because
"kinematic" *sounds* like the right word for a player character. We built
the `Kinematic` version first, and it compiled and ran with no errors or
warnings — the bug was entirely behavioral. It only surfaced when we
actually drove a player into a mountain collider in a live two-client
playtest and watched it slide straight through; a build/test/clippy pass
alone would never have caught it, since there is no compile-time signal
that a `Kinematic` body ignores collision. If a future change reintroduces
a `Kinematic` body anywhere collision with it is expected to matter, don't
trust that it compiles cleanly as evidence it works — playtest the specific
collision.

---

## Moving enemies server-side forces combat server-side too (M4)

**Context:** Enemies were entirely client-local through M3.5 (spawned and
simulated independently by each client, per M1/M2). The M4 goal was just
"enemies exist on the server, replicated" — combat networking looked like a
separable, later step.

**Decision:** It isn't separable. The moment `Health` becomes a replicated
component, the client's own local `attack_system` call (resolving the
player's attack against its local copy of an enemy's `Health`) starts
fighting the replication system — any damage the client applies locally
gets overwritten by the server's next update, since the client is no longer
the source of truth for that component. So moving enemies server-side and
moving combat resolution server-side had to land together: the client sends
`AttackInput` (a new client message, no payload — the server resolves
range/cooldown/target/damage from its own authoritative state, same
principle as `MoveInput` never trusting a client-supplied speed), and
`game_core::combat::attack_system`/`death_system`/`tick_attack_timers` now
only run in the server's schedule.

**Consequences:** Any future component that moves from "client-simulated"
to "server-authoritative + replicated" should be checked for this same
trap — if the client mutates that component's data anywhere (not just reads
it), replicating it will silently make those client-side mutations
pointless/flickery rather than erroring. Treat "replicate X" and "stop
letting the client mutate X" as one atomic change, not two.

---

## Replicating a content-template *key*, not template data (M4)

**Context:** Enemies are now spawned server-side from `content::EnemyTemplate`
RON files (color, size, stats). The client needs to know which appearance
(color/size) to render for a given replicated enemy entity, but the full
template isn't (and shouldn't be) sent over the wire — `content` stays a
local-filesystem-loaded crate on both sides, not a network-transmitted one.

**Decision:** Added `game_core::EnemyKind(pub String)` — just the template's
key (e.g. `"converted_farmer"`, the `.ron` filename stem) — as a replicated
component. The client keeps loading all `.ron` files locally at startup
(same as before M4, just for appearance lookup now instead of simulation)
into an `EnemyTemplates` resource, and a reactive system
(`init_replicated_enemies`, keyed off `Added<EnemyKind>`) looks up the
matching template by that key to pick `Sprite` color/size.

**Consequences:** This is the general pattern for any future content-backed
entity that needs to be spawned server-side and rendered client-side (items
dropped in the world, spell-effect visuals, etc. — see M6/M7): replicate the
content key, not the content data, and let each side resolve the rest from
its own already-loaded copy of the same `.ron` files. Keeps `protocol`'s
wire format small and stable even as template schemas grow richer fields
over time (a new `EnemyTemplate` field never needs a protocol change).

---

## `AttackInput` is reliable-unordered, not unreliable like `MoveInput` (M4)

**Context:** `protocol` needed a new client→server message for attack
input. `MoveInput` (continuous per-frame state) uses `Channel::Unreliable` —
dropping one frame's movement input doesn't matter, the next frame's arrives
almost immediately and corrects it.

**Decision:** `AttackInput` uses `Channel::Unordered` (bevy_replicon's
reliable-but-unordered channel) instead. It's a discrete, one-shot action
tied to a keypress, not continuous state — losing one has no "next frame"
to self-correct with, so it would read as an attack that silently didn't
register. Ordering relative to other `AttackInput` messages doesn't matter
(each just resolves independently against whatever's in range at the time),
so `Ordered` would be unnecessary overhead.

**Consequences:** When adding future discrete player actions (dodge, skill
activation, item use, etc. — M6/M7), default to this same reasoning: is it
continuous state that self-corrects next frame (→ `Unreliable`), or a
one-shot action where a drop is a visible bug (→ `Unordered`, or `Ordered`
only if relative sequencing between messages of that type actually
matters)?

---

## Supersedes the above: `AttackInput`'s reliable channel doesn't reliably deliver (M4)

**Context:** The previous entry's reasoning was sound in theory but wrong in
practice. Live playtesting after wiring up `DamageType`/`Resistances`
surfaced a concrete, reproducible bug: with `AttackInput` on
`Channel::Unordered` (reliable), the client kept sending the message every
time the attack key was pressed (confirmed via client-side logging — it
logged "sending" on every press, no exceptions), but the server stopped
receiving them entirely after the first ~8 messages of a session — not
delayed, not occasional, *permanently* silent for the rest of that
connection's lifetime, confirmed by watching both sides' logs over several
minutes and dozens of further presses.

**Decision:** Switched `AttackInput` to `Channel::Unreliable` (matching
`MoveInput`). Verified the fix directly: 20/20 presses delivered
client-to-server in a controlled back-to-back test on `Unreliable`, versus
the reliable channel's silent cutoff. Root cause not fully isolated (a
plausible but unconfirmed theory: `AttackInput` is a zero-field unit struct,
so every message serializes to identical bytes — if this `bevy_replicon`/
`renet` version's reliable-channel path does anything content-addressed
rather than purely sequence-number-based for dedup/ack tracking, identical
payloads could be the trigger), but not worth the time to root-cause further
given a working alternative existed. The `protocol` crate's `AttackInput`
doc comment carries this same explanation.

**Consequences:** Don't trust `Channel::Unordered`/`Channel::Ordered`
(reliable channels) as actually reliable in this dependency stack
(`bevy_replicon` 0.41.1 + `bevy_replicon_renet` 0.17.0 + `renet` 2.0.0)
without live-testing sustained delivery first — a handful of manual presses
during development is not enough to catch this, since the first several
messages of any session go through fine. If a future feature genuinely
needs guaranteed delivery (not just "drops are tolerable"), that reliable
channel claim needs verifying with a real burst test before depending on it,
and may need a version bump or a different transport investigated as its
own deliberate task.

---

## Player death currently disconnects the client (found via testing, not new) (M4)

**Context:** While live-testing the `DamageType`/`Resistances` work,
repeated manual combat attempts kept producing confusing, seemingly-broken
results — inputs appeared to stop registering entirely partway through
testing sessions. Root cause: `game_core::combat::death_system` despawns
*any* entity at zero health, with no special case for players. A player's
entity is the same entity bevy_replicon associates with that client's
connection (see `on_client_connected`'s doc comment in `server/src/main.rs`)
— despawning it while the client is still connected tears down the
connection itself (confirmed in logs: `renetcode::client: Failed to update
client: disconnected: connection terminated by server`), rather than
leaving the player in any recoverable state.

**Decision:** Not fixing this now — `ROADMAP.md`'s M4 already has "Player
death → downed state (not respawn)" as its own explicit, not-yet-done item,
and this *is* that gap, not a new regression introduced by the
`DamageType`/`Resistances` work. Recording it here because it was
non-obvious enough to burn significant debugging time (the symptom looks
identical to "the client stopped sending input" or "the UI automation lost
focus," not "the player character died"), and because enemies now
auto-attack via `ai_system` the moment they're in range, so a player can die
this way well before anyone builds a HUD to show it happening.

**Consequences:** Until the downed-state milestone lands, treat any
"combat/input mysteriously stopped working" symptom during manual testing
as a first suspect for this — check the client log for a
`renetcode`/`bevy_replicon` disconnect message before assuming the bug is
elsewhere. Also worth remembering when building the downed-state system:
`death_system` will need to stop treating players and enemies identically,
likely via a marker/branch that routes a zero-health `Player` entity to a
downed-state transition instead of `commands.entity(entity).despawn()`.

---

## `DamageType` is a data-keyed string, not a fixed Rust enum (M4)

**Context:** `DESIGN.md`'s Damage & faction system and `MECHANICS.md` both
frame `DamageType` as something new content should be able to add "as
content, not engine changes" — the same extensibility principle already
applied to enemies (`content::EnemyTemplate`) and enemy appearance
(`EnemyKind`). The obvious default in Rust is a `enum DamageType { Primal,
Holy, ... }`, since enums are the idiomatic way to model a closed set of
kinds — but a closed set is exactly what this isn't supposed to be: every
new tier of enemy (per `DESIGN.md`'s Enemy tiering — sailors, monks,
priests, and beyond eventually introducing "christian" holy/radiant types)
is expected to bring its own damage types, and a Rust enum would mean an
engine-code change (a new variant, plus every `match` over it) for each one.

**Decision:** `DamageType(pub String)` — a plain newtype wrapping an owned
`String`, deriving `Hash`/`Eq` so it works as a `HashMap` key in
`Resistances`. Content authors write damage types as plain strings directly
in `.ron` files (e.g. `melee_damage_type: "primal"`, `resistances:
{"holy": 0.2}`); `content::spawn_enemy` wraps the parsed strings into
`DamageType` only at spawn time when building the `MeleeAttack` and
`Resistances` components — `EnemyTemplate` itself stores plain
`String`/`HashMap<String, f32>` fields, not `DamageType` directly, so RON
parsing doesn't need `DamageType` to implement any special map-key
deserialization behavior.

**Consequences:** There's no compile-time exhaustiveness checking — a typo
in a `.ron` file's damage type string (`"primmal"` instead of `"primal"`)
won't be caught by the compiler or by `content`'s existing malformed-file
tests; it'll just silently resolve to 0% resistance at runtime (no matching
`Resistances` key), which could read as "this enemy just doesn't resist
that type" rather than "there's a typo somewhere." If mismatched damage-type
strings turn out to be a recurring content-authoring mistake once more
damage types exist (M4's later tiers), worth revisiting: e.g. a content
crate test that cross-checks every enemy template's damage-type strings and
resistance keys against a canonical list, without giving up the
data-driven-ness of the type itself.

---

## Downed state is a marker component, not a separate life-cycle state machine (M4)

**Context:** M4's remaining "player death → downed state" item needed
`death_system` to stop despawning players outright (see this file's "Player
death currently disconnects the client" entry for why that was breaking
connections). The options were something heavier — an explicit player
life-cycle enum (`Alive`/`Downed`/`Dead`) driving a state machine — versus
just adding a marker component alongside the existing `Health`.

**Decision:** A plain unit-struct `Downed` component (in `game_core::player`,
next to `Player`), inserted by `death_system` on a `Player` entity that
reaches zero health, never removed by anything yet (ally-revive, the very
next roadmap item, will be what removes it). `attack_system`'s attacker and
target queries, and `ai_system`'s player-targeting query, all gained a
`Without<Downed>` filter — a downed entity is simply invisible to combat
resolution and enemy AI, rather than being specially handled inside them.
Replicated (`protocol::NetworkPlugin`) so the client can render it. On the
server, `apply_move_input` also filters `Without<Downed>` and a new
`on_player_downed` observer (mirroring `on_client_connected`'s existing
observer pattern) zeroes the entity's `LinearVelocity` at the moment of
transition, so a player who dies mid-slide doesn't keep coasting.

**Consequences:** This keeps "downed" as pure ECS composition — any system
that shouldn't apply to a downed player just adds one query filter, instead
of a central state machine every such system would need to consult. The
tradeoff is there's no single place that enumerates "what downed means";
that meaning is spread across each filtered query. This has stayed cheap
enough (four call sites total) to be worth it over a state-machine
abstraction for a single boolean-ish state. Revisit if a third player state
shows up beyond alive/downed (e.g. M5's full-wipe handling needs its own
state) and the marker-component-per-system-filter approach starts feeling
scattered rather than simple.

---

## Ally-revive: no banked progress, and client sprite color is fully recomputed, not overlaid (M4)

**Context:** Ally-revive needed a hold-to-channel interaction
(`MECHANICS.md`: "holds/presses an action button... in place"), with exact
range/timing explicitly called out there as "a reasonable starting
assumption, not a settled decision." Two implementation choices came up
that were non-obvious enough to record.

**Decision 1 — progress isn't banked across separate attempts.**
`game_core::revive::ReviveProgress` on the downed entity is dropped
entirely (not paused) the instant no `Reviving` ally is within
`REVIVE_RANGE` — walking away or letting go at 2.9s of a 3s channel means
starting over from zero, not resuming from 2.9s later. Simpler to reason
about and implement (one `bool` check per tick, no separate "paused since"
bookkeeping), and defensible as a starting assumption exactly because
`MECHANICS.md` flagged this as unsettled — revisit if playtesting makes
losing near-complete progress to a brief interruption feel bad.

**Decision 2 — `client`'s per-frame sprite-color systems were merged, not
extended.** Adding revive means `Downed` gets removed again, and the
existing `leash_indicator_system` (local player only) plus
`downed_indicator_system` (both, added for the downed-state milestone item)
only ever *overlaid* a tint on top of whatever color was already set —
neither recomputed a `RemotePlayer`'s base color from scratch each frame.
Without a third change, a revived `RemotePlayer` would stay grey forever:
nothing runs every frame to put their blue color back once `Downed` is
gone. Rather than patch this with a `RemovedComponents<Downed>` special
case, both systems were replaced with one `player_appearance_system` that
recomputes every party member's color from current state every frame
(`Downed` → grey, else local-near-leash-limit → warning red, else
local/remote → their base color). No system now assumes "whatever color is
already there is correct except for this one thing I might change."

**Consequences:** Any future transient visual state on a player sprite
(e.g. a stun/status-effect tint in the upcoming status-effect system)
should extend `player_appearance_system`'s single `if`/`else` chain rather
than adding another standalone tint-overlay system — that's exactly the
shape of bug this decision closed off.

---

## Found via testing: enemies never attacked anyone with two clients connected (M4)

**Context:** While manually playtesting ally-revive with two clients
connected, enemies didn't attack at all — not "attacked the wrong player,"
not "attacked intermittently," just never engaged, for the whole session.
Root cause: `game_core::enemy::ai_system` picked its target with
`player_query.single()`, which only returns `Ok` when *exactly one* entity
matches the query. With two clients connected there are two `Player`
entities, so `.single()` returned `Err` every tick and the function
returned immediately before touching any enemy — regardless of either
player's position, aggro range, or anything else. This had been true since
`ai_system` was first written (single-player-shaped code that was never
updated when networked multiplayer landed), not something introduced by
recent work.

This also means M4's "combat works correctly across client/server" item
(checked off after "a live two-client playtest") was only actually
confirmed for player-initiated attacks (`attack_system` off an `AttackInput`
message, unaffected by this bug) and one enemy-kills-player instance that,
in hindsight, must have happened with only one client connected at the
time — not for enemy-initiated attacks with a real two-player party, which
is the scenario this bug fully broke.

**Decision:** Changed `ai_system` to pick each enemy's target independently
via `player_query.iter().min_by(...)` on distance (the same nearest-target
pattern already used in `combat::attack_system`), instead of assuming
there's exactly one player. Zero matching players (all disconnected, or the
sole player downed) now falls out naturally as "no target found" rather
than as a distinguished `.single()` error case.

**Consequences:** Different enemies can now chase different players
simultaneously, which is the actually-intended co-op behavior, not an
extension beyond it. Added `enemy_targets_nearest_of_two_players` and
`enemy_idles_when_no_players_are_connected` as regression tests in
`game_core::enemy` — the former reproduces the exact broken scenario. Any
future system that queries for "the player" via `.single()` (there's
nothing else doing this today, but worth checking before adding one) should
be treated as suspect the moment the system is meant to work with a full
co-op party, not just a lone developer testing solo.

---

## Enemy collision migrates enemies onto avian2d entirely; facing uses gizmos, not sprites (M4)

**Context:** A design discussion on movement/combat feel (see `ROADMAP.md`'s
M4 additions) settled on: enemies should be solid, normal dynamic-body
physics (not a scripted "immovable object"), with mass scaled by size;
enemies collide with each other too; downed players stay solid; and both
players and enemies get a movement-derived facing direction, visualized
with an arrow. Before implementing, three `avian2d`/`bevy_gizmos` API facts
were verified against the actual 0.7.0/0.19.0 source rather than assumed,
since getting any of them wrong would have meant reaching for unnecessary
extra components:

1. Mass is auto-computed from a `Collider`'s shape area × `ColliderDensity`
   (default `1.0`), unless `NoAutoMass` is added. Sizing each enemy's
   `Collider::circle` from the same `template.size` field already used for
   its sprite therefore gives bigger enemies proportionally more mass with
   zero new content fields or explicit `Mass`/`ColliderDensity` components.
2. `CollisionLayers::default()` is memberships=ALL, filters=ALL — dynamic
   bodies collide with everything by default, so giving enemies the same
   bare `RigidBody::Dynamic` + `Collider` setup players already have was
   enough to get enemy-vs-player, enemy-vs-terrain, and enemy-vs-enemy
   collision all at once, no `CollisionLayers` configuration needed.
3. `bevy_gizmos` ships as part of `DefaultPlugins` (already used by
   `client`) and is immediate-mode: a system that calls `gizmos.arrow_2d`
   every frame from current `Position`/`Facing` needs no spawned indicator
   entity and has no lifecycle to manage — it can't go stale the way
   ally-revive's sprite-tint overlay did, since there's no persisted state
   to leave behind.

**Decision:** Enemies fully migrated onto `avian2d` (`RigidBody::Dynamic`,
`Collider::circle(template.size / 2.0)`, `LockedAxes::ROTATION_LOCKED`,
`Friction::ZERO`, mirroring `on_client_connected`'s player setup exactly),
attached in `server::spawn_enemies` rather than `content::spawn_enemy` —
keeping `content` free of an `avian2d` dependency it would otherwise gain
for every crate that depends on it (including `client`, which never spawns
enemies itself) for zero benefit, the same feature-unification concern
`CLAUDE.md` already flags for `bevy_ecs_tiled`. `ai_system` still decides a
plain `game_core::Velocity` per enemy; a new `sync_enemy_velocity_to_physics`
copies that into `avian2d`'s `LinearVelocity` each tick, the same role
`apply_move_input` plays for players. `game_core::movement::movement_system`
(the old plain integrator) is no longer called by `server` — enemies were
its only user — but stays in `game_core`, still unit-tested, since it's
valid engine-agnostic logic that just happens to be unused by this binary
right now.

`Facing` (`game_core::movement`) is a plain `{x, y}` normalized-direction
component updated by whichever system already decides movement for that
entity (`apply_move_input` for players from `MoveInput`, `ai_system` for
enemies from its own chase-velocity decision) — not a new movement path.
Visualized via one client system, `facing_indicator_system`, matching *any*
entity with `(Position, Facing)` with a `gizmos.arrow_2d` call — no
`Or<With<Player>, With<RemotePlayer>, With<Enemy>>` filter needed, since
gizmos don't care what else is on the entity.

**Consequences:** Enemy movement now has the same one-tick physics latency
as players (a velocity decision this tick is only visible after the next
physics step), which is consistent, not a new source of desync. Current
tier-1 enemies (`converted_farmer` size 28, `missionary` size 30) come out
slightly *lighter* than the player (collider radius 16) by this formula —
mass ∝ radius² — so expect mild, not strong, pushback from today's weakest
enemies; this is untuned and expected to improve as later, larger enemy
tiers are added, not a bug to fix now. If a `Mass`/`ColliderDensity`
override per template ever proves necessary (e.g. a big enemy that still
feels too light), that's a `content::EnemyTemplate` schema addition to make
then, not now.

---

## Status effects: buffs read fresh (never cached), and a real Bevy query-aliasing bug found while building it (M4)

**Context:** M4's last item, the generic status-effect system, needed to
support all three of MECHANICS.md's named examples (bleed/DoT, stun/CC,
"fury"/buff) through one data-driven mechanism, including a case
`MECHANICS.md`'s own wording implied but the existing attack model didn't
have a slot for: a buff like "fury" applies to the *attacker*, not the
attack's target, unlike bleed/stun which afflict whoever got hit. Added
`EffectDefinition::applies_to: EffectTarget` (`Target` | `Attacker`) to
`attack_system` to route each effect to the right side of a landed hit.

**Decision 1 — buffs are never applied by mutating `CombatStats` (or any
base stat component).** `ActiveEffects::stat_bonus(stat)` sums active
`StatModifier` magnitudes and is added to the base value fresh, at the
point of use (`attack_system` builds an ephemeral "effective" `CombatStats`
just for that call). Nothing ever writes a buffed value back into the
entity's real `CombatStats`, so there's no "restore the original value when
the buff expires" step to forget — this is deliberately the mirror image of
ally-revive's original sprite-tint-overlay bug (see that entry above):
compute fresh every time, don't cache a modified value and hope to
invalidate it correctly later.

**Decision 2 (found via `cargo test`, not live play) — `ActiveEffects`
needed its own dedicated query, fetched via `get_many_mut`, not one query
per side.** The natural first draft put `&mut ActiveEffects` in both
`attack_system`'s attacker query and its target `healths` query. This
panics at runtime (Bevy error B0001, "queries with conflicting mutable
access to the same component"): an attacker and its target are always
different entities (the nearest-target search excludes the attacker
itself), but Bevy's query-conflict check is purely structural — it sees
that *some* entity could in principle match both queries (any combatant
has both `MeleeAttack` and `Health`) and refuses to compile/run the system
regardless of the runtime guarantee. Fixed by pulling `ActiveEffects` into
its own unfiltered `Query<&mut ActiveEffects>` and fetching both sides at
once with `.get_many_mut([attacker, target])`, which is exactly the API
Bevy provides for "I need mutable access to two entities from one query
and I can prove they differ."

**Consequences:** Any future system needing mutable access to the *same*
component type on two different logical roles (attacker/target,
mover/pusher, etc.) should reach for `get_many_mut` on one shared query
from the start, not two separately-typed queries — the conflict only
surfaces at compile/run time, not from reading the code, so this is easy to
reintroduce by accident. Enemies don't get a `Stunned` sprite tint yet
(players only, via `player_appearance_system`) — an enemy's base color
varies per content template rather than being a fixed constant, so
reverting it cleanly after a stun ends needs storing each enemy's base
color somewhere first; deferred as a real gap, not an oversight, since the
mechanical effect (a stunned enemy visibly stops moving/attacking) is
already observable without it.

---

## Effects gained a per-hit `chance`, found necessary via live testing (M4)

**Context:** Playtesting `converted_farmer`'s daze (a guaranteed,
`RefreshDuration` `Stun`, 1.5s duration) surfaced a real permastun: the
farmer's own attack cooldown (1.0s) is shorter than the stun's duration,
so every landed hit refreshed the clock before it could ever expire —
the player was locked out of acting indefinitely. This is a general hazard
of the status-effect system as originally built, not specific to this one
enemy: any guaranteed CC effect whose duration outlasts its inflictor's
attack cooldown can do this.

**Decision:** Added `EffectDefinition::chance: f32` (fraction of landed
hits that actually apply the effect; `1.0` = the old always-apply
behavior), rolled independently per effect in `attack_system` using the
same `rng`/`random_bool` pattern `resolve_damage` already uses for crits.
`content::EffectTemplate` mirrors it with `#[serde(default = "full_chance")]`
so existing/future content that wants "always applies" doesn't need to
name the field. Fixed `converted_farmer.ron` concretely: daze's duration
dropped to 0.6s (comfortably under the farmer's 1.0s cooldown) and
`chance: 0.25` — even a lucky streak of procs can't permalock, since the
stun always expires before the next possible hit lands.

**Consequences:** `chance` is a general per-effect field, not special-cased
to `Stun` — a `DamageOverTime` or `StatModifier` effect can use it too
(e.g. an "on-hit poison chance"), consistent with the rest of the system
being data, not hardcoded per-effect-kind logic. Any *future* guaranteed,
refresh-duration CC effect should still be checked against its inflictor's
own attack cooldown before shipping — `chance` makes permalock unlikely,
not structurally impossible, if someone sets `chance: 1.0` again with a
too-long duration. `bleed`/`fury` are unaffected (`chance: 1.0`, and
neither is a CC effect that can lock the target out of acting either way).

---

## Found via testing: players could damage each other (M4)

**Context:** `DESIGN.md`'s Multiplayer scope already says "Co-op only. No
PvP" — a settled rule, not a new decision. But `attack_system`'s
nearest-target search only ever checked `With<Health>` + `Without<Downed>`,
with no concept of faction, so a player standing near a teammate (which
co-op play makes routine — see the leash mechanic) could end up as another
player's "nearest target in range" and take damage, found via live
testing.

**Decision:** `AttackTargets` and `Attackers` both gained `Has<Player>` as
query *data* (not a filter — whether a `Player` target is excluded depends
on whether the attacker is *also* a `Player`, which a static query filter
can't express). `attack_system`'s nearest-target search adds one line:
skip any candidate where both attacker and target are players. Enemies
remain attackable by players either way, and this doesn't touch
enemy-vs-player targeting (`ai_system` only ever targets players, never
other enemies, so enemy-vs-enemy friendly fire was never a live pathway to
begin with).

**Consequences:** If enemy-vs-enemy attacks are ever introduced (not
planned, but nothing currently prevents it structurally), this same
`Has<Player>` check only prevents *player*-vs-player damage — it would not
by itself stop two enemies from hurting each other, which may or may not
be desired at that point. Added `attack_system_prevents_player_on_player_damage`
and `attack_system_still_hits_an_enemy_past_a_nearer_teammate` to
`game_core::combat`'s tests — the latter specifically proves the fix
skips past an ineligible nearer teammate to the next valid target rather
than failing closed on the whole search.

---

## Character/game identity model, and how auth rides the existing connection handshake (M5)

**Context:** M5's persistence bullet, and `DESIGN.md`'s open "save
architecture... account/character identity model" question, needed an
actual answer before any save code could be written — there was no
login/account system at all (a client got a random ephemeral ID per
connection). Confirmed with the user first, since this is exactly the kind
of foundational, hard-to-reverse decision later milestones (M6/M7) will
build on:

- **One server process is one game.** No multi-tenant rewrite; a "game ID"
  is a server-side save-directory namespace (`saves/<game_id>/`, from a CLI
  arg, default `"default"`), never typed by a player — direct IP:port
  connection already tells a client "which game" it's joining, since
  there's no lobby/matchmaking service to make that ambiguous. A real
  shareable-discovery-ID system would need an actual lobby layer, out of
  scope here.
- **`ServerAuthentication::Unsecure` stays, deliberately, for a LAN-style
  host-and-share model** — not a public service. Verified in the actual
  `renetcode`/`renet` source before accepting this: `Secure { private_key }`
  already exists alongside `Unsecure` in the same crate, using a signed
  `ConnectToken` instead of the plain handshake, so upgrading later is a
  contained change, not a rewrite, if the hosting model ever changes.
- **A character is scoped to one game**, persisting across reconnects to
  *that* game, not a roster portable across different games — matches
  `DESIGN.md`'s existing "only the player's character persists across
  sessions" framing more closely than a portable-roster model would.
- **Menu UI is still M8's job.** The client prompts for both passwords via
  a blocking terminal `stdin` read at connect time (`client::prompt`) —
  the closest non-UI stand-in for "enter it every time," not a permanent
  design.

**Decision — auth rides netcode's existing connection handshake, not a new
gameplay message.** `protocol::ConnectAuth { game_password, character_id,
character_password }` is RON-encoded (reusing `serde`/`ron`, not a
hand-rolled byte layout — this is a small, rarely-(dis)assembled struct,
not a hot-path wire message) into netcode's already-existing 256-byte
connection-time `user_data` field (`NETCODE_USER_DATA_BYTES`, verified in
`renetcode` source), which the client already had wired to `None`. The
server reads it back via `NetcodeServerTransport::user_data(client_id)`
(verified this method exists) inside the existing `on_client_connected`
observer, before inserting any gameplay component — a rejection just calls
`RenetServer::disconnect(client_id)` (verified exists) and returns early;
`bevy_replicon_renet` despawns the `ConnectedClient`-only entity itself
once the disconnect is processed (confirmed by reading its actual
`ClientDisconnected` handler), so there's nothing to clean up on a
rejected connection.

**Decision — the disconnect-time save hooks `On<Remove, ConnectedClient>`,
not the same `RenetServerEvent` bevy_replicon_renet itself reacts to.**
`bevy_replicon_renet`'s own disconnect handling and this save logic both
need to react to a client disconnecting, but they need different
*ordering* guarantees: `bevy_replicon_renet` despawns the entity;
`server::on_character_disconnected` needs to read that same entity's
`CharacterId`/`Level`/`Stats` *before* it's gone. Reacting to the same
`RenetServerEvent(ServerEvent::ClientDisconnected)` trigger `bevy_replicon_renet`
itself uses would mean depending on two independent observers for the same
custom event firing in a specific relative order — not a guarantee Bevy
actually makes. `On<Remove, ConnectedClient>` sidesteps the race entirely:
Bevy's own component-removal hooks are documented to run *before* the
component (and the rest of the entity, mid-despawn) is actually gone, so
reading sibling components from within that hook is always safe,
regardless of how the despawn was triggered or which plugin triggered it.

**Found via `cargo test`, not live play — RON's `u128` support needs a
non-default feature flag.** `ConnectAuth::character_id` is a `u128`;
`ron::to_string` failed at runtime with "u128 is not supported" despite
`ron`'s serializer having `serialize_u128` in its source. Root cause:
that method (and `serialize_i128`) is gated behind `#[cfg(feature =
"integer128")]`, which is not in `ron`'s default feature set. Fixed by
adding `features = ["integer128"]` to the workspace `ron` dependency — a
dependency-config change (flagged per `CLAUDE.md`, though it's activating
an existing feature of an already-approved, exact-pinned dependency, not a
version bump or a new crate, and the feature itself pulls in no additional
dependencies).

**Consequences:** Character saves live at
`saves/<game_id>/characters/<character_id>.ron`, containing the character's
password in plaintext alongside `Level`/`Stats`/`UnspentStatPoints` —
acceptable given the confirmed "not real security, LAN-style hosting"
scope, but worth revisiting together with the `Unsecure`→`Secure` upgrade
if the hosting model ever changes. A corrupt *game* save panics at Startup
(before anyone's connected, matching the existing "malformed content fails
loudly" convention); a corrupt *character* save or a failed disconnect-time
write only logs an error and rejects/skips that one character, never
crashing the server for everyone else already connected — deliberately not
the same panic-on-malformed-content treatment, since per-character I/O
failure is a recoverable, isolated case, not a startup-time invariant.

---

## XP-on-death penalty hooks `Added<Downed>`; found a `run_system_once` testing gotcha (M5)

**Context:** M5's last item, the XP-on-death penalty, needed to fire
exactly once per downing (not every tick a player stays downed) and needed
a separate full-party-wipe check that resets everyone's in-level XP to
zero rather than stacking with the individual penalty — see
`MECHANICS.md`'s Progression section.

**Decision:** `apply_death_xp_penalty` queries `Added<Downed>` rather than
having `death_system` (in `combat.rs`) call into progression code directly
— `death_system` stays entirely ignorant of XP/leveling, and the "exactly
once" guarantee comes for free from Bevy's own change detection instead of
manual bookkeeping. `reset_xp_on_full_wipe` runs unconditionally every
tick and just re-zeroes an already-zero value once the wipe condition
holds, rather than trying to detect the precise transition tick — simpler,
and reapplying zero is a harmless no-op.

**Found while writing the test, not live play — `run_system_once` doesn't
carry `Added<T>` state between calls.** The first version of
`death_penalty_only_applies_once_at_the_moment_of_downing` called
`world.run_system_once(apply_death_xp_penalty)` twice and expected the
second call to see `Added<Downed>` as false. It didn't: each
`run_system_once` builds a fresh, stateless system with no memory of a
prior invocation, so change-detection "last run" tracking never
accumulates between separate calls — the second call still saw the
component as newly-added and reapplied the penalty (xp landed on 64.0, not
80.0). Fixed by using a persistent `Schedule` (`schedule.add_systems(...)`
+ two `schedule.run(&mut world)` calls on the same `World`), which does
preserve each system's last-run tick across calls, matching how the system
actually behaves across real ticks in the server's schedule.

**Consequences:** Any future `game_core` test asserting "a system's
`Added`/`Changed` behavior differs across two runs" needs a `Schedule` run
twice, not two separate `run_system_once` calls — the latter will silently
test the wrong thing (every call looking like "first ever run") rather
than failing to compile or erroring obviously. `run_system_once` stays
fine for the common case of "run this system once and check the result,"
just not for change-detection-across-ticks assertions specifically.

---

## M6 skill system: gated acquisition built now, `Od` naming, three `SkillKind` shapes

**Context:** M6's roadmap called for data-driven skill acquisition/upgrade,
2-3 mechanically distinct skills, and the "öd" resource power attacks
consume (see `MECHANICS.md`). Confirmed with the user before building:
build the full gated-acquisition data model now even though nothing will
be castable in a live playthrough until M8's skill-tree UI can spend
points into it — same call already made for M5's `Stats` bonuses, just
applied to skills too. Also confirmed: rename "öd" to "od" everywhere,
including in code identifiers.

**Decision — the resource-pool component is `Od`, not `Resource`.**
Straightforward once the rename was decided: `Resource` is already Bevy's
own ECS-resource derive macro, so naming the pool component that would
have shadowed a core Bevy concept for no reason. `Od { current, max,
regen_rate }` regenerates passively (`tick_od_regen`) and gains a flat
bonus on any landed melee hit (`combat::OD_GAIN_PER_HIT`, in
`attack_system`) — MECHANICS.md's "dual generation."

**Decision — `KnownSkills`/`UnspentSkillPoints` are empty/zero for every
character until M8.** Mirrors `UnspentStatPoints`/`Stats` from M5 exactly:
`progression::grant_xp` now awards skill points alongside stat points on
every level-up, but nothing exists yet to spend them into `KnownSkills`
(keyed by skill id, valued by upgrade level), so no character can actually
cast anything in a live playthrough this milestone. Confirmed explicitly
with the user rather than defaulting every character to "knows everything"
— the latter would have made the cast/cooldown/cost path live-testable
sooner, but at the cost of a fake acquisition model that would need
unwinding once the real UI arrives. The cast/cooldown/cost resolution path
itself (`skill_cast_system`) is still fully real and unit-tested; only the
"how does a character come to know a skill" gate is stubbed out.

**Decision — three `SkillKind` shapes, not one generic "attack" shape.**
`PowerStrike` (nearest single target, reuses `combat::attack_system`'s
targeting rule) and `AoeBurst` (every valid target within radius — a
genuinely different resolution, not a parameterized variant of nearest-
target search) both support the same per-hit `effects: Vec<EffectDefinition>`
melee attacks already have. `SelfBuff` has no target at all — it applies an
`EffectDefinition` straight to the caster's own `ActiveEffects`. New skills
reusing one of these three shapes are just a new `assets/spells/*.ron` file
(`content::SkillTemplate` mirrors `EnemyTemplate`'s load-time-conversion
pattern exactly, including reusing `EffectTemplate` from `enemy.rs` rather
than duplicating it); a fourth *shape* would be an engine change.

**Found while wiring the server's schedule — Bevy's tuple-based
`IntoSystemConfigs`/`Bundle` impls have a fixed arity ceiling.** Adding four
new systems to the already-long `Update` schedule chain (21 systems total)
made `.chain()` stop resolving — the trait is only implemented for tuples
up to a fixed size, not arbitrary length. Same problem hit the player
entity's component-insert bundle once `Od`/`KnownSkills`/`UnspentSkillPoints`/
`SkillCooldowns` were added. Fixed the same way in both places: group the
overflow into a nested tuple (`(a, b, c).chain()` as one element of an outer
chained tuple; a nested `(x, y, z)` as one element of the outer `insert(...)`
bundle) — Bevy already used this trick for the player's physics components
before this pass, it just hadn't been needed for the schedule tuple yet.
Ordering is unaffected: a nested `.chain()` group still runs start-to-finish
before the next tuple element starts.

**Consequences:** Any further schedule/bundle growth should watch for this
same ceiling and reach for the same nested-tuple fix rather than trying to
flatten everything into one tuple.

---

## M7 part 1: items, sockets/runes, loot tables — a dangling doc reference and a real scope decision

**Context:** ROADMAP's M7 bullet read "Affix/forging system (the 'custom
system' from `DESIGN.md`)" — checked `DESIGN.md`'s full git history before
starting and confirmed that phrase has never actually been described
anywhere in it, in any commit. Unlike M6 (where `MECHANICS.md` had a
concrete resource/skill shape to build against), the forging mechanic had
no real spec, just a dangling reference. Confirmed with the user before
building: a **socket/rune system** (items have a fixed number of sockets
from their template; runes are found/socketed for permanent stat bonuses;
unsocketing is free and reversible, not a currency sink — that can be
layered on once the vendor economy, M7 part 2, actually exists to spend
into).

**Decision — an item is its own instance (`template_key` + `sockets`), not
an entity.** Unlike enemies (one shared `EnemyTemplate` instantiated as
many identical-shape entities), two drops of the same item template have
independently-empty then independently-socketed sockets — genuinely
unique per-instance state, not just a content lookup key. Modeled as plain
data (`Item { template_key, sockets }`) carried inline inside
`Inventory`/`Equipment`/`ItemDrop`, not a full ECS entity with its own
lifecycle — avoids the complexity of transferring "ownership" of an entity
between world-drop and inventory-slot conceptual containers for something
that's fundamentally owned, non-shared data. Runes, by contrast, are
fungible (`RuneInventory: HashMap<rune_id, count>`), matching how
real-world stackable currency/materials are normally modeled rather than
tracking individual rune instances.

**Decision — item/rune stat bonuses actually wire into combat/movement
this pass**, not left inert like M5's `Stats` bonuses. Reusing the exact
"compute fresh at point of use" pattern `ActiveEffects::stat_bonus`
already established (see the M4 fury/ally-revive entry above):
`Equipment::stat_bonus(stat, &RuneLibrary)` sums matching socketed runes
across all three slots, added into `attack_system`'s effective crit stats
and `apply_move_input`'s effective speed, never baked into the base
component — so unsocketing a rune can't leave a stale bonus behind.
Deliberately **not extending `Stat` with a `MaxHealth` variant yet**:
`status_effect::Stat`'s own doc comment says "extend only once a new stat
is actually wired up somewhere, not speculatively," and safely rescaling
current health when max changes (what happens to a player at 40/100 when
max drops to 80?) needs its own deliberate handling, not a rushed addition
alongside everything else in this pass.

**Decision — a loot roll happens inline inside `combat::death_system`, not
a second system also watching for zero health.** A separate system
checking the same `Health::is_dead()` condition to decide whether to spawn
loot would race against `death_system`'s own despawn over which runs
first and whether the dying entity's `Position`/`LootTable` are still
readable — the same class of hazard the M5 disconnect-save entry solved
by moving off a second observer entirely. Inlining the roll into
`death_system`'s existing despawn branch sidesteps the race the same way.

**Found while wiring `game_core::item`'s `ItemDrop` spawn — `game_core`
can't insert `bevy_replicon`'s `Replicated` marker itself.**
`death_system` spawns a world-visible loot drop entity, but `game_core` has
no networking dependency at all (see `CLAUDE.md`'s crate boundaries), so
unlike `spawn_enemies`/`on_client_connected` (which insert `Replicated`
inline, in the same server-side function call, right after spawning),
there's no way for `game_core` itself to tag its own spawn as replicated.
Fixed with a small dedicated server-side system,
`tag_item_drops_for_replication`, reacting to `Added<ItemDrop>` and
inserting `Replicated` a moment later — a one-tick delay before a drop
becomes network-visible, which is imperceptible and not worth avoiding by
compromising the crate boundary.

**Consequences:** Any future `game_core` system that needs to spawn a
world-visible entity (not just mutate/despawn an existing one) will hit
the same `Replicated`-insertion gap and needs the same "server reacts to
`Added<T>` a moment later" fix, not an attempt to give `game_core` a
`bevy_replicon` dependency.

---

## M8 planning: a generic interact system, Tiled-authored placement, and what's deliberately deferred

**Context:** Planning M8 (UI: HUD & menus) surfaced more ground than the
roadmap bullets alone suggested — the user wants NPCs (blacksmith, sage)
and world objects (runestones) that trigger dialogs, effects, or panels on
interaction, not just static HUD/inventory screens. Worked through several
design questions with the user before writing any code; this entry
records the resulting decisions plus the concrete Bevy/`tiled` findings
implementation needs to know about.

**Decision — one generic `Interactable` concept, not one-off logic per
object type.** A runestone (dialog + effect), a blacksmith (opens the
forging panel), and a future objective-giving NPC (M9) are all the same
underlying shape: proximity + an action-button press triggers *something*.
Modeled as `Interactable { range, dialog: Option<String>, effect:
Option<EffectDefinition>, opens_panel: Option<String> }`. This pulls
forward part of M9's "special-character dialog" bullet by necessity (the
blacksmith needs a trigger mechanism regardless), but only the generic
mechanism — the actual *content* (objective triggers, boss-completion
hooks) stays M9's job, added later using the same component.

**Decision — dialog text is replicated data, read locally; only the
effect grant is a server round-trip.** `Interactable` (including its
`dialog` field) replicates like any other content-key-adjacent component,
so the client can show dialog text the instant the action button is
pressed, no round-trip needed for something purely informational. The
`effect` grant, if any, still goes through a server-resolved
`InteractRequested` — anything that mutates game state stays
server-authoritative, matching every other action system in this project.

**Decision — one action button (`E`) does double duty**, confirmed with
the user: it checks for the nearest `Interactable` in range first, falling
back to the nearest `ItemDrop` (pickup) if none is in range. One button to
learn, a clear priority rule, no separate "interact" key needed alongside
`PickupItemInput`'s existing `E` binding.

**Decision — forging becomes NPC-gated, a real behavior change from what
M7 shipped.** Confirmed with the user: socketing/unsocketing now requires
being in range of a blacksmith-kind `Interactable` (`opens_panel ==
Some("forging")`), not free-from-anywhere-via-hotkey like M7's stand-in.
`apply_socket_rune_input`/`apply_unsocket_rune_input` need a new
server-side proximity check against that specific `Interactable`, not just
against a plain range constant.

**Decision — Interactable placement is authored in the Tiled map, not a
hardcoded Rust position constant.** Raised directly by the user: a fixed
`SPAWN_BASE + spacing` constant (the pattern `spawn_enemies` already uses)
would disconnect placement from the actual level layout they're
designing — no way to put a blacksmith in a specific village square short
of guessing coordinates. Instead: a new "interactables" object layer in
`assets/maps/valley.tmx`, read the same way `spawn_map_colliders` already
reads the "collision" layer (see `server::spawn_map_colliders`), matching
each named point object against a `content::InteractableTemplate` key —
same convention as `EnemyKind`. **Found via source inspection, not
assumed:** the `tiled` crate (already pinned at `=0.16.0`, already a
dependency for collision) supports exactly this —
`~/.cargo/registry/.../tiled-0.16.0/src/objects.rs`'s `ObjectData` has a
`name: String` field and `x`/`y`, and `ObjectShape::Point(x, y)` exists
alongside the `Polygon` shape `spawn_map_colliders` already reads. The
user doesn't have the Tiled editor installed yet, so these object-layer
edits go directly into the TMX (plain XML) by hand for this pass, not
placed via the GUI.

**Decision — `bevy_egui` added as a client-only dependency.** Named in
`CLAUDE.md`'s stack from the start ("UI: egui via bevy_egui"), so this
isn't a new choice, just the point where it actually gets added. **Found,
not assumed:** `cargo add bevy_egui --dry-run` inside `client/` resolves
`bevy_egui v0.41.1` cleanly against the workspace's pinned `bevy = "=0.19.0"`
— verified this way instead of guessing a version or web-searching.
`bevy_egui`'s exact `wants_pointer_input`/`wants_keyboard_input`-equivalent
API on 0.41 hasn't been checked yet — implementation needs to verify the
real method names before wiring up the input-focus guard described below,
not assume they match older bevy_egui versions' API.

**Decision — skill tree is a flat spend-a-point list, no prerequisite
tree topology.** Neither `MECHANICS.md` nor `DESIGN.md` describes an
actual tree shape ("skill tree" is the roadmap's naming, not a confirmed
structure) — building prerequisite/branching logic now would be designing
for a shape nobody has actually decided on yet.

**Decision — `AttackTimer`/`SkillCooldowns` become replicated this pass.**
Both were server-only up to now (nothing needed them client-side). The
HUD showing real cooldown countdowns needs them on the client, so both
gain `Serialize`/`Deserialize` and a `.replicate::<T>()` registration.

**Decision — M7's UI-stand-in hotkeys (`F1`-`F9`) get removed once the
real panels exist**, not kept as a parallel shortcut path — two ways to
trigger the same equip/socket action would drift out of sync over time
for no real benefit. `WASD`/`Space`/`F` (revive)/`1`-`3` (skills) are
real-time combat actions and stay exactly as they are; per `CLAUDE.md`,
menus are overlays, not pauses, so none of these ever get suppressed just
because a panel is open.

**Explicitly deferred, with the reason recorded so it isn't re-litigated
later:** a `TAB`-style toggle-through skill selector, dedicated
per-attack hotkeys (e.g. slice/cleave/thrust), and dual primary/secondary
weapon slots. The user raised these, and the honest assessment (agreed
with the user) is that they're blocked on something more fundamental than
UI: **equipped weapons currently carry no combat-stat data at all.**
`ItemTemplate` only has `slot`/`socket_count`; `MeleeAttack`
(damage/range/cooldown) is a fixed constant set once at player spawn,
completely independent of what's equipped — a socketed rune changes crit/
speed, but swapping the weapon itself currently changes nothing
mechanically. Weapon-swap hotkeys are meaningless until weapons actually
drive `MeleeAttack`, which needs its own deliberate pass (new
`ItemTemplate` combat fields, `MeleeAttack` derived from `Equipment`
instead of a spawn-time constant) once weapon variety is actually
designed — not guessed at now alongside four new UI panels. M8 part 1's
skill HUD instead just shows icons for known skills next to the existing
fixed `1`-`3` keys, which needs none of this.

**Consequences:** When weapon-driven combat stats and a real moveset
design exist, revisit `Equipment`/`EquipSlot` for a second weapon slot and
the hotbar/toggle input model then — don't add either speculatively
before that prerequisite work happens.

---

## Found via live playtesting: `bevy_egui` 0.41 panels render but don't
## accept clicks unless drawn from `EguiPrimaryContextPass` (M8 part 1)

**Context:** The inventory and character panels (M8 steps 5-6) rendered
correctly in every screenshot taken during this pass's own live
playtests — HUD bars, item rows, stat/skill rows, buttons, all visually
present and correctly bound to replicated data. But when the user tried
the panels themselves: clicking "Equip" did nothing, the window's own
close (X) button didn't close it, the collapse arrow didn't respond, and
buttons never highlighted on hover. Only the window itself registered a
generic "brought to front" click somewhere in its bounds — every specific
widget was dead. `cargo build`/`clippy`/`fmt`/`test` were all clean
throughout, and nothing in the client log indicated an error; this is a
class of bug (rendering correct, real mouse interaction broken) that
static checks and screenshot-based verification cannot catch, only
driving the actual UI with a real pointer can.

**Root cause, found by reading `bevy_egui` 0.41's own source and
examples:** `hud_system`/`inventory_panel_system`/`character_panel_system`
were registered in the plain `Update` schedule, calling
`EguiContexts::ctx_mut()` and `egui::Window::show()` directly, matching
how earlier `bevy_egui` versions (and countless other integrations) are
normally used. But 0.41 introduced a dedicated `EguiPrimaryContextPass`
schedule — confirmed by reading `bevy_egui-0.41.1/examples/simple.rs`,
which registers its one UI system there rather than in `Update`, and by
the crate's own doc comment: "If you add UI systems, make sure they go
into the `EguiPrimaryContextPass` schedule - this will guarantee your
plugin supports both the single-pass and multi-pass modes." Tracing
`run_egui_context_pass_loop_system` (registered in `PostUpdate`'s
`EguiPostUpdateSet::EndPass`) confirmed why: it runs
`EguiPrimaryContextPass` *inside* `egui::Context::run_ui`'s closure — the
exact begin-pass/end-pass window in which egui's `Context::input()` holds
this frame's real pointer/keyboard state. A `.show()` call from anywhere
outside that window (like plain `Update`) still queues paint output
against the persistent `egui::Context` — hence correct rendering — but
runs with no valid current-frame input state, so every widget's
hover/click detection silently comes up empty. This is exactly why the
symptom was "visible but inert" rather than a panic or a blank panel.

**Decision:** Moved all three UI-drawing systems out of the `Update`
tuple into their own `.add_systems(EguiPrimaryContextPass, (hud_system,
inventory_panel_system, character_panel_system).chain())` call. Verified
`EguiPrimaryContextPass` runs in `PostUpdate`, strictly after the entire
`Update` schedule (including `init_replicated_players`, which sets
`LocalPlayer`) — so this move needed no companion change to make
`Res<LocalPlayer>`/replicated-component reads inside these systems see
fresh per-frame state; ordering already worked in our favor. Confirmed
live: the user re-tested immediately after and reported Equip now works.

**Consequences:** Any *future* egui UI system (the forging panel, dialog
panel, or anything else M8 steps 9-10 add) must be registered in
`EguiPrimaryContextPass`, not `Update` — this is now the load-bearing
convention going forward, not a one-off fix. If a UI system is ever added
back into `Update` by mistake, expect this exact symptom (renders, but
every widget is unclickable) and no compiler/clippy/test signal to catch
it — only a live click-through will surface it. Screenshot-based
verification (as used throughout this pass) is good for confirming
layout/data-binding but cannot substitute for actually clicking a button
with a real pointer at least once per new interactive panel.

---

## M7 part 2 planning: currency built once, shared by both vendor economy and forging cost

**Context:** With M8 part 1 (forging UI) shipped, the user asked about
adding a forging cost and a coins/reward system. Currency was already
scoped as part of M7 part 2's deferred vendor economy (`MECHANICS.md`'s
Economy section; individual per player, not shared/pooled). Rather than
bolt a forging-only currency field on now and rework it once vendors
land, confirmed with the user to build currency once as shared M7 part 2
groundwork, with vendor buy/sell and forging cost both layered on top of
it.

**Decision — currency is another weighted `LootTable`/`LootEntry` drop,
not a guaranteed per-kill grant.** Consistent with how item/rune drops
already work (`game_core::roll_loot`, M7 part 1) — a content author tunes
drop odds the same way for currency as for anything else, rather than a
second, differently-shaped reward mechanism living alongside the existing
loot roll.

**Decision — socketing costs currency, unsocketing stays free.** This
reverses the passing guess in M7 part 1's entry above (which floated
unsocketing as the eventual currency sink); confirmed instead that
*socketing* is the cost point, unsocketing remains the already-shipped
free/reversible action. Amounts, scaling (flat vs. per-rune-tier), and
whether it varies by socket/item are explicitly deferred — "dive deeper
into the socketing stuff later," not decided yet.

**Not yet decided (deferred to a follow-up conversation):** the concrete
currency amount for socketing, vendor price shape/content schema, and
whether currency needs its own HUD display before or alongside the vendor
UI.

**Noted in passing, not yet scoped:** the user also raised a future
rune/shard combination system — lower-tier runes/shards combine into more
powerful ones across several levels, performed at blacksmiths and a new
"sejdr" NPC type, at a quadratically-scaling currency cost per level. See
`MECHANICS.md`'s new "Rune/shard combination" section for the fuller
(still deliberately underspecified) shape. No design conflict with the
current `RuneLibrary`/`RuneInventory` model — both are already
open/data-keyed — so nothing here needs to change to keep this option
open; recorded purely so it isn't lost before its own design pass.

## M8.5: lighting/ambience foundation — `bevy_lit`/`bevy_hanabi` added, gizmo
## status-indicator redesign, two findings from live debugging

**Context:** Before starting real art (maps, tiles, character sprites), the
user wanted lighting/ambience effects (glow, torches, shadows, sparks)
available and tunable first, so sprite/tile colors get chosen against real
lighting rather than guessed blind — plus the one remaining real M8 UI gap
(minimap, party status). Research during planning ruled out `bevy_tiles`
(an abandoned, unrelated low-level grid-indexing crate, last released 2024
on Bevy 0.13 — a naming mistake, not adopted) and confirmed `bevy_lit`
0.11.0 and `bevy_hanabi` 0.19.0 both explicitly support Bevy 0.19 (this
project's exact pin). Everything in this milestone is client-only
rendering/presentation — no `game_core`/`server`/`protocol` changes — since
lights and particles have no gameplay effect to authorize or keep in sync,
the same category as the map's existing client-only visual tile layers.

**Decision — both new crates added client-only, verified rather than
trusted.** `bevy_lit = "=0.11.0"` and `bevy_hanabi = { version = "=0.19.0",
default-features = false, features = ["2d"] }` (this project has no 3D
rendering; `bevy_hanabi`'s default features are 3D-oriented) went into
`client/Cargo.toml` and `[workspace.dependencies]` only. Both pins were
confirmed via `cargo add --dry-run` against the resolvable version before
writing them down, not trusted from research alone, and `cargo tree -p
server` was re-run after each addition to confirm no leak — matching this
project's `bevy_ecs_tiled`/`avian2d` precedent for keeping the headless
server's dependency footprint minimal.

**Decision — status indicators (downed/stunned/leash-warning) moved off
`Sprite.color` onto a gizmo overlay.** The old `player_appearance_system`
signaled status by overwriting each entity's base sprite color every frame
— a real conflict with `bevy_lit`, which wants to own the sprite's "true"
color to shade it, and would have blanked out real texture detail once art
lands. Replaced with `status_indicator_system`, drawing a colored ring via
`gizmos.circle_2d` with the same `Downed` > `Stunned` > leash-warning
priority the old code used — immediate-mode, no spawned entity, no
lifecycle/stale-state risk, following the exact precedent
`facing_indicator_system` already established. Trade-off: less icon-like
than a spawned overlay sprite, but the right call for a placeholder-art-era
milestone. `player_appearance_system`/`PartySprites` removed entirely; base
sprite color is still set once at spawn (`init_replicated_players`/
`init_replicated_enemies`), untouched by this change.

**Decision — occluder count deliberately capped, backed by a measured
number, not assumed.** `bevy_lit` has an open upstream issue reporting
heavy performance cost from `LightOccluder2d` even without shadows
enabled. Capped at one placeholder occluder this pass. A temporary
`FrameTimeDiagnosticsPlugin`/`LogDiagnosticsPlugin` (added, measured, then
fully removed — never left in the codebase) recorded a stable ~60 FPS /
~16.7ms both right after the occluder was added and again at the very end
of the milestone with lighting + the occluder + two torch lights + torch
particle effects + the new debug panel all active together — no
regression from stacking the rest of the milestone on top. Zoom-extreme
(`MIN_ZOOM`/`MAX_ZOOM`) frame time wasn't separately isolated: camera zoom
only changes the orthographic projection scale, not scene complexity (light/
occluder/particle-emitter count is fixed regardless of zoom at this map's
scale), so a single steady-state reading was judged to generalize rather
than standing up a second client purely to force party spread.

**Decision — torches are placed via a Tiled object layer and read
client-side only, no server involvement.** A new `"ambience"` object layer
in `assets/maps/valley.tmx` holds named `"torch"` point objects (same
hand-edit-the-TMX approach the M8 interactables layer used). A new
client-only `spawn_torch_lights` system reads `bevy_ecs_tiled`'s
`TiledEvent<ObjectCreated>` and inserts a `PointLight2d` plus a
`ParticleEffect` directly onto the Tiled-spawned object entity — no
separate entity spawn, no `Replicated` marker, no protocol/server
involvement at all, matching the map's other purely-visual client-only
layers.

**Found via live debugging, not assumed — two real findings from the torch
work:**
- The anticipated risk (that `bevy_ecs_tiled`'s own per-object `Transform`
  computation might use a different coordinate convention than this
  project's hand-rolled `world_y = -tiled_y` rule) did **not** materialize.
  A temporary debug print confirmed the object `Transform` lands at exactly
  `(tiled_x, -tiled_y)` under `TilemapAnchor::TopLeft`, matching the
  server's manual convention precisely — no fallback to a second
  `tiled::Loader` pass was needed.
- The actual bug was unrelated: `bevy_ecs_tiled` sets the spawned object's
  Bevy `Name` component to a wrapped `"Point(torch)"`-style string (shape
  kind included, for its own debugging), not the raw Tiled object name —
  an exact-match filter against `Name` silently matched nothing. Fixed by
  matching against the separate `TiledName` component instead, which holds
  the plain `"torch"` string. Found only after the first attempt rendered
  no torches at all and a live debug print traced why, not guessed at up
  front.

**Decision — the lighting/ambience debug panel is a dev-only sandbox, not
persisted or replicated.** `lighting_debug_panel_system` (registered in
`EguiPrimaryContextPass`, per the M8-part-1 finding above it) exposes
sliders for the single `AmbientLight2d` plus one collapsible section per
`PointLight2d` entity, labeled by `TiledName` where available (the
torches) and by entity id otherwise (the smoke-test light). This is the
actual tool the user will use to pick sprite/tile colors against real
lighting before any art exists — values reset to their `setup_scene`/
`spawn_torch_lights` defaults on restart, deliberately, since nothing here
needs to survive a session.

**Found via live testing: an egui widget-id clash from two lights sharing
one label.** Both torches share the same `TiledName` (`"torch"`); using
that label directly as a `ui.collapsing(...)` section's id caused egui's
own "first/second use of widget ID" warning box to render on top of the
second torch's sliders, visually blocking them (not a crash, not a build/
clippy/test failure — only visible by actually opening the panel and
expanding both sections). Fixed by wrapping each light's whole widget
block in `ui.push_id(entity, ...)`, salting every child widget's id with
the owning entity so same-labeled lights no longer collide. Same general
lesson as the M8-part-1 `EguiPrimaryContextPass` finding: interactive-UI
bugs in this stack tend to be invisible to static checks and only surface
by actually operating the widget.

**Consequences:** Any future spawned light (new torch types, a spell VFX,
etc.) that reuses a shared/generic name in its `TiledName` or label must
either salt its debug-panel id with the entity (`ui.push_id`) or otherwise
avoid label-only egui ids, per the finding above. Any future status-effect
visual should default to a gizmo overlay rather than sprite-color
overwriting, per the `bevy_lit` conflict this milestone resolved. Real art
(sprites/tiles) can now be color-chosen against actual ambient + point
lighting via the debug panel, unblocking the graphics-design work this
whole milestone existed to prepare for.

## M8.6: weapon-driven combat — decisions confirmed before implementing,
## built autonomously, and two real bugs caught before they shipped

This milestone was scoped and implemented in one autonomous session (the
user was unavailable to answer follow-up questions mid-session), following
an explicit up-front decision round rather than guessing shapes that
weren't settled — see `ROADMAP.md`'s M8.6 section for the full
implementation writeup; this entry covers the *why* behind the choices and
the two real correctness gaps caught along the way.

**Decisions confirmed with the user before writing any code:**
1. **Phasing (windup/recovery) is player-only.** Enemies keep
   `combat::AttackTimer`'s flat-cooldown resolution completely unchanged.
   The alternative (phasing both) would have forced every `EnemyTemplate`
   `.ron` file to gain new fields and required deciding an AI
   "committed to windup" behavior — real scope this milestone didn't need
   to take on to prove the player-side mechanic.
2. **Windup/recovery state is a single `AttackPhase` enum**
   (`Idle | Windup{remaining, target} | Recovery{remaining}`), not two
   loose `f32` fields alongside the old `AttackTimer`. An enum can't
   represent the invalid state of being in both phases simultaneously; two
   plain fields could only be kept mutually exclusive by convention.
3. **Target locks in at windup-start**, re-validated (still has `Health`,
   not vanished) but not re-selected at hit-resolution — a player who
   commits to a swing doesn't have it silently retarget to something closer
   that wandered in mid-animation.
4. **Cone-gating applies to both players and enemies** — the one place the
   user went against the initial recommendation (which was player-only, to
   avoid touching `attack_system`'s already-shipped, live-tested enemy
   path). This had a real, non-optional consequence: cone-gating enemies
   without also fixing their facing would make them whiff constantly,
   since `MECHANICS.md` had documented enemy `Facing` as purely
   movement-derived since M4. Rather than silently deciding how to resolve
   that tension, it was surfaced as a follow-up question and the user chose
   to fold a narrow fix — an attack-time facing snap, not a rewrite of the
   general facing system — into this same milestone. `MECHANICS.md`'s
   Facing section now documents this as an explicit, narrow exception.

**Why `attack_system`'s hit-resolution was extracted into a shared
`resolve_melee_hit`, touching already-tested code deliberately.** The
initial plan was to leave `attack_system` completely untouched and
duplicate its crit/resistance/XP/effect logic for the new player path, to
minimize risk to code that couldn't be live-verified this session. That
was reconsidered: the duplication risk (two formulas silently drifting
apart over time, exactly what MECHANICS.md's "Effective combat values are
always computed fresh" section warns about) was judged worse than the
extraction risk, given the *existing* `attack_system` test suite — every
one of those tests still had to pass, unchanged, after the refactor, which
is a strong behavior-preservation check even without a live playtest.
`resolve_melee_hit`/`AttackerProgress` (`combat.rs`) is now the one place
a landed hit resolves, called from both `attack_system` (enemies) and the
new `weapon_attack::tick_player_attack_phases` (players).

**Two real "stat exists but does nothing" bugs, caught before shipping, not
after.** Both are exactly the bug class `MECHANICS.md` names explicitly
(citing `Stats`' M5-era bonus fields sitting unread for a full milestone
before M8 wired them in) — worth recording precisely because this pass had
no live playtest to catch them the way that earlier bug eventually was:
1. `Equipment::resistance_bonus` and the `Resistances::default()` component
   on player spawn were both built, but the first pass never actually
   *called* `resistance_bonus` from damage resolution — armor/helmet
   resistance would have been completely inert. Caught while writing this
   changelog entry (re-reading the roadmap bullet against the actual diff),
   not by a failing test. Fixed by threading `target_equipment`/
   `ItemLibrary` into `resolve_melee_hit`; a new test
   (`attack_system_applies_the_targets_equipped_armor_resistance`) fails
   without the fix and passes with it.
2. The player attack path's first draft passed a throwaway
   `RuneLibrary::default()` into `resolve_melee_hit` instead of the real
   `Res<RuneLibrary>`, which would have silently disabled socketed
   crit-chance/multiplier rune bonuses on player attacks specifically
   (enemies never socket runes, so `attack_system`'s existing tests
   wouldn't have caught this either — it was invisible to the whole
   existing test suite). Caught in the same review pass as the resistance
   bug; fixed by adding a real `Res<RuneLibrary>` parameter.

Both bugs shared a root cause: new plumbing (`ItemLibrary`/`RuneLibrary`
resources) needed by the new player path wasn't threaded all the way
through on the first pass, and nothing forced a compile error because
`Option`/`Default` made the gap type-check cleanly. Neither would have
been caught by `cargo build`/`clippy` — only by a test written specifically
against the missing behavior, or a live playtest. Worth remembering next
time new cross-cutting plumbing (a `Res<T>` needed by a newly-shared
helper) gets threaded through multiple call sites: verify each call site
actually passes the *real* resource, not a structurally-valid placeholder.

**Consequences / what's still open:**
- **Confirmed live, single-client, after this milestone was built.** It was
  implemented and fully verified (build/test/clippy/fmt, ~30 new/changed
  tests) without a graphical session available, then actually played
  afterward: player attacks instrumented with temporary debug logging (see
  the logging entry below) confirmed hits were landing and dealing real
  damage server-side, and the user confirmed combat was functional live
  (cone-gating/windup made landing a kill "difficult" — a real first
  feel-signal, not yet tuned against) alongside pickups/dialogs (unaffected
  pre-existing M8 features). **Not yet co-op-verified with two clients** —
  the enemy facing-snap and cone-gating haven't specifically been exercised
  from a second player's perspective.
- `AttackPhase` is deliberately not yet replicated — the HUD windup/
  recovery display is explicitly deferred, not forgotten (see `ROADMAP.md`).
- Enemy phasing (windup/recovery for enemy attacks) remains unbuilt and
  unblocked, per decision #1 above.
- The melee cone half-angle (`combat::MELEE_ARC_HALF_ANGLE_RADIANS`, ~50
  degrees) is a single fixed value shared by every weapon — not yet a
  per-weapon content field, flagged in `MECHANICS.md`'s Open questions.

## Debug logging: `tracing` added to `game_core`, gated by `RUST_LOG` (M8.6 follow-up)

While live-testing M8.6, ad hoc `eprintln!` debug prints (added to trace
whether player attacks were finding targets/resolving hits) needed to
become something toggleable rather than hand-added-and-removed each time.
Confirmed with the user: add `tracing` (the lightweight macro facade, not
`tracing-subscriber`) as a real dependency to `game_core`, pinned to
`=0.1.44` — the exact version already resolved transitively via
`bevy::log::LogPlugin`, confirmed via `cargo tree -p server` to still
resolve to a single version, no duplicate. `server` already installs the
actual subscriber/backend through `bevy::log::LogPlugin`, so `game_core`'s
`tracing::debug!`/`trace!` calls "just work" and are filterable via the
standard `RUST_LOG` env var (e.g. `RUST_LOG=game_core=debug cargo run -p
server`) with zero custom on/off plumbing — off by default. The
`start_player_windups`/`tick_player_attack_phases` debug prints added
during this same testing session were converted to `tracing::debug!` calls
rather than removed, since they're exactly the kind of "why didn't my
attack land" instrumentation worth keeping available for future debugging.

**Unrelated operational finding from the same session, worth remembering:**
a server restart briefly panicked on `failed to read content file
assets/vendors` — not a code bug. `server`'s asset paths (`ITEM_TEMPLATES_DIR`
etc.) are relative to the process's current working directory, and an
earlier `cd game_core && cargo add ...` in the same shell session had
permanently changed that directory for every subsequent command (shell cwd
persists across tool calls). Always run `cargo run -p server`/`cargo run -p
client` from the repo root; prefer not to `cd` at all mid-session when
relative-path-sensitive commands (like starting the server) are coming up
later in the same session.
