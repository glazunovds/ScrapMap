# Architecture

One portable EXE. A Tauri 2 host in Rust owns the window, the database and the
filesystem; a single WebView draws the map. The compact minimap and the full map
are two states of the same window, not two windows.

Local-first: everything works with no network. Shared fog and markers are an
optional layer on top (see `SYNC.md`), never a dependency.

## Why it is shaped this way

**Tauri rather than Electron** — one portable EXE against the system WebView2,
no bundled runtime, no localhost server, no listening port.

**Canvas 2D rather than a map library** — the world is a fixed grid of 64 m
cells with a per-tile bitmap. Tiles and cells are drawn into a cached static
frame and only players are redrawn live, so a cell costs one `drawImage`
regardless of how much is on it. Per-cell DOM nodes were the original cause of
the slowness at large scales.

**Rust owns SQLite** — the WebView gets narrow commands, never SQL and never a
path it could traverse. This boundary keeps a rendering bug from becoming a
data-loss bug.

## Processes and threads

Two polling loops in the host:

- **Window tracker**, 200 ms, backing off to 1 s while the game is absent.
  Finds the top-level `ScrapMechanic.exe` window and applies overlay geometry
  and visibility. The image name is checked with
  `PROCESS_QUERY_LIMITED_INFORMATION`; process memory is never opened.
- **Diagnostic source**, 100 ms. Tails the game log for telemetry, and watches
  for POI photography cues.

Geometry, visibility, focusability and click-through are serialised under one
transition lock. The user's hide preference is tracked separately from actual
visibility, so an automatic hide never silently undoes `Ctrl+Shift+H`. In full
mode the overlay's own window counts as foreground, otherwise clicking the map
would dismiss it.

Shortcuts: `Ctrl+Shift+M` compact/full, `Ctrl+Shift+H` hide, `Ctrl+Shift+Q`
quit, `Escape` returns the full map to compact.

## Identity

Two derived identifiers decide where data is written. Both are stable across
sessions and reveal nothing about the machine.

**World fingerprint** — `smwf1_<64 hex>`, SHA-256 over canonical JSON with the
domain `scrapmap-world-v1`: cell size, bounds, and cells sorted by
`(x, y, tileUuid)` with UUIDs lowercased. Only structure is included — cell
coordinates, tile UUID, rotation, offsets and stable flags. Local paths, the
game's own `worldId`, POI display names, game mode, timestamps and session
numbers are excluded, so the same world fingerprints identically for both
players. Game mode is mutable profile metadata rather than part of the hash, so
detecting it late does not fork a new world.

**Profile key** — `smp1_<64 hex>`, SHA-256 of
`["scrapmap-profile-v1", scopeKind, scopeId, worldFingerprint]`. Scope is
`local`, a stable `server`, or an explicit `fallback`. A transient Steam lobby
or session ID is deliberately *not* a server identity.

### Recognising a peer-hosted world

The window tracker gives the game's PID. A bounded reader takes the newest
`Logs/game-YYYYMMDD-HHMMSS.log`, confirms the PID in its header, and reads the
last complete `Connecting to ...` line. `Connecting to self` means a local
world; a SteamID64 becomes
`steam-sha256:SHA-256("scrapmap-peer-host-v1\0" + steamId64)`, so the raw ID is
never stored. Each connection also produces a separate `connection-sha256:...`
observation, accepted only after a repeat observation consistent with live
`isHost`.

A PID mismatch, an unfamiliar format, a partial write, a file over 4 MiB or any
error yields `unknown` — and `unknown` must never auto-select a previously
chosen manual profile.

### Quarantine

A profile identified only by fingerprint is read-only quarantine. Manual
profiles are offered only for that fingerprint; choosing one reactivates with a
fresh `sessionId` and an opaque `manual:<UUID v4>`. Fog discovered before the
choice is kept in memory and migrates to the chosen profile; staged markers and
settings do not. Ambiguity is re-confirmed on every new connection.

