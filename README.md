# [Project name TBD]

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
sudo apt-get install g++ pkg-config libx11-dev libasound2-dev libudev-dev

# Fedora
sudo dnf install gcc-c++ libX11-devel alsa-lib-devel systemd-devel
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

```bash
# Start a server (listens for co-op connections; requires an open UDP port)
cargo run -p server

# In another terminal, start a client
cargo run -p client
```

For local co-op testing on one machine, run one server and two client
instances; connect both clients to `127.0.0.1`. For testing across machines,
the host will need to forward the server's UDP port through their router.

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

- Compile times are the biggest friction point during iteration. Consider
  enabling Bevy's `dynamic_linking` feature for local dev builds, and using a
  faster linker (`lld` or `mold` on Linux) to speed up rebuilds.
- The server never needs a GPU, audio device, or display — it's safe to run
  headless on a minimal machine or CI runner.
