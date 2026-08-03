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