## Storage

`%LOCALAPPDATA%\ScrapMap\scrapmap.sqlite3`, migrated on open. Tables cover world
profiles and layouts, settings, fog cells, markers, routes, trails and the last
active app state.

Writes are gated on the triple `profileKey + worldFingerprint + sessionId`, so a
frame arriving during a profile switch cannot land in the wrong world. Trail
batches must be sequential; an identical repeat is idempotent, a stale session
or a conflicting repeat is rejected. The recent-trail query returns at most 4096
points plus the full count and a `truncated` flag.

Telemetry carries a sequence number, gated to strictly increasing within the
current activation. Missing, zero, repeated or smaller frames move nothing and
reveal no fog.

Commands exposed to the WebView: the nine `profile_*` commands, plus
`game_layout_snapshot`, `atlas_manifest`, `atlas_preview`, `atlas_bake_refresh`,
`poi_capture_prepare`, `server_identity_probe`, `set_overlay_mode`,
`set_mini_overlay_layout`, `overlay_status`, `diagnostic_snapshot` and
`diagnostic_status`.

## Game data

Covered in detail in `GAME-INTEGRATION.md`. In outline: a Lua patch writes the
world layout and per-tile terrain samples to disk and player positions to the
game log; Rust converts the samples into tile PNGs and serves them to the
WebView as data URLs. Tile images are files in the cache directory, never BLOBs
in SQLite.

The atlas is a function of the game build rather than the world, so it is built
once and reused across worlds and players.

## Security posture

- No Cheat Engine, no process-memory writes, no DirectX hook.
- Window tracking uses Win32 window information and
  `PROCESS_QUERY_LIMITED_INFORMATION` only.
- Peer-host recognition reads at most 4 MiB of the current game log. Raw
  SteamID64s, filesystem paths and log lines never reach the WebView or SQLite.
- Atlas previews are served only from the cache directory, only for PNG/JPEG,
  with path traversal rejected, and only up to 4 MiB per file.
- Window capture uses `PrintWindow` on a window already tracked — no hook and no
  injection — and only when the user has asked for a photography sweep.
- Read-only with respect to gameplay: inventory, health, crafting and
  server-controlled state are never touched.

## Packaging

One portable `scrapmap.exe` against the system Evergreen WebView2. Runtime data
in `%LOCALAPPDATA%\ScrapMap`. No MSI/NSIS, no service, no auto-update, no
listening port in local mode.

Build with `pnpm tauri build --no-bundle`. A plain `cargo build` produces an EXE
that looks for a dev server instead of the bundled frontend.

## Known dead code

Recorded rather than quietly deleted, because deciding their fate is a real
decision:

- **`diagnostic_source.rs`** (~1000 lines) reads a JSON telemetry file. Nothing
  in the repository writes that file — live telemetry comes from the game log
  instead. It remains wired as a fallback behind the log source. See
  `DIAGNOSTIC-FEED.md`.
- **`native_process.rs`** — only `game_log_directory` is used. The
  `NativeProcessReader` machinery and the `native_transport_probe` command are
  unreachable from the UI.
- **`game_build.rs`** — a SHA-256 allowlist that gates nothing, and whose only
  entry is one game version out of date. Either wire it up as the compatibility
  gate it was meant to be, or remove it; a stale unwired safety check is the
  worst of the three states.
- `rectifiedAtlasPreview` in `public/map/app.js` — an unreachable branch left
  from the abandoned isometric-preview approach.
- `tools/tile-atlas/import-sm-overview.mjs` — imported a third-party
  CC BY-NC-SA screenshot atlas. Superseded by the procedural bake, which covers
  all 493 tiles with no licensing constraint.
- `src/data-sources/`, `src/sync/`, `src/main.ts` — interface declarations and
  an empty entry point. See `public/map/CLAUDE.md`.
