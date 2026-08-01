import {
  FOG_DELTA_KIND,
  FOG_DELTA_SCHEMA_VERSION,
  LAYOUT_KIND,
  LAYOUT_SCHEMA_VERSION,
  MARKER_KIND,
  MARKER_SCHEMA_VERSION,
  ROUTE_KIND,
  ROUTE_SCHEMA_VERSION,
  TELEMETRY_KIND,
  TELEMETRY_SCHEMA_VERSION,
  WORLD_IDENTITY_KIND,
  WORLD_IDENTITY_SCHEMA_VERSION,
} from "../domain/index.js";
import type {
  FogDeltaV1,
  LayoutCellV1,
  LayoutV1,
  MarkerV1,
  PoiPlacementV1,
  QuarterTurnV1,
  RoadDirectionV1,
  RouteV1,
  TelemetryFrameV1,
  WorldIdentityV1,
  WorldReferenceV1,
  WorldSessionReferenceV1,
} from "../domain/index.js";

const DEMO_TIMESTAMP = "2026-01-01T00:00:00.000Z";
const DEMO_WORLD_FINGERPRINT = "demo-synthetic-world-v1";
const DEMO_SESSION_ID = "demo-session-v1";
const DEMO_CELL_SIZE = 64;

export interface DemoFixtureV1 {
  readonly identity: WorldIdentityV1;
  readonly layout: LayoutV1;
  readonly telemetry: TelemetryFrameV1;
  readonly fogDelta: FogDeltaV1;
  readonly marker: MarkerV1;
  readonly route: RouteV1;
}

function demoWorldReference(): WorldReferenceV1 {
  return {
    worldFingerprint: DEMO_WORLD_FINGERPRINT,
  };
}

function demoWorldSessionReference(): WorldSessionReferenceV1 {
  return {
    ...demoWorldReference(),
    sessionId: DEMO_SESSION_ID,
  };
}

function demoPois(): ReadonlyMap<string, PoiPlacementV1> {
  return new Map([
    [
      "-3,2",
      {
        poiId: "demo-poi-warehouse",
        type: "warehouse",
        category: "warehouse",
        displayName: "Warehouse",
        groupId: null,
      },
    ],
    [
      "0,0",
      {
        poiId: "demo-poi-mechanic",
        type: "mechanic",
        category: "service",
        displayName: "Mechanic Station",
        groupId: null,
      },
    ],
    [
      "4,1",
      {
        poiId: "demo-poi-packing",
        type: "packing",
        category: "service",
        displayName: "Packing Station",
        groupId: null,
      },
    ],
    [
      "2,-4",
      {
        poiId: "demo-poi-ruin",
        type: "ruin",
        category: "landmark",
        displayName: "Ruined City",
        groupId: null,
      },
    ],
    [
      "-4,-3",
      {
        poiId: "demo-poi-camp",
        type: "camp",
        category: "camp",
        displayName: "Camp",
        groupId: null,
      },
    ],
    [
      "1,4",
      {
        poiId: "demo-poi-lab",
        type: "lab",
        category: "dungeon",
        displayName: "Grow Lab",
        groupId: null,
      },
    ],
  ]);
}

function roadsAt(x: number, y: number): RoadDirectionV1[] {
  const roads: RoadDirectionV1[] = [];

  if (y === 0 && x >= -6 && x <= 5) {
    if (x > -6) roads.push("w");
    if (x < 5) roads.push("e");
  }
  if (x === 0 && y >= -5 && y <= 5) {
    if (y > -5) roads.push("s");
    if (y < 5) roads.push("n");
  }
  if (x === 3 && y >= 0 && y <= 3) {
    if (y > 0) roads.push("s");
    if (y < 3) roads.push("n");
  }
  if (y === 3 && x >= 0 && x <= 4) {
    if (x > 0) roads.push("w");
    if (x < 4) roads.push("e");
  }

  return roads;
}

function demoCells(): LayoutCellV1[] {
  const terrains = [
    "meadow",
    "meadow",
    "forest",
    "field",
    "desert",
    "industrial",
    "autumn",
  ] as const;
  const pois = demoPois();
  const cells: LayoutCellV1[] = [];

  for (let y = -6; y <= 6; y += 1) {
    for (let x = -7; x <= 7; x += 1) {
      const distance = Math.hypot(x * 0.92, y);
      if (distance > 7.6 || (x === 6 && y < -2)) {
        continue;
      }

      const terrainIndex = Math.abs(
        (x * 7 + y * 11 + Math.floor(distance)) % terrains.length,
      );
      const rotation = Math.abs((x + y) % 4) as QuarterTurnV1;

      cells.push({
        x,
        y,
        tileUuid: `demo-${x + 8}-${y + 7}`,
        terrain: terrains[terrainIndex],
        rotation,
        roads: roadsAt(x, y),
        poi: pois.get(`${x},${y}`) ?? null,
        xOffset: 0,
        yOffset: 0,
        groupId: null,
        flags: 0,
      });
    }
  }

  return cells;
}

