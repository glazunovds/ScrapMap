# Handoff — POI photography

State of the POI photography sweep, what was wrong with the last run, and what
is still open. Written 2026-08-02.

## Where it stands

The sweep works end to end: 116 of 116 tiles photographed and published. Four
separate faults have been found and fixed since; **none of the fixes have been
run in the game yet.** The next sweep is the test.

Everything else about the map is unaffected — this document is only about the
photographs.

## What the last run actually produced

116 photographs, of which the user judged roughly 70% good. Measuring them
rather than guessing turned up three distinct causes, and only one of them was
what it looked like.

**1. Cinematic mode cropped every photograph.** `sm.render.setCinematic( true )`
draws letterbox bars. All 116 photographs have ~51 black rows at the top and
another ~51 at the bottom of a 512-pixel image: **20% of every tile was lost**,
top and bottom. This is the "POI cut from the top / cut from the bottom".
Cinematic mode is now off; `sm.gui.hideGui` was already doing the useful part.

**2. The in-game probes never worked at all.** `sm.physics.raycast` returns a
RaycastResult **userdata**, not a table, so the `type( result ) == "table"` guard
was always false. Consequences, invisible in the log except as `miss`:

- every shot was framed from sea level rather than the tile's own ground;
- the pull-back that stops a tall building leaning out over the tile's edges
  **never engaged once** — `structure` is `0.0` on all 116 log lines.

The second one is the "towers too close". The first one mattered less than it
sounds: this world's POI tiles all sit within ~12 m of sea level, so it
mis-framed exactly 1 of 116. It would matter a lot in a hillier world.

Both measurements now come from data ScrapMap already has, with no physics
involved:

| Quantity | Source |
|---|---|
| Ground height | median of the tile's baked height raster |
| Terrain relief | 95th percentile minus the median |
| Structure height | vertical extent of each asset's collision mesh |

The heights were checked against telemetry before being trusted: with the player
on foot, its reported height falls inside its own tile's sampled band 79,481
times out of 79,540, and every exception is *above* the band — a player standing
on a base, not a shifted origin. 15 of the 116 tiles need a pull-back, the
tallest asset being a 16.8 m builder-quest structure.

**3. 62 of 116 photographs were taken at a rotated placement.** Cells carry a
`rotation` of 0–3 and the renderer turns each tile image by it. The generated
atlas samples tiles *unrotated*, so that is right for them — but a photograph is
of a real placement and already carries the rotation, so it gets turned twice.
`build_targets` now prefers an unrotated placement. **33 tiles have no unrotated
placement anywhere in the world** and remain wrong; see open questions.

## Not a fault: a tile is not a POI

Several of the images that looked wrong are correct. A photograph is keyed to a
**tile**, and the game builds a large point of interest out of several tiles. A
tile that is the corner of a warehouse must photograph as the corner of a
warehouse; its neighbours photograph their own parts and the map assembles them.

Capturing "the whole object" in one frame is not possible: the image has to map
exactly onto the tile's footprint or it will not line up, and the same tile is
reused elsewhere in the world where the rest of the building is not there.

So "corner of the building", "part of the acid pool" and "cut POI" from the
*sides* are all expected. Only cut from the top and bottom was a bug.

## The first run with the fixes: 49 of 116, then the player died

Confirmed working from the log — `probed` reads `atlas` throughout, `covered`
exceeds `metres` where a structure was found (77.40 against 64.00, a 1.21×
pull-back, structure 16.8 m), and clearance sits at a steady 141 m so the perch
holds. No `travel refused`, no `slow`.

The death itself is not visible in the log and its cause is unknown. What the
run did expose is that dying was expensive: photographs were only published when
Lua reported `done`, and the request was only cleared then too, so an interrupted
sweep put nothing on the map and started again from the first tile.

Both are fixed. Each photograph is published as it is taken, and
`remaining_targets` drops targets already photographed since the outstanding
request was written — so pressing prepare again after an interruption resumes
instead of restarting. A completed sweep is unaffected, because by then the
request is gone and every target is offered afresh.

A sweep still leaves the player helpless for fifteen minutes with locked
controls. If deaths keep happening, that is the thing to attack: either shorten
the exposure or accept `/godmode` as a documented precondition.

## Open questions

**The 33 tiles with no unrotated placement.** The fix is to rotate the captured
image by the inverse before storing, so every photograph ends up in the same
unrotated convention as the generated tiles. What is not established is the
direction: the mapping from world axes to screen axes with the camera pointing
straight down has never been observed. Do not guess it — photograph one tile
with a known asymmetric layout, compare against its generated tile, and read the
direction off that.

**18 of 116 photographs are night.** Mean brightness under 60 against a median
of 106. The sweep takes about fifteen minutes and runs through the day/night
cycle. `SurvivalGame.sv_setTimeOfDay` and `sv_setTimeProgress` could pin it to
midday and restore afterwards. **This is server-controlled state**, which the
project otherwise leaves alone, so it needs the user's agreement rather than
being done quietly.

**2–4 white frames.** Not diagnosed. The `MIN_LIT_FRACTION` / `MIN_DETAIL`
guards in `poi_capture.rs` are meant to reject these; either they are too loose
or those tiles fail some other way.

## How to run it

Rust changed, so the EXE was rebuilt. The Lua changed, so the game needs a
**full restart** — a world reload re-runs the file but leaves the old closure
installed.

1. `node tools/game-patch.mjs apply` (already done on this machine)
2. Launch through Steam with `-dev`, load the survival world
3. Start `scrapmap.exe` and trigger the sweep; it reports `targets`,
   `withoutGroundHeight` and `rotatedPlacements`
4. Reload the world — the sweep runs on the next load, ~15 minutes

Reading the result:

```bash
grep -o "SCRAPMAP_SHOT_V1|ready|.*" "$(ls -t '<game>/Logs/'*.log | head -1)"
```

Fields are `uuid|x|y|size|metres|covered|height|probed|clearance|structure`.
`covered > metres` means the camera pulled back and ScrapMap cropped the
surplus. `probed` should now read `atlas`, never `sealevel`.

## What to check on the next run

- No black bands. Any at all means cinematic mode came back.
- The 15 pull-back tiles: towers standing up rather than leaning outward.
- Whether photographs line up with their neighbours on the map — that is the
  rotation question, and it is the one most likely to still be wrong.
