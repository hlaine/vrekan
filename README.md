# Vrekan

*Old Norse: "I will avenge."*

A co-op multiplayer action RPG (ARPG) — real-time, endgame-style loot grinding,
no cutscenes, built in Rust with Bevy.

See [`DESIGN.md`](./DESIGN.md) for the game design and [`CLAUDE.md`](./CLAUDE.md)
for engineering conventions.

> This README is kept up to date as the project evolves — if a command below
> doesn't work, that's a bug, not a stale doc.

## Supported platforms

**Windows, macOS, and Linux (native) only for v1.** Web and mobile are not
supported — see `CLAUDE.md` for why.

## Prerequisites

### All platforms
- [Rust](https://rustup.rs/) (stable channel) via `rustup`

### Linux only
Build dependencies for windowing, audio, and input:

```bash
# Debian/Ubuntu
sudo apt-get install g++ pkg-config libx11-dev libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev

# Fedora
sudo dnf install gcc-c++ libX11-devel alsa-lib-devel systemd-devel wayland-devel libxkbcommon-devel
```

You'll also need a Vulkan driver matching your GPU, e.g. `mesa-vulkan-drivers`
(AMD/general), `vulkan-intel` (Intel), or your vendor's proprietary driver
(NVIDIA). Intel GPUs in particular don't get Vulkan installed by default on
most distros — install it explicitly or the client won't launch.

### Windows / macOS
No extra setup beyond the standard Rust toolchain (Windows: MSVC build tools,
usually installed alongside `rustup`; macOS: Xcode command line tools via
`xcode-select --install`).

## Building

```bash
# Build everything (client, server, all crates)
cargo build --workspace

# Build just the server (headless, no rendering/audio deps required)
cargo build -p server

# Build just the client
cargo build -p client

# Release build (much faster runtime, much slower to compile)
cargo build --workspace --release
```

## Running

> **Current status:** M4 (networked combat, damage types, downed/revive,
> enemy collision, status effects) is complete; M5 (leveling/persistence)
> is in progress — see `ROADMAP.md`.

```bash
# Start a server (listens on UDP port 5000). The optional argument names
# which save directory this server instance uses (saves/<name>/) —
# defaults to "default" if omitted. One server process is one game.
cargo run -p server -- my_game

# In one or two other terminals, start a client
cargo run -p client
```

Run both from the repo root — the client loads enemy templates from
`assets/enemies/` relative to the current directory. Controls: **WASD** to
move (sent to the server, which simulates and replicates position back),
**Space** to melee-attack the nearest enemy in range, **F** (hold) to
revive a downed ally standing nearby.

**Connecting:** each client generates a persistent character ID on first
run (saved to `character_id.txt` next to where you ran it) and prompts in
the terminal for a game password and a character password before
connecting. The game password is checked against whatever the *first*
client to ever connect to that `saves/<name>/` directory supplied (i.e. the
host effectively sets it); the character password is checked against that
character's own save the first time it's used and must match on every
later reconnect. Neither is remembered client-side — expect the prompt
every launch. There's no real account system or menu UI yet (planned for
M8) — this is a deliberately minimal stand-in; see `DECISIONS.md` for the
identity model this is based on.

For local co-op testing on one machine, run one server and up to two client
instances (`ROADMAP`/`DESIGN` party cap for v1) — each client connects to
`127.0.0.1:5000` automatically. For testing across machines, the host will
need to forward the server's UDP port (5000) through their router, and
clients will need the `server` crate's connection address made configurable
(currently hardcoded to localhost — a future milestone).

Enemies are data-driven: adding a new enemy type is just a new `.ron` file in
`assets/enemies/` (see the existing files for the schema) — no code changes
needed.

Map terrain/collision is authored in [Tiled](https://www.mapeditor.org/) —
`assets/maps/valley.tmx` is the current test map, a valley with a narrow
pass between two mountain shapes. Collision polygons live in the map's
"collision" object layer; both `client` (rendering, via `bevy_ecs_tiled`)
and `server` (physics, via the plain `tiled` crate + avian2d) load the same
file, so the visual and collision geometry can't drift apart — see
`server/src/main.rs`'s `spawn_map_colliders` doc comment for the coordinate
convention this depends on if you add or edit maps.

## Testing

```bash
# Run all tests
cargo test --workspace

# Lint (warnings are treated as errors, matching CI)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Formatting check
cargo fmt --check
```

These three checks plus `cargo build --workspace` all run in CI on every push
and PR — see `CLAUDE.md` for the full verification-loop policy.

## Development tips

- Compile times are the biggest friction point during iteration. For faster
  client rebuilds during local dev, run with Bevy's `dynamic_linking` feature:
  `cargo run -p client --features dynamic_linking` (don't use this for release
  builds — it requires shipping Bevy's `.so`/`.dylib`/`.dll` alongside the
  binary). Using a faster linker (`lld` or `mold` on Linux) also helps.
- The server never needs a GPU, audio device, or display — it's safe to run
  headless on a minimal machine or CI runner.
