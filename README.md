# ScrapMap

A portable Windows overlay that draws a live map for Scrap Mechanic: a compact
minimap pinned to the game window, expandable to a full map, with fog of war,
markers, points of interest and player positions.

Local-first and read-only with respect to gameplay. No Cheat Engine, no writes
to game memory, no graphics hook, no installer and no background service.

## What works

- Compact minimap attached to the game window, expanding to a full map
- A tile atlas generated from the game's own terrain data — all 493 tiles, with
  ground cover, water, hillshading, buildings and forest
- Fog of war, with a reveal-all control
- Points of interest by category, with icons, filters and search
- Local markers, per-world profiles, separate profiles for peer-hosted worlds
- Live position for the local player

Not yet: shared fog and markers between players. Photographed points of interest
are implemented but unproven — see `docs/ROADMAP.md`.

## Running it

Launch Scrap Mechanic through Steam with `-dev` in the launch options, then
start `scrapmap.exe`. It finds the game, attaches to its window and follows it.

| Shortcut | Action |
|---|---|
| `Ctrl+Shift+M` | Compact map ↔ full map |
| `Ctrl+Shift+H` | Hide / show the overlay |
| `Ctrl+Shift+Q` | Quit |
| `Escape` | Return the full map to compact |

The map needs a one-time setup that patches the game's Lua scripts:

```bash
node tools/game-patch.mjs snapshot   # once, on a clean install
node tools/game-patch.mjs apply
```

Then load a survival world. The first load bakes the tile atlas — about twenty
seconds, once. `node tools/game-patch.mjs revert` puts the game back.

**Playing with someone else** requires both players to run identical patched
files, because the game verifies script checksums on connect. See
`docs/MULTIPLAYER-TEST.md`.

## Building

Requires Node.js 20+, pnpm, Rust stable `x86_64-pc-windows-msvc`, the Visual
Studio Build Tools C++ workload, and the WebView2 Runtime.

```bash
pnpm install
pnpm tauri dev                 # development
pnpm tauri build --no-bundle   # portable EXE
pnpm test                      # frontend tests
cargo test --manifest-path ./src-tauri/Cargo.toml
```

The portable executable lands at `src-tauri\target\release\scrapmap.exe`. Use
`pnpm tauri build`, not `cargo build` — a plain cargo build produces an EXE that
looks for a dev server instead of the bundled frontend.

Runtime data lives in `%LOCALAPPDATA%\ScrapMap`: the SQLite database and the
tile atlas cache. Everything in the atlas cache is disposable and rebuilds.

## Documentation

| Document | Contents |
|---|---|
| `CLAUDE.md` | Orientation, conventions, and the expensive lessons |
| `docs/ARCHITECTURE.md` | How the pieces fit; identity and storage rules |
| `docs/GAME-INTEGRATION.md` | The Lua patch, the atlas bake, POI photography |
| `docs/MULTIPLAYER-TEST.md` | Instructions for the second player |
| `docs/SYNC.md` | Shared fog and markers design (not started) |
| `docs/ROADMAP.md` | Milestones and what remains |
| `docs/DIAGNOSTIC-FEED.md` | Legacy JSON telemetry input |

## Scope

Private project for a couple of people. It reads what the game already shows its
own client and draws a map from it. It does not modify inventory, health,
crafting, movement, combat or any server-controlled state, and it is not
intended to.

Game files are never redistributed. The patch is exactly reversible, and the
pristine copies it restores from are kept out of the repository.
