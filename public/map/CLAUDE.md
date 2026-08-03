# Renderer

This is the shipping frontend. Plain JavaScript, no build step, no framework —
Vite copies `public/` verbatim.

```
index.html          markup and the side panel
app.js              state, canvas rendering, POI icons, fog
map-core.js         pure helpers: layout normalisation, POI categories, geometry
overlay-bridge.js   everything that talks to Tauri
styles.css          panel and map styling
overlay.css         compact-mode chrome
```

## `src/` is not this

The repository also has a TypeScript layer under `src/` — `domain/`,
`data-sources/`, `sync/`, `fixtures/`. **None of it runs.** `src/main.ts` is an
empty stub and the root `index.html` redirects to `public/map/`. It exists so
`pnpm test` can typecheck contracts against fixtures, and `sync/` and
`data-sources/` are interface declarations with no implementation.

So: changing behaviour means changing `public/map/`. Changing `src/` changes
only what the tests check. Do not assume a type there constrains the renderer.

## Load order matters

`app.js` loads before `overlay-bridge.js`. Anything `app.js` dispatches at
startup fires into a listener that does not exist yet — this silently broke the
saved minimap placement. Defer startup dispatches to the `load` event.

## Rendering

One canvas, two layers. Terrain, POIs, markers and bounds are drawn into an
offscreen static frame, keyed on
`{revision, atlasRevision, worldId, expanded, size, camera, scale}`; only
players are redrawn live. Invalidate with `invalidateStaticFrame()` after any
change that alters the static content.

There are no per-cell DOM nodes and there must not be. Tile imagery is baked
per tile by the Rust side, so a cell costs one `drawImage` regardless of how
many trees are on it.

**A tile image covers its whole tile, which may span up to 16×16 cells.**
`drawAtlasTile` takes the slice for this cell using `xOffset`/`yOffset`; drawing
the whole image per cell reproduces the entire tile in each of its cells. Tile
size comes from the atlas entry rather than the layout cell, because the layout
persisted to SQLite drops it.

Row 0 of a tile image is north. Rotation is applied per cell as
`((4 - rotation) % 4) * π/2`, and it is applied in two places — keep them in
agreement.

## POIs

Categories live in `poiCategories` and silhouettes in `poiIcons`, both in
`app.js`; the filter list draws the same shapes through `poiIconSwatch`, so the
panel is the legend. Add a category in both places or it will fall back to
`landmark`.

The generator marks every random lake and roadside patch as a point of
interest — over six hundred of them. `map-core.js` sorts anything named
`RANDOM` into the `filler` category, which is off by default.

**Only the tile's origin cell carries the POI.** `normalizeLayout` clears it
wherever `xOffset`/`yOffset` is non-zero, so a warehouse on a 4×4 tile answers
`cell.poi` in one corner and `null` in the other fifteen. Anything asking what
a cell belongs to — hover, search, a future click target — goes through
`cellPoi`, which walks back to the origin and then to the group. Reading
`cell.poi` directly is right only for drawing, where one icon per tile is what
you want.

## Talking to the host

All `invoke` calls live in `overlay-bridge.js`. The renderer raises DOM events
and the bridge translates them, which keeps `app.js` runnable in a browser for
testing.

`dispatchPersistenceEvent` is gated on profile hydration — correct for world
data, wrong for anything about the local window. Native window placement is
dispatched directly for that reason.
