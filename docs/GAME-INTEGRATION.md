# Game integration

How ScrapMap gets data out of Scrap Mechanic, and what it costs.

Everything here runs through the game's own Lua API and its log file. There is
no Cheat Engine, no process-memory access and no graphics hook.

## Prerequisites

The game must be launched **through Steam with `-dev`**. Without that flag the
game ignores edited Lua entirely, which for a long time looked like "loose Lua
does not work". Running `ScrapMechanic.exe` directly fails with
`SteamAPI_init failed`.

```bash
node tools/game-patch.mjs snapshot   # once, on a clean install
node tools/game-patch.mjs apply
```

`snapshot` records the untouched stock files as the restore baseline; `apply`
installs the addons and appends one marked block to each stock file. `revert`
puts the game back exactly.

## Multiplayer: script checksums

**Scrap Mechanic verifies Lua script checksums when a client joins a server.**
The host sends `m_serverGameInfo.m_vecFileChecksums`, the client compares its
own files, and any mismatch is refused with `Invalid checksum`.

So a patched client cannot join an unpatched host. Every player in the session
must run byte-identical files — which is workable (both run `apply` from the
same repository revision and compare the hashes `status` prints) but is a hard
constraint on anything that touches game scripts.

The map itself does not need the patch once the atlas is built. You can
`revert`, join anyone, and still have the full map; only live player positions
depend on the patch.

See `MULTIPLAYER-TEST.md` for the second player's instructions.

## Live telemetry

`ScrapMapTelemetry.lua` wraps `SurvivalGame.client_onUpdate` and, four times a
second, writes one line per visible player into the game log:

```
SCRAPMAP_TELEMETRY_V1|<playerId>|<isLocal>|<name>|<worldId>|x|y|z|dx|dy|dz
```

`sm.log.info` is used rather than `print` — `print` does not reach the log file.
`src-tauri/src/game_log_source.rs` tails the newest log, parses these lines and
evicts stale remote players.

Enumeration is client-side (`sm.player.getAllPlayers`), so it reports what this
client can see. Remote players outside streaming distance simply do not appear.

## World layout

`ScrapMapLayoutExport.lua` runs after the world loads and writes
`<game>/Survival/ScrapMapLayout.json`: every cell with its tile UUID, path,
size, terrain type, rotation, offsets, road mask, flags and POI code. That is
the map's skeleton — roughly 16k cells and a few megabytes.

Rust reads it through the `game_layout_snapshot` command, cached on
path/mtime/length so an unchanged file is not re-sent every second.

## The tile atlas

`ScrapMapAtlasBake.lua` samples **every tile the survival generator knows
about** — not just the ones in your world — through `sm.terrainTile`, and writes
one JSON per tile UUID into `<game>/Survival/ScrapMapAtlas/`.

Tiles are sampled **unrotated and without world-space effects**, so the result
depends only on the game build. The same atlas is valid for every world and for
both players, and it is a one-time cost: about twenty seconds on the first world
load, then nothing. Progress lives in game storage, keyed by a signature that
includes the version and the sampling resolutions.

What is sampled, and why each one is needed:

| Layer | API | Resolution | Purpose |
|---|---|---|---|
| Material | `getMaterialAt` | 64 / cell | The visible surface. Carries roads. |
| Ground cover | `getClutterIdxAt` | 32 / cell | Meadow vs forest floor vs burnt ground |
| Colour | `getColorAt` | 32 / cell | A tint over the material, not a colour |
| Height | `getHeightAt` | 32 / cell | Hillshading and water |
| Objects | `getAssetsForCell`, `getHarvestablesForCell` | per cell | Buildings, ships, trees, boulders |

Three things about this are easy to get wrong:

**`getColorAt` is a tint, not a colour.** Sampling it alone produces near-white
tiles. The visible surface comes from `getMaterialAt`, collapsed into
grass/sand/dirt/rock exactly as the game does in `GetEffectMaterialAt`.

**Clutter is addressed in half-metres**, `0..128` for a 64 m cell, because the
game passes `CELL_SIZE * 2 - 1` as its wrap limit. Everything else uses metres.

**Objects come from two different calls.** `getAssetsForCell` returns set pieces
— buildings, the crashed ship, giant trees, rock formations. Ordinary forest is
*harvestables*, via `getHarvestablesForCell`. `Forest.assetset` contains only
giant-tree parts, so looking there for woodland finds nothing. Clutter is only
small ground cover.

Rust decodes the rasters, shades them and writes
`%LOCALAPPDATA%\ScrapMap\atlas\tiles\generated\<uuid>.png`, then points the
manifest's `topDownRelativePath` at them. Because the raw samples are kept,
palette, hillshading and the water threshold can all be retuned with a rebuild
and no re-bake.

### Describing objects

`getAssetsForCell` yields a UUID and a position, which is not enough to draw
anything. The game ships databases that fill in the rest:

- `Terrain/Database/AssetSets/*.assetset` — name, default colours, collision mesh
- `Harvestables/Database/HarvestableSets/*.harvestableset` — name, colour list
- `Terrain/Database/clutter.json` — 114 ground-cover types, indexed by the
  integer `getClutterIdxAt` returns

Asset sets classify poorly: their own `Type/...` categories cover barely half of
them, and Forest, Building and Spaceship have none, so classification keys off
the set file name. Harvestable sets are cleaner — `trees`, `burntforest`,
`stones`, `ore` are named for what they hold.

Collision meshes give a footprint, but **they are not in metres**. Calibrated
against objects of known size a warehouse lands near 28 m across and a giant
tree near 16 m, which reads as decimetres. That is a plausibility fit, not a
documented unit: `MESH_UNITS_PER_METRE` is the first thing to adjust if objects
come out consistently mis-sized. Measure the bounding box rather than the
distance from the origin — asset meshes are not centred on their pivot.

