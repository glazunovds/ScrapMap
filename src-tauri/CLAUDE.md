# Rust host

Owns everything the WebView must not: the game window, the SQLite database, the
filesystem, and the tile atlas. The WebView gets narrow commands and never SQL
or a path it could traverse.

## Modules

| File | Responsibility | Live? |
|---|---|---|
| `lib.rs` | Commands, overlay state, the two polling threads | yes |
| `game_window.rs` | Finds the game window, computes overlay geometry | yes |
| `storage/mod.rs` | SQLite: profiles, fog, markers, routes, trails | yes |
| `game_log_source.rs` | Tails the game log for telemetry lines | yes |
| `server_identity.rs` | Local vs peer-hosted world, from the game log | yes |
| `atlas_bake.rs` | Baked terrain rasters → tile PNGs + manifest | yes |
| `asset_catalogue.rs` | Game asset/harvestable/clutter databases | yes |
| `window_capture.rs` | `PrintWindow` frame grab, crop, rescale | yes |
| `poi_capture.rs` | POI photography sweep orchestration | yes |
| `tile_atlas.rs` | Serves atlas manifest and images to the WebView | yes |
| `diagnostic_source.rs` | JSON telemetry file reader | **no producer** |
| `native_process.rs` | Only `game_log_directory` is used | mostly dead |
| `game_build.rs` | SHA-256 allowlist, wired to nothing, hash stale | dead |

The last three are documented in `docs/ARCHITECTURE.md` under known dead code.
Do not build on them without deciding their fate first.

## Threads

Two polling loops, both started in `setup()`:

- **Window tracker**, 200 ms (1 s when the game is absent). Applies overlay
  geometry and visibility. Owns `game_process_id` and `game_window_handle` in
  `OverlayState`, which is how other code reaches the game without
  re-discovering it.
- **Diagnostic source**, 100 ms. Tails the game log for telemetry, and watches
  for POI photography cues.

Geometry, visibility, focusability and click-through changes are serialised
under a single transition lock. Keep them there — interleaving them produces a
window that is visible but not clickable, or vice versa.

## Reading the game's data

Parse with `asset_catalogue::parse_game_json`, not `serde_json` directly: some
shipped files are JSONC and strict parsing drops them silently.

Anything written by Lua needs float-tolerant deserialisation — `sm.json.save`
writes `512.0` for `512`, and `null` for an empty table. `lua_number` /
`lua_span` in `atlas_bake.rs` exist for this; serde's `default` does **not**
cover a null, only a missing field.

## The atlas pipeline

```
Lua samples tiles      -> <game>/Survival/ScrapMapAtlas/<uuid>.json
atlas_bake converts    -> %LOCALAPPDATA%\ScrapMap\atlas\tiles\generated\<uuid>.png
poi_capture photographs-> ...\atlas\tiles\photo\<uuid>.png
both publish           -> manifest.json topDownRelativePath
```

The renderer gates purely on `topDownRelativePath` existing, so a photo simply
outranks a generated tile for that UUID and no frontend change is needed.

Conversion is incremental — a PNG newer than its source JSON is skipped — so
`atlas_bake_refresh` is cheap to call repeatedly. To force a re-render, delete
the PNGs rather than editing the manifest. Rendering constants
(`SURFACE_PALETTE`, `HILLSHADE_STRENGTH`, `WATER_LEVEL`, `GroundCover::tint`)
are all tunable without re-baking, since the raw samples are kept.

## Capture

`BitBlt` returns pure black on this window; `PrintWindow` with
`PW_RENDERFULLCONTENT` returns a real frame. `examples/capture_probe.rs` proves
which works and is the quickest way to re-check after a driver or game update.

Validate captures on **contrast**, not brightness. A camera inside a hill or
above the clouds returns an evenly lit, perfectly bright, entirely useless
frame; `Frame::detail` is what distinguishes it from terrain.

## Conventions

Commands return `Result<_, String>` and never leak filesystem paths in error
text — several tests assert this. Keep new state in `OverlayState` as atomics
where a polling thread reads it.
