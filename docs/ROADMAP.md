# Roadmap

## Where things stand

Working: the portable overlay attached to the game window, compact and full map
with global shortcuts, SQLite profiles with world and server isolation, fog of
war with a reveal-all control, local markers, POI categories with per-category
icons, search and filters, live local-player telemetry, and a procedurally
generated tile atlas covering all 493 tiles with terrain, ground cover, water,
hillshading, buildings and forest.

Not working: shared fog and markers. POI photography is implemented but has not
yet been run end to end in the game.

## Done

**M0–M2** — repository and contracts, the Tauri overlay, world/server profiles
in SQLite. The identity and quarantine rules in `ARCHITECTURE.md` are
implemented and tested.

**M3, CE-free telemetry** — not as originally planned. The plan was a native
bridge reading replicated players out of process memory behind signature and
prologue checks. What actually shipped is a Lua patch that logs player positions
and a log tail that parses them: no memory access, no hook, and far less to go
wrong on a game update.

The constraint that discovery brought with it: **Scrap Mechanic verifies Lua
script checksums on connect**, so both players need byte-identical patched files
or the client is refused. `tools/game-patch.mjs` exists to make that exact and
reversible.

**M4, the tile atlas** — also not as planned. The original idea was to index the
game's 807 isometric preview PNGs, and later to import a third-party screenshot
atlas. Both were abandoned: the previews are isometric and unusable as a
top-down map, and the imported set covered 43% of tiles, was six years stale and
carried a CC BY-NC-SA obligation.

Instead the atlas is sampled from the game's own terrain API — 493/493 tiles,
no licensing constraint, and valid for every world on the same game build.

**M5, POI catalogue** — categories, filters, search and distinct per-category
icons. Generator filler (`POI_*_RANDOM*`, over six hundred cells) is sorted into
its own category and off by default.

## Next

### POI photography

The blocker was real — terrain streams around the player, not the camera, so a
camera-only sweep photographed sky wherever the player was not, and about half a
116-target run came back as skybox.

The way out was the game's own teleport. `SurvivalGame.sv_e_recreatePlayerInWorld`
loads the destination cell and *then* recreates the character in the load
callback, so the sweep now moves the player and the camera together. Two hops
per target: in high to force the cell to load, then down onto the measured
ground for the exposure. `GAME-INTEGRATION.md` has the details.

**Not yet verified in the game.** What remains is to run it: check the hit rate,
confirm the player is returned to their starting position, and see whether two
seconds standing still per target is survivable in a populated world.

If the sweep turns out to be too disruptive, opportunistic capture during normal
play remains the cheap alternative — notice when the player happens to be near
an un-photographed POI and capture then, with no teleporting and no locked
controls. It only covers places actually visited.

### Shared fog and markers

`SYNC.md` has the design. Nothing is implemented.

### Navigation

Markers exist. Target bearing, breadcrumbs, route drawing, A* and breadcrumb
simplification remain.

### Housekeeping

- Decide the fate of the dead code listed in `ARCHITECTURE.md`, particularly
  `game_build.rs` — a stale unwired compatibility gate is worse than either
  wiring it up or deleting it.
- The compact map can be moved by corner and size; direct drag-to-move and
  drag-to-resize while holding a modifier would be better, and is deferred.
- `MESH_UNITS_PER_METRE` is a plausibility fit, not a documented unit. If
  objects render consistently mis-sized, that is the dial.
- No tray icon. Still absent, still arguably wanted.

## Out of scope

- Host-side workshop mods.
- Any change to inventory, crafting, health, movement or server state.
- A DirectX hook for exclusive fullscreen. Windowed mode is sufficient.
- Redistributing the game's art or scripts.
- End-to-end encryption, public accounts, or a multi-party backend.