export function createDemoFixtureV1(): DemoFixtureV1 {
  const world = demoWorldReference();
  const worldSession = demoWorldSessionReference();
  const cells = demoCells();
  const playerPosition = { x: 54, y: -18, z: 12 };
  const markerPosition = {
    cell: { x: -3, y: 2 },
    world: {
      x: (-3 + 0.5) * DEMO_CELL_SIZE,
      y: (2 + 0.5) * DEMO_CELL_SIZE,
      z: 0,
    },
  };

  const identity: WorldIdentityV1 = {
    kind: WORLD_IDENTITY_KIND,
    schemaVersion: WORLD_IDENTITY_SCHEMA_VERSION,
    ...worldSession,
    gameMode: "survival",
    server: { kind: "local", stableId: null },
    gameBuild: {
      displayVersion: "synthetic-demo",
      executableSha256: null,
      compatibilityId: "synthetic-demo-v1",
    },
    observedAt: DEMO_TIMESTAMP,
  };

  const layout: LayoutV1 = {
    kind: LAYOUT_KIND,
    schemaVersion: LAYOUT_SCHEMA_VERSION,
    world: { ...world },
    seed: "667978921",
    cellSize: DEMO_CELL_SIZE,
    bounds: { minX: -7, maxX: 7, minY: -6, maxY: 6 },
    cells,
  };

  const telemetry: TelemetryFrameV1 = {
    kind: TELEMETRY_KIND,
    schemaVersion: TELEMETRY_SCHEMA_VERSION,
    world: { ...worldSession },
    sequence: 1,
    capturedAt: DEMO_TIMESTAMP,
    staleAfterMs: 2_000,
    localPlayerId: "demo-player-local",
    players: [
      {
        playerId: "demo-player-local",
        displayName: "Локальный игрок",
        isLocal: true,
        presence: "active",
        position: playerPosition,
        headingDegrees: 34,
      },
    ],
  };

  const revealedCells = cells
    .filter(
      (cell) =>
        Math.hypot(cell.x - 0.2, cell.y + 0.1) < 4.25 ||
        (cell.y === 0 && cell.x <= 4),
    )
    .map(({ x, y }) => ({ x, y }));

  const fogDelta: FogDeltaV1 = {
    kind: FOG_DELTA_KIND,
    schemaVersion: FOG_DELTA_SCHEMA_VERSION,
    world: { ...world },
    operationId: "demo-fog-operation-v1",
    createdAt: DEMO_TIMESTAMP,
    baseCursor: null,
    revealedCells,
  };

  const marker: MarkerV1 = {
    kind: MARKER_KIND,
    schemaVersion: MARKER_SCHEMA_VERSION,
    id: "demo-marker-warehouse",
    world: { ...world },
    scope: "local",
    version: 0,
    serverRevision: null,
    position: markerPosition,
    label: "Вернуться за лутом",
    color: "#ff7464",
    icon: "x",
    category: "note",
    createdAt: DEMO_TIMESTAMP,
    updatedAt: DEMO_TIMESTAMP,
    updatedByMemberId: null,
  };

  const directDistance = Math.hypot(
    markerPosition.world.x - playerPosition.x,
    markerPosition.world.y - playerPosition.y,
  );
  const route: RouteV1 = {
    kind: ROUTE_KIND,
    schemaVersion: ROUTE_SCHEMA_VERSION,
    id: "demo-route-to-warehouse",
    world: { ...world },
    generatedAt: DEMO_TIMESTAMP,
    strategy: "direct",
    status: "ready",
    start: {
      kind: "point",
      referenceId: null,
      label: "Текущее положение",
      position: playerPosition,
    },
    destination: {
      kind: "marker",
      referenceId: marker.id,
      label: marker.label,
      position: marker.position.world,
    },
    path: [playerPosition, marker.position.world],
    pathCells: [
      {
        x: Math.floor(playerPosition.x / DEMO_CELL_SIZE),
        y: Math.floor(playerPosition.y / DEMO_CELL_SIZE),
      },
      marker.position.cell,
    ],
    directDistanceWorldUnits: directDistance,
    routeDistanceWorldUnits: directDistance,
  };

  return { identity, layout, telemetry, fogDelta, marker, route };
}
