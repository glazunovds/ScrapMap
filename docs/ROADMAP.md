# Roadmap

## Where things stand

Working: the portable overlay attached to the game window, compact and full map
with global shortcuts, SQLite profiles with world and server isolation, fog of
war with a reveal-all control, local markers, POI categories with per-category
icons, search and filters, live local-player telemetry, a procedurally generated
tile atlas covering all 493 tiles with terrain, ground cover, water, hillshading,
buildings and forest, photographs of eleven points of interest taken with the
game's own camera, and a tray icon that installs and reverts the game patch
without a terminal.

Not working: fog and markers do not persist between players.

Live positions and names for every player in a session already work and need
no backend -- the game tells each client where everyone is, and fog reveals
around all of them. What is missing is persistence *between* players: markers
one of you placed while the other was offline. The requirement is settled:
syncing only while both are online is acceptable, provided each machine keeps
its own fog and markers between sessions, which it already does.

That makes the remaining feature much smaller than `SYNC.md` assumes. Passing
markers through the game's own network rather than through Cloudflare is worth
trying first: no account, no service, no listening port. The uncertain part is
getting a marker *into* a running game, since `sm.json.fileExists` cannot see
files written during the session -- a twenty-minute experiment, not a design.

The tile atlas renders at four pixels per sampled metre, and buildings, wrecks
and rock formations are drawn from their own collision-mesh footprints rather
than as tinted discs. POI photography exists and works, but only eleven
photographs survived review — see `HANDOFF.md` for why, and why the generated
tiles turned out to be the better lever.

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

**M6, POI photography** — done, after two false starts.

Terrain streams around the player, not the camera, so a camera-only sweep
photographed sky wherever the player was not: half of a 116-target run came back
as skybox. The way out was the game's own teleport —
`SurvivalGame.sv_e_recreatePlayerInWorld` loads the destination cell and *then*
recreates the character in the load callback, so the player and the camera
travel together.

The second attempt stood the player on the ground at the tile's centre, which
photographed a falling character in the middle of every frame and dropped it
into a warehouse full of robots. The player is now parked above the camera,
where it is both out of frame and out of reach.

The last correction was framing: a tower photographed from a camera barely
higher than itself leans out over the tile's edges, so each tile is measured for
what is standing on it and the camera pulls back proportionally, with ScrapMap
cropping the surplus. `GAME-INTEGRATION.md` has the details.

Reviewed on the map, most photographs were not worth keeping: eleven are
published and the rest are hidden. The conclusion that matters is in
`HANDOFF.md` — the atlas draws the map better than the camera photographs it,
and it needs no game running to do it.

## Next

### Shared markers

`SYNC.md` has a Cloudflare design that now looks heavier than the problem
needs; see above. Nothing is implemented either way.

### Navigation

Markers exist. Target bearing, breadcrumbs, route drawing, A* and breadcrumb
simplification remain.

### Release

The executable carries the game patch and a tray icon, so a released build
needs neither Node nor a checkout. `README.md` has been checked claim by claim
against the code, and two things it promised were wrong: that the map works
without the patch (it does not -- the patch exports the layout), and that a new
language needs no Rust change (it needs three).

What remains before publishing: screenshots of the English interface, a version
tag, and a look at whether the panel still offers things that no longer earn
their place -- the photography button most of all, now that eleven photographs
are kept out of a hundred and sixteen taken.

### Housekeeping

- Decide the fate of the dead code listed in `ARCHITECTURE.md`, particularly
  `game_build.rs` — a stale unwired compatibility gate is worse than either
  wiring it up or deleting it.
- The compact map can be moved by corner and size; direct drag-to-move and
  drag-to-resize while holding a modifier would be better, and is deferred.
- `MESH_UNITS_PER_METRE` is a plausibility fit, not a documented unit. If
  objects render consistently mis-sized, that is the dial.
- Adding a language means editing Rust as well as JSON. A build script that
  emitted the locale table from `public/map/locales/*.json` would make the
  one-file promise true; today the README simply says so.

## Out of scope

- Host-side workshop mods.
- Any change to inventory, crafting, health, movement or server state.
- A DirectX hook for exclusive fullscreen. Windowed mode is sufficient.
- Redistributing the game's art or scripts.
- End-to-end encryption, public accounts, or a multi-party backend.
