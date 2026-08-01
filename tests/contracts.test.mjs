import assert from "node:assert/strict";
import test from "node:test";

import {
  FOG_DELTA_SCHEMA_VERSION,
  LAYOUT_SCHEMA_VERSION,
  MARKER_SCHEMA_VERSION,
  PROFILE_SCHEMA_VERSION,
  ROUTE_SCHEMA_VERSION,
  TELEMETRY_SCHEMA_VERSION,
  TRAIL_SCHEMA_VERSION,
  WORLD_IDENTITY_SCHEMA_VERSION,
} from "../node_modules/.cache/scrapmap-test-build/domain/index.js";
import { createDemoFixtureV1 } from "../node_modules/.cache/scrapmap-test-build/fixtures/demo-v1.js";

const cellKey = ({ x, y }) => `${x},${y}`;

test("the demo fixture is deterministic, JSON-safe, and sanitized", () => {
  const first = createDemoFixtureV1();
  const second = createDemoFixtureV1();
  const serialized = JSON.stringify(first);

  assert.equal(serialized, JSON.stringify(second));
  assert.deepEqual(JSON.parse(serialized), first);
  assert.doesNotMatch(
    serialized,
    /(?:[a-z]:[\\/]|\\\\|\/users\/|steamlibrary|bearer\s+|smap_v1_|\.png")/i,
  );
  assert.deepEqual(
    first.telemetry.players.map((player) => player.displayName),
    ["Локальный игрок"],
  );
  assert.match(first.identity.worldFingerprint, /^demo-synthetic-/);
});

test("every versioned demo DTO advertises schema v1", () => {
  const fixture = createDemoFixtureV1();

  assert.equal(fixture.identity.schemaVersion, WORLD_IDENTITY_SCHEMA_VERSION);
  assert.equal(fixture.layout.schemaVersion, LAYOUT_SCHEMA_VERSION);
  assert.equal(fixture.telemetry.schemaVersion, TELEMETRY_SCHEMA_VERSION);
  assert.equal(fixture.marker.schemaVersion, MARKER_SCHEMA_VERSION);
  assert.equal(fixture.fogDelta.schemaVersion, FOG_DELTA_SCHEMA_VERSION);
  assert.equal(fixture.route.schemaVersion, ROUTE_SCHEMA_VERSION);
});

test("profile snapshot keeps persistent state and its session gate explicit", () => {
  const fixture = createDemoFixtureV1();
  const profileKey = `smp1_${"1".repeat(64)}`;
  const snapshot = {
    schemaVersion: PROFILE_SCHEMA_VERSION,
    profile: {
      schemaVersion: PROFILE_SCHEMA_VERSION,
      profileKey,
      worldFingerprint: fixture.identity.worldFingerprint,
      scopeKind: "local",
      scopeId: "default",
      identityQuality: "stable",
      gameMode: fixture.identity.gameMode,
      serverKind: fixture.identity.server.kind,
      serverStableId: fixture.identity.server.stableId,
      displayName: null,
      needsManualDisambiguation: false,
    },
    sessionId: fixture.identity.sessionId,
    settings: {
      schemaVersion: PROFILE_SCHEMA_VERSION,
      fogEnabled: true,
      poiEnabled: ["schematic", "service"],
    },
    visited: {
      schemaVersion: PROFILE_SCHEMA_VERSION,
      worldId: fixture.identity.worldFingerprint,
      visited: fixture.fogDelta.revealedCells.slice(0, 2),
    },
    markers: {
      schemaVersion: PROFILE_SCHEMA_VERSION,
      worldId: fixture.identity.worldFingerprint,
      markers: [],
    },
    activeRoute: fixture.route,
    recentTrail: {
      schemaVersion: TRAIL_SCHEMA_VERSION,
      trailId: "demo-trail",
      sessionId: fixture.identity.sessionId,
      startedAtMs: 1_700_000_000_000,
      endedAtMs: null,
      pointCount: 1,
      points: [
        {
          sequence: 0,
          capturedAtMs: 1_700_000_000_000,
          world: fixture.telemetry.players[0].position,
          breakBefore: true,
        },
      ],
      truncated: false,
    },
  };
  const writeContext = {
    profileKey: snapshot.profile.profileKey,
    worldFingerprint: snapshot.profile.worldFingerprint,
    sessionId: snapshot.sessionId,
  };

  assert.equal(PROFILE_SCHEMA_VERSION, 1);
  assert.equal(TRAIL_SCHEMA_VERSION, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(snapshot)), snapshot);
  assert.deepEqual(writeContext, {
    profileKey,
    worldFingerprint: fixture.identity.worldFingerprint,
    sessionId: fixture.identity.sessionId,
  });
});

