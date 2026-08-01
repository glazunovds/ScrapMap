import type {
  MarkerV1,
  PlayerTelemetryV1,
} from "../domain/index.js";
import type { DemoFixtureV1 } from "./demo-v1.js";

function toLegacyPlayer(player: PlayerTelemetryV1, worldId: string) {
  const position = player.position ?? { x: 0, y: 0, z: 0 };
  return {
    id: player.playerId,
    name:
      player.displayName ??
      (player.isLocal ? "Локальный игрок" : "Удалённый игрок"),
    local: player.isLocal,
    active: player.presence !== "disconnected",
    hasCharacter:
      player.position !== null && player.presence !== "no-character",
    sameWorld: player.presence !== "other-world",
    worldId,
    ...position,
    heading: player.headingDegrees ?? 0,
  };
}

function toLegacyMarker(marker: MarkerV1) {
  return {
    id: marker.id,
    cellX: marker.position.cell.x,
    cellY: marker.position.cell.y,
    kind: marker.icon,
    label: marker.label,
    createdAt: marker.createdAt,
    local: marker.scope === "local",
  };
}

/**
 * Temporary compatibility boundary for tests and incremental migration. New
 * data sources speak versioned DTOs; the legacy Canvas renderer keeps its
 * existing input shape until it is moved to TypeScript.
 */
export function toLegacyRendererBundle(fixture: DemoFixtureV1) {
  const worldId = fixture.identity.worldFingerprint;
  const players = fixture.telemetry.players.map((player) =>
    toLegacyPlayer(player, worldId),
  );
  const localPlayer =
    players.find((player) => player.id === fixture.telemetry.localPlayerId) ??
    players.find((player) => player.local);

  if (!localPlayer) {
    throw new TypeError("Telemetry fixture must contain a local player.");
  }

  return {
    layout: {
      schemaVersion: fixture.layout.schemaVersion,
      worldId,
      seed: fixture.layout.seed,
      cellSize: fixture.layout.cellSize,
      bounds: fixture.layout.bounds,
      cells: fixture.layout.cells.map((cell) => ({
        x: cell.x,
        y: cell.y,
        uuid: cell.tileUuid,
        terrain: cell.terrain,
        rotation: cell.rotation,
        roads: [...cell.roads],
        poi: cell.poi
          ? {
              id: cell.poi.poiId,
              kind: cell.poi.type,
              category: cell.poi.category,
              label: cell.poi.displayName,
              groupId: cell.poi.groupId,
            }
          : null,
        xOffset: cell.xOffset,
        yOffset: cell.yOffset,
        groupId: cell.groupId,
        flags: cell.flags,
      })),
    },
    telemetry: {
      schemaVersion: fixture.telemetry.schemaVersion,
      worldId,
      payloadWorldId: worldId,
      timestamp: fixture.telemetry.capturedAt,
      staleAfterMs: fixture.telemetry.staleAfterMs,
      player: localPlayer,
      players,
    },
    visited: {
      worldId,
      visited: fixture.fogDelta.revealedCells.map(({ x, y }) => ({ x, y })),
    },
    markers: {
      worldId,
      markers: [toLegacyMarker(fixture.marker)],
    },
  };
}

