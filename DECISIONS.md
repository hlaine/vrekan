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