test("layout references are consistent and cells are bounded and unique", () => {
  const fixture = createDemoFixtureV1();
  const expectedWorld = {
    worldFingerprint: fixture.identity.worldFingerprint,
  };
  const expectedSessionWorld = {
    ...expectedWorld,
    sessionId: fixture.identity.sessionId,
  };
  const referencedDtos = [
    fixture.layout,
    fixture.marker,
    fixture.fogDelta,
    fixture.route,
  ];

  for (const dto of referencedDtos) {
    assert.deepEqual(dto.world, expectedWorld);
  }
  assert.deepEqual(fixture.telemetry.world, expectedSessionWorld);

  const keys = new Set();
  let poiCount = 0;
  for (const cell of fixture.layout.cells) {
    const key = cellKey(cell);
    assert.equal(keys.has(key), false, `duplicate layout cell ${key}`);
    keys.add(key);
    assert.ok(cell.x >= fixture.layout.bounds.minX);
    assert.ok(cell.x <= fixture.layout.bounds.maxX);
    assert.ok(cell.y >= fixture.layout.bounds.minY);
    assert.ok(cell.y <= fixture.layout.bounds.maxY);
    assert.match(cell.tileUuid, /^demo-/);
    assert.ok([0, 1, 2, 3].includes(cell.rotation));
    assert.ok(cell.roads.every((road) => ["n", "e", "s", "w"].includes(road)));
    if (cell.poi) {
      poiCount += 1;
      assert.match(cell.poi.poiId, /^demo-poi-/);
    }
  }

  assert.ok(keys.size > 100);
  assert.equal(poiCount, 6);
});

test("fog, marker, telemetry, and route stay inside the synthetic world", () => {
  const fixture = createDemoFixtureV1();
  const layoutKeys = new Set(fixture.layout.cells.map(cellKey));
  const revealedKeys = fixture.fogDelta.revealedCells.map(cellKey);

  assert.equal(new Set(revealedKeys).size, revealedKeys.length);
  assert.ok(revealedKeys.every((key) => layoutKeys.has(key)));

  const marker = fixture.marker;
  assert.ok(layoutKeys.has(cellKey(marker.position.cell)));
  assert.equal(
    marker.position.world.x,
    (marker.position.cell.x + 0.5) * fixture.layout.cellSize,
  );
  assert.equal(
    marker.position.world.y,
    (marker.position.cell.y + 0.5) * fixture.layout.cellSize,
  );

  const localPlayers = fixture.telemetry.players.filter(
    (player) => player.isLocal,
  );
  assert.equal(localPlayers.length, 1);
  assert.equal(localPlayers[0].playerId, fixture.telemetry.localPlayerId);

  assert.equal(fixture.route.destination.referenceId, marker.id);
  assert.deepEqual(fixture.route.destination.position, marker.position.world);
  assert.equal(fixture.route.path.length, 2);
  assert.ok(fixture.route.directDistanceWorldUnits > 0);
  assert.equal(
    fixture.route.routeDistanceWorldUnits,
    fixture.route.directDistanceWorldUnits,
  );
});
