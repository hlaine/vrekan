# CLAUDE.md

Conventions and context for working on this project. Read this before making changes.

## Project

A co-op multiplayer action RPG (ARPG), loot-grind/hack-and-slash genre.
Priorities, in order:
1. Clear, idiomatic architecture over fastest-possible delivery — prefer
   solutions that are easy to read and reason about over clever/opaque ones.
2. Genuine content extensibility: adding a new enemy, item, or spell should
   mean adding data, not rewriting engine code.
3. Functional authoritative client-server multiplayer.

Stack: **Rust + Bevy (ECS)**. Networking: **bevy_replicon**. UI: **egui via
bevy_egui**. Content data: **RON**.

See `DESIGN.md` for game design context and `ROADMAP.md` for the milestone
sequence — check `ROADMAP.md` before starting work to confirm what's next and
what's explicitly out of scope for the current milestone.

## Architecture

Workspace with these crates. Respect the boundaries — if you're unsure which crate
something belongs in, flag it rather than guessing.

- `engine_core/` — generic ECS utilities, scheduling, math helpers. Nothing
  game-specific lives here. Should still make sense if this became a different game.
- `game_core/` — shared simulation logic: components, events, combat resolution,
  damage math, status effects. **No rendering or networking dependencies.** This is
  what makes gameplay logic unit-testable without spinning up a Bevy app, and what
  keeps client and server running identical simulation logic.
- `protocol/` — network message types and (de)serialization. Depends on `game_core`
  for shared types, but is a distinct crate so the network boundary stays visible.
- `content/` — RON schemas + loaders for enemy/item/spell templates, and the logic
  to spawn entities from a template. This is where "add a new enemy" should live
  entirely: a new `.ron` file, plus a new component only if genuinely new behavior
  is needed.
- `server/` — binary. Headless authoritative simulation, tick loop, networking
  listener. Thin — wires the other crates together plus server-only concerns.
- `client/` — binary. Rendering, input, UI, camera, prediction/interpolation. Thin,
  same principle.

## Autonomy — when to proceed vs. flag for review

**Proceed without asking:**
- New components, systems, or events within an existing crate
- New tests
- New content files (enemy/item/spell `.ron` entries) following the existing schema
- Refactors contained within a single crate/module

**Flag for review before doing it** (explain the tradeoff, wait for a decision):
- New crates, or moving code across existing crate boundaries
- New external dependencies
- Changes to the `protocol` crate (wire format changes affect client/server compat)
- Bumping Bevy or other core dependency versions
- Anything that would touch more than ~3 crates at once

## Error handling

- **Panic on invariant violations** — bugs, states that should never occur given
  correct code (e.g. an entity missing a component it should always have). Don't
  wrap these in `Result` just to avoid a panic; that hides bugs instead of
  surfacing them.
- **Use `Result` for genuinely recoverable cases**: content file parsing, asset
  loading, network I/O. These can legitimately fail at runtime and callers should
  be able to handle it.
- Avoid bare `.unwrap()` in non-test code where a `Result` case is realistic; a
  panic from an invariant violation is fine, an unhandled parse error crashing the
  server is not.

## Documentation

Doc comments (`///`) only where behavior isn't obvious from the name and
signature — e.g. non-obvious units, ordering requirements, or why something is
structured a particular way. Don't add boilerplate doc comments that just restate
the function name.

## Verification loop & definition of done

A task isn't finished until all of the following pass cleanly:

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
```

Run these after making changes, fix what fails, and repeat until clean — don't
hand back code that doesn't compile, has failing tests, or has unresolved
clippy warnings as if the task were done.

- Clippy warnings must be resolved, not suppressed by default. If a warning is
  a genuine false positive or a deliberate exception, use a narrowly scoped
  `#[allow(...)]` with a comment explaining why — never a blanket crate-level
  allow to make the build quiet.
- If a test fails, fix the code (or, if the test itself was wrong, fix the
  test) — don't delete or weaken a test just to make it pass.
- If stuck after reasonable iteration, stop and report the specific blocker
  rather than shipping something broken or silently working around it.

## Continuous integration

A GitHub Actions workflow runs on every push/PR: `cargo fmt --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`, and
`cargo test --workspace`. Set this up as part of the initial workspace
scaffolding, not deferred — every commit from the start should be checked, not
just ones after some future cleanup pass.

## Testing

Pragmatic, not test-first. Write tests for logic where correctness is easy to get
subtly wrong or expensive to debug in-game:
- Combat/damage resolution
- Loot table rolls (especially weighted/probabilistic logic)
- Status effect stacking/expiry
- Content loading (a malformed `.ron` file should fail loudly and clearly)

Skip tests for trivial code (simple getters, straightforward data structs).
`game_core` logic should be testable without a running Bevy app — that's the
point of keeping it free of rendering/networking dependencies.

## Dependencies & versioning

Pin exact versions in `[workspace.dependencies]`. Do not bump Bevy or other core
dependencies as a side effect of an unrelated task — flag it and treat upgrades as
their own deliberate task, since Bevy minor versions routinely break APIs.

## Module organization

Group by feature/domain within a crate (e.g. `combat.rs` holds related combat
components and systems together) rather than one file per component. Split a file
further only once it's actually unwieldy, not preemptively.

## Prerequisites & platforms

- **Target platforms for v1: Windows, macOS, Linux (native) only.** No web/WASM,
  no mobile. Web is ruled out specifically because bevy_replicon/Renet's UDP
  transport doesn't work in-browser without swapping transports — not just
  deprioritized, architecturally incompatible with the current networking
  choice unless that changes.
- **Dev machine**: Rust stable via `rustup`. Linux additionally needs
  `pkg-config`, a C compiler, and dev packages for ALSA, libudev, X11/Wayland,
  plus a Vulkan driver matching the GPU. Windows/macOS need no equivalent setup
  step beyond their standard toolchains.
- **Server is headless**: built with `MinimalPlugins`, not `DefaultPlugins` —
  no rendering, audio, or windowing dependencies at all. Needs an open UDP port
  for the Renet/bevy_replicon transport; NAT/port-forwarding is a real concern
  when testing co-op across machines, not just at eventual deployment.
- **Binaries are mostly self-contained** but not fully static: Linux builds
  dynamically link ALSA/X11/Wayland/the Vulkan loader at runtime (normally
  already present on a desktop install, not bundled). Assets ship as a folder
  alongside the binary, not embedded — packaging strategy is a distribution-time
  decision, not a v1 blocker.

## Keeping the README current

`README.md` contains install/build/run instructions and must stay accurate.
Whenever a change affects how someone sets up, builds, or runs the project —
new crate, new dependency with its own setup step, changed run command, new
platform-specific requirement — update the relevant section of `README.md` in
the **same commit** as the change. Don't let it drift; treat outdated setup
instructions as a bug, not a documentation nice-to-have.

## UI & simulation state

Menus (inventory, skill tree, forging, etc.) are **overlays, not pauses** — the
simulation keeps running while a menu is open, in both solo and co-op, since
co-op has no way to pause for one player without freezing it for everyone. Do
not add code paths that stop/suspend the Bevy schedule when a menu opens. A
player can be hit while their inventory is open; this is intentional, not a bug
to fix later.

## Commits

Conventional commits: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`.
Keep commits scoped to one logical change.
