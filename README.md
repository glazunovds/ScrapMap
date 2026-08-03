# ScrapMap — a live map overlay for Scrap Mechanic

A portable Windows overlay that draws a live map for Scrap Mechanic: a compact
minimap pinned to the game window, expandable to a full map, with fog of war,
markers, points of interest and player positions.

Local-first and read-only with respect to gameplay. No Cheat Engine, no writes
to game memory, no graphics hook, no installer and no background service.

**Keywords:** Scrap Mechanic map, Scrap Mechanic minimap, Survival map,
in-game overlay, fog of war, exploration tracker, points of interest, POI
locations, waypoints, markers, world map, tile atlas, player coordinates,
multiplayer player positions, co-op, warehouse and trader finder, Windows
overlay, Steam, portable, no Cheat Engine.

![The full map: baked terrain, roads, water, and points of interest by
category, with the world overview panel on the right](screenshots/full-map.png)

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
4. **Tray → Install game patch.** Not optional: the patch is what writes the
   world layout and the terrain samples, so without it there is nothing to draw
   but the built-in demo. It is also what reports player positions.
5. Start the game and load a survival world. The first load exports the layout
   and bakes the tile atlas — about twenty seconds, once.

**Tray → Restore game files** puts the game back exactly, which you need before
joining anyone who has not patched. It restores from pristine copies taken the
first time you patched, kept in `%LOCALAPPDATA%\ScrapMap\vanilla` so that
verifying the game through Steam cannot take them with it.

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

## Where things are kept

Two roots, and nothing outside them: its own folder under `%LOCALAPPDATA%`, and
the game directory while the patch is installed. Uninstalling is *Restore game
files*, then deleting the executable and `%LOCALAPPDATA%\ScrapMap`.

| Path | What it is |
|---|---|
| `%LOCALAPPDATA%\ScrapMap\scrapmap.sqlite3` | Profiles, fog, markers, routes |
| `%LOCALAPPDATA%\ScrapMap\vanilla\` | Pristine game scripts; what *Restore* restores from |
| `%LOCALAPPDATA%\ScrapMap\language.txt` | The interface language the tray reads at startup |
| `%LOCALAPPDATA%\ScrapMap\ui.log` | Panel errors and profile decisions, if any |
| `%LOCALAPPDATA%\ScrapMap\atlas\` | Tile images and the manifest — **disposable**, rebuilds |
| `<game>\Survival\Scripts\…` | Four added Lua files, and two stock ones with a block appended |
| `<game>\Survival\ScrapMapLayout.json` | The world layout, exported on world load |
| `<game>\Survival\ScrapMapAtlas\` | Raw terrain samples the game writes during a bake |

Delete the atlas cache freely to force a re-bake; nothing else in there is
worth keeping. The other files are not caches — losing `vanilla\` means
*Restore* has to strip the patch out of the game's own files instead of copying
originals back, which works but is less exact.

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
| `src-tauri/CLAUDE.md` | Rust module map, threads, the atlas pipeline |
| `public/map/CLAUDE.md` | Renderer: load order, static frame, POI categories |
| `game-patch/CLAUDE.md` | What each Lua file does and why it is shaped that way |

Interface strings live in `public/map/locales/<code>.json`, and missing keys
fall back to English, so a partial translation is usable. Adding a language is
a new file plus an entry in `LANGUAGES` in `public/map/i18n.js` — and, because
the tray menu exists before any WebView does, the matching entry in `LANGUAGES`
in `src-tauri/src/lib.rs` and the two `include_str!` sites beside it. The panel
would work without the Rust side; the tray would not list the language.

## Scope

Private project for a couple of people, published in case it is useful. It reads
what the game already shows its own client and draws a map from it. It does not
modify inventory, health, crafting, combat or server state.

One deliberate exception, and it asks first: the POI photography sweep moves
your character and turns on god mode for its duration, restoring both
afterwards. Nothing else touches gameplay.

Game files are never redistributed. The patch is exactly reversible, and the
pristine copies it restores from are kept out of the repository.
