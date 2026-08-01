import assert from "node:assert/strict";
import test from "node:test";

import {
  createDemoFixtureV1,
  toLegacyRendererBundle,
} from "../node_modules/.cache/scrapmap-test-build/fixtures/index.js";

await import("../public/map/map-core.js");

const core = globalThis.SMMapCore;

test("the versioned fixture remains compatible with the legacy renderer", () => {
  assert.ok(core, "map-core.js must expose SMMapCore");

  const fixture = createDemoFixtureV1();
  const bundle = toLegacyRendererBundle(fixture);
  const layout = core.normalizeLayout(bundle.layout);
  const telemetry = core.normalizeTelemetry(bundle.telemetry);
  const visited = core.normalizeVisited(bundle.visited);
  const markers = core.normalizeMarkers(bundle.markers);

  assert.equal(core.classifyPayload(bundle, "demo-bundle.json"), "bundle");
  assert.equal(layout.worldId, fixture.identity.worldFingerprint);
  assert.equal(layout.cells.length, fixture.layout.cells.length);
  assert.equal(layout.warnings.length, 0);
  const poiCatalog = core.buildPoiCatalog(layout);
  assert.equal(poiCatalog.length, 6);
  assert.equal(
    core.searchPoiCatalog(poiCatalog, "склад")[0]?.id,
    "poi:demo-poi-warehouse",
  );
  assert.equal(
    core.searchPoiCatalog(poiCatalog, "warehouse")[0]?.name,
    "Warehouse",
  );
  assert.equal(telemetry.player.id, fixture.telemetry.localPlayerId);
  assert.equal(telemetry.players.length, fixture.telemetry.players.length);
  assert.equal(visited.keys.size, fixture.fogDelta.revealedCells.length);
  assert.equal(markers.markers.length, 1);
  assert.equal(markers.markers[0].id, fixture.marker.id);

  const newlyRevealed = core.newlyRevealedCells(telemetry.players, {
    cellSize: layout.cellSize,
    radius: 2,
    validKeys: layout.cellsByKey,
    visitedKeys: new Set(),
  });
  assert.ok(newlyRevealed.length > 0);
  assert.ok(
    newlyRevealed.every((cell) => layout.cellsByKey.has(cell.key)),
  );
});
