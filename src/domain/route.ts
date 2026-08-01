import type {
  CellCoordinateV1,
  WorldPositionV1,
} from "./primitives.js";
import type { WorldReferenceV1 } from "./world.js";

export const ROUTE_KIND = "scrapmap.route" as const;
export const ROUTE_SCHEMA_VERSION = 1 as const;

export type RouteTargetKindV1 = "poi" | "marker" | "point";
export type RouteStrategyV1 = "roads" | "direct";
export type RouteStatusV1 = "ready" | "partial" | "unreachable";

export interface RouteEndpointV1 {
  readonly kind: RouteTargetKindV1;
  readonly referenceId: string | null;
  readonly label: string | null;
  readonly position: WorldPositionV1;
}

export interface RouteV1 {
  readonly kind: typeof ROUTE_KIND;
  readonly schemaVersion: typeof ROUTE_SCHEMA_VERSION;
  readonly id: string;
  readonly world: WorldReferenceV1;
  readonly generatedAt: string;
  readonly strategy: RouteStrategyV1;
  readonly status: RouteStatusV1;
  readonly start: RouteEndpointV1;
  readonly destination: RouteEndpointV1;
  readonly path: readonly WorldPositionV1[];
  readonly pathCells: readonly CellCoordinateV1[];
  readonly directDistanceWorldUnits: number;
  readonly routeDistanceWorldUnits: number | null;
}