Note that `farming.harvestableset` contains `//` comments. Strict JSON parsing
drops the whole file silently; use `asset_catalogue::parse_game_json`.

## POI photography

The procedural atlas samples terrain, so it cannot show the crashed ship, a
warehouse or a ruin. Those need a real picture.

`ScrapMapPoiShoot.lua` drives the game's own camera — `sm.camera.setPosition`,
`setDirection`, `setFov`, plus `sm.gui.hideGui` — over each POI in turn, holding
each pose while ScrapMap captures the window with `PrintWindow`. Capture needs
no hook: plain `BitBlt` returns pure black because the window is
DirectX-presented, but `PrintWindow` with `PW_RENDERFULLCONTENT` returns a real
frame. `cargo run --example capture_probe` re-checks that in one command.

The handshake is one-way by necessity. `sm.json.fileExists` cannot see files
written during the same session, so Lua can neither be signalled mid-sweep nor
poll for an acknowledgement. Instead Lua announces each pose in the log and
holds it for a fixed dwell; ScrapMap captures inside that window. For the same
reason the request file is picked up on the next world load rather than the
running one.

### The camera is not enough — the player has to travel

**Terrain streams around the player, not the camera.** A camera-only sweep
photographs sky wherever the player is not; in a 116-target run roughly half the
frames came back as skybox, and they are not detectable by brightness, because
sky is bright and full of contrast.

So the sweep moves the player as well, using the game's own travel path rather
than anything invented:

```lua
sm.event.sendToGame( "sv_e_recreatePlayerInWorld",
    { player = player, world = world, pos = pos, dir = dir } )
```

`SurvivalGame.sv_e_recreatePlayerInWorld` calls
`world:loadCell( x, y, player, "sv_recreatePlayerCharacter", … )` — it **loads
the destination cell first** and recreates the character in the load callback.
That is precisely the guarantee a camera cannot give itself. It is the same path
warehouse ejection and the underground elevators use.

No shipped script calls it, so its parameter contract is read off its body: a
`pos` and a normalised `dir` (`sv_recreatePlayerCharacter` takes
`math.asin( dir.z )`, so an unnormalised vector is a silent NaN), a `world`, and
the `player`. `fadeFromBlack` is deliberately left unset — a fade would be in
the photograph.

Only the server may recreate a character, so the client half asks for each move
over `network:sendToServer( "sv_scrapMapShootTravel", … )`.

**Two hops per target.** The first drops the player in at 400 m above the tile,
which forces the cell to load and lets its neighbours stream. Only then does a
downward raycast have anything to hit, and the second hop parks the player
150 m **above the camera**. Recreating the character resets its velocity, so the
fall never accumulates and never lands.

Above the camera is the whole trick. The camera looks straight down, so anything
higher than it is out of frame regardless of how the capture is cropped, and a
player that high is out of reach of anything that would like to kill it. The
first version stood the player on the ground at the tile's centre, which put a
falling character in the middle of the picture and dropped it into a warehouse
full of robots. `setVisible( false )` did not save it.

That the perch works at all rests on an observation from that run: the ground
probes were hitting real terrain while the player was still at 400 m, so
streaming does not need the player on the ground — only nearby in plan.

Details that each cost a debugging round:

- The ground probe is filtered to `sm.physics.filter.terrainSurface`. Unfiltered,
  a cast from 800 m down hits the falling character first and frames the shot
  around it.
- The pose is not announced until the character is measurably above the camera,
  with a timeout. Waiting a fixed time instead let a slow second hop expose the
  frame while the character was still falling through it.
- A recreated character brings back the HUD, the player's controls and the
  default camera. The pose is re-applied whenever the character identity
  changes, not once at the start.
- The sweep refuses to run when `sm.isHost` is `false`. Teleporting yourself
  around someone else's server for ten minutes is not a neighbourly default.
- Arrival is detected by the character's horizontal distance to the target, with
  a timeout so one stuck target cannot stall the sweep.
- At the end the player is returned to where the sweep found them.

Roughly seven seconds per target, so a 116-target world takes about fifteen
minutes. The player spends all of it airborne.

Tile sizes in the test world run 1, 2, 4 and 8 cells (68 / 28 / 16 / 4 of them).
An 8-cell tile is 512 m across and the camera sits 443 m up, which asks for more
ground than streams around a standing player; expect the four largest tiles to
be the ones that come back partly sky.

Everything downstream is unchanged and tested: target selection, the one-way
handshake, window capture, cropping, rescaling and manifest precedence.

## Reference

| Path | Contents |
|---|---|
| `<game>/Survival/ScrapMapLayout.json` | World layout, written each load |
| `<game>/Survival/ScrapMapAtlas/<uuid>.json` | Per-tile samples |
| `<game>/Survival/ScrapMapCapture.json` | Photography request |
| `<game>/Logs/game-*.log` | Telemetry and sweep progress |
| `%LOCALAPPDATA%\ScrapMap\atlas\` | Manifest, generated tiles, photos |

Log prefixes, all greppable: `SCRAPMAP_TELEMETRY_V1|`, `SCRAPMAP_LAYOUT_V1|`,
`SCRAPMAP_ATLAS_V1|`, `SCRAPMAP_SHOT_V1|`.

```bash
grep -E "SCRAPMAP_ATLAS_V1.(begin|done|fail)" "$(ls -t '<game>/Logs/'*.log | head -1)"
```

A complete bake reports `done|baked=493|failed=0|pending=0`.
