# Game patch

Lua installed into the Scrap Mechanic directory. These files are the canonical
copies; `tools/game-patch.mjs` installs them and can put the game back.

```
Survival/Scripts/game/ScrapMapTelemetry.lua     player positions -> game log
Survival/Scripts/game/ScrapMapPoiShoot.lua      camera sweep for POI photography
Survival/Scripts/terrain/ScrapMapLayoutExport.lua   world layout -> JSON
Survival/Scripts/terrain/ScrapMapAtlasBake.lua      terrain sampling -> per-tile JSON
vanilla/                                        restore baseline, gitignored
```

## The patch must stay exactly reversible

The game compares Lua script checksums with the host when joining a server, so
a patched client cannot join an unpatched host and vice versa. Two consequences
shape everything here:

**Additive only.** Stock files are never edited in place. They get one appended
block between `-- SCRAPMAP ADDON BEGIN` / `-- SCRAPMAP ADDON END`, rebuilt from
the vanilla baseline each time `apply` runs, so applying twice is idempotent and
reverting is exact. When a stock function needs different behaviour, wrap it at
the end of the file rather than editing its body — that is why
`terrain_overworld.lua` gets a `Load()` wrapper instead of an inline call.

**Byte-identical across machines.** `apply` normalises line endings when
installing, because `core.autocrlf` on a fresh clone rewrites `.lua` files and a
single changed byte is enough to be refused. `.gitattributes` pins these to LF.
`status` prints hashes so two players can compare.

`vanilla/` holds verbatim copies of Axolot's shipped scripts. It is deliberately
gitignored — it is their source, not ours — and is recreated locally with
`snapshot` after a Steam file verification.

## Lua quirks that have bitten this code

**Globals persist across world loads.** Each addon guards with
`g_scrapMap...Installed` so re-execution is a no-op. The cost is that editing an
addon requires a full game restart; reloading the world re-runs the file but the
old closure stays installed.

**`sm.json.fileExists` cannot see files written during the same session.** Do
not use it to detect a request file or to check whether output already exists.
`ScrapMapAtlasBake.lua` tracks progress in
`sm.terrainGeneration.loadGameStorage`/`saveGameStorage`; `ScrapMapPoiShoot.lua`
simply attempts `sm.json.open` and treats failure as "nothing to do".

**`sm.json.save` writes floats and nulls.** Every number is a double and an
empty table becomes `null`. The Rust side tolerates both; keep it that way.

**`print()` does not reach the game log — `sm.log.info` does.** All the
machine-readable output uses `sm.log.*` with a `SCRAPMAP_..._V1|` prefix so it
can be tailed and parsed.

## Sampling conventions

The terrain API addresses tiles, not the world, and the units are not uniform:

- `getMaterialAt` / `getColorAt` / `getHeightAt` take cell-local metres, `0..64`.
- `getClutterIdxAt` takes **half-metres**, `0..128` — the game passes
  `CELL_SIZE * 2 - 1` as its wrap limit. Getting this wrong silently repeats a
  quarter of the tile four times.
- `getColorAt` returns a **tint over the material textures**, not the visible
  colour. Sampling it alone yields near-white tiles; the surface comes from
  `getMaterialAt`, collapsed the same way the game does in `GetEffectMaterialAt`.
- Tiles are sampled **unrotated**. The renderer rotates per placed cell, so the
  atlas depends only on the game build and is valid for every world.
- Tile sizes run 1×1 to **16×16** cells.

Objects live in three different places, which is not obvious:

| What | Where it comes from |
|---|---|
| Buildings, the crashed ship, giant trees, rock formations | `getAssetsForCell` |
| Ordinary forest, boulders, ore | `getHarvestablesForCell` |
| Grass tufts, burnt stubble, pebbles | `getClutterIdxAt` |

`Forest.assetset` contains only giant-tree parts, so looking there for woodland
finds nothing.

## Bumping the bake

`SCRAPMAP_ATLAS_VERSION` feeds the storage signature. Raise it whenever the
sampled data changes shape and every tile re-bakes on the next world load;
resolution constants are part of the signature too.
