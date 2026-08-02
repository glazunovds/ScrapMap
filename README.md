# ScrapMap — a live map overlay for Scrap Mechanic

A portable Windows overlay that draws a live map for Scrap Mechanic: a compact
minimap pinned to the game window, expandable to a full map, with fog of war,
markers, points of interest and player positions.

Local-first and read-only with respect to gameplay. No Cheat Engine, no writes
to game memory, no graphics hook, no installer and no background service.

**Keywords:** Scrap Mechanic map, minimap, overlay, fog of war, points of
interest, waypoints, survival map, world map, multiplayer positions.

| | |
|---|---|
| Game | Scrap Mechanic **1.0.4.874**, Survival, Windows x64 |
| Requires | `-dev` in the Steam launch options |
| Platform | Windows 10 / 11, WebView2 (preinstalled on Windows 11) |
| Distribution | a single portable `.exe` — no installer, no service, no ports |
| Languages | English, Russian |

Other game builds are untested rather than blocked. The tile atlas is sampled
from whatever build is installed, so it stays correct across updates; the Lua
patch is the part a game update can break, and it is exactly reversible.

## What works

- Compact minimap attached to the game window, expanding to a full map
- A tile atlas generated from the game's own terrain data — all 493 tiles, with
  ground cover, water, hillshading, roads and forest, and buildings drawn from
  their own collision-mesh footprints
- Fog of war, with a reveal-all control
- Points of interest by category, with icons, filters and search
- Local markers, per-world profiles, separate profiles for peer-hosted worlds
- Live positions and names for **every player in the session**, not just you —
  and fog reveals around all of them
- Photographs of selected points of interest, taken with the game's own camera

Not yet: fog and markers do not persist *between* players — each machine keeps
its own. See `docs/ROADMAP.md`.

## Installing

1. Download `scrapmap.exe` from the releases page. There is nothing to install.
2. Add `-dev` to Scrap Mechanic's launch options in Steam. Without it the game
   ignores the patched scripts entirely.
3. Run `scrapmap.exe`. It adds a tray icon.
4. **Tray → Install game patch.** This is what makes live positions work; the
   map itself does not need it.
5. Start the game and load a survival world. The first load bakes the tile
   atlas — about twenty seconds, once.

**Tray → Restore game files** puts the game back exactly, which you need before
joining anyone who has not patched.

| Shortcut | Action |
|---|---|
| `Ctrl+Shift+M` | Compact map ↔ full map |
| `Ctrl+Shift+H` | Hide / show the overlay |
| `Ctrl+Shift+Q` | Quit |
| `Escape` | Return the full map to compact |

Interface language is under **Tray → Interface language**.

## Playing together

Scrap Mechanic verifies Lua script checksums when a client joins a server, so
**both players must run the same patch revision** or neither gets in. Install
the patch from the same ScrapMap version on both machines, and revert before
joining anyone unpatched. See `docs/MULTIPLAYER-TEST.md`.

Live positions work while you are in a session together, with no server or
account involved: the game already tells each client where everyone is, and
ScrapMap reads that.

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

`tools/game-patch.mjs` does the same job as the tray entries from a terminal,
which is useful while developing. A released build needs neither it nor Node:
the Lua is compiled into the executable.

Runtime data lives in `%LOCALAPPDATA%\ScrapMap`: the SQLite database, the tile
atlas cache, and `ui.log` if the panel reports a problem. Everything in the
atlas cache is disposable and rebuilds.

## Documentation

| Document | Contents |
|---|---|
| `CLAUDE.md` | Orientation, conventions, and the expensive lessons |
| `docs/ARCHITECTURE.md` | How the pieces fit; identity and storage rules |
| `docs/GAME-INTEGRATION.md` | The Lua patch, the atlas bake, POI photography |
| `docs/MULTIPLAYER-TEST.md` | Instructions for the second player |
| `docs/HANDOFF.md` | POI photography: state, faults found, open questions |
| `docs/SYNC.md` | Shared fog and markers design (not started) |
| `docs/ROADMAP.md` | Milestones and what remains |
| `docs/DIAGNOSTIC-FEED.md` | Legacy JSON telemetry input |

Interface strings live in `public/map/locales/<code>.json`. A new language is a
file plus one entry in `LANGUAGES` in `public/map/i18n.js`; missing keys fall
back to English, so a partial translation is usable.

## Scope

Private project for a couple of people, published in case it is useful. It reads
what the game already shows its own client and draws a map from it. It does not
modify inventory, health, crafting, combat or server state.

One deliberate exception, and it asks first: the POI photography sweep moves
your character and turns on god mode for its duration, restoring both
afterwards. Nothing else touches gameplay.

Game files are never redistributed. The patch is exactly reversible, and the
pristine copies it restores from are kept out of the repository.
