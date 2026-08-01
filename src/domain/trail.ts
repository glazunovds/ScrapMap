import type { WorldPositionV1 } from "./primitives.js";

export const TRAIL_SCHEMA_VERSION = 1 as const;

export interface BreadcrumbPointV1 {
  readonly sequence: number;
  readonly capturedAtMs: number;
  readonly world: WorldPositionV1;
  readonly breakBefore: boolean;
}

export interface RecentTrailV1 {
  readonly schemaVersion: typeof TRAIL_SCHEMA_VERSION;
  readonly trailId: string;
  readonly sessionId: string;
  readonly startedAtMs: number;
  readonly endedAtMs: number | null;
  readonly pointCount: number;
  readonly points: readonly BreadcrumbPointV1[];
  readonly truncated: boolean;
}

export interface TrailWriteResultV1 {
  readonly trailId: string;
  readonly appended: number;
  readonly total: number;
  readonly endedAtMs: number | null;
}
