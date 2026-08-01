import type { MapPositionV1 } from "./primitives.js";
import type { WorldReferenceV1 } from "./world.js";

export const MARKER_KIND = "scrapmap.marker" as const;
export const MARKER_TOMBSTONE_KIND = "scrapmap.marker-tombstone" as const;
export const MARKER_SCHEMA_VERSION = 1 as const;

export type MarkerScopeV1 = "local" | "shared";

interface MarkerRecordBaseV1 {
  readonly schemaVersion: typeof MARKER_SCHEMA_VERSION;
  readonly id: string;
  readonly world: WorldReferenceV1;
  readonly scope: MarkerScopeV1;
  readonly version: number;
  readonly serverRevision: string | null;
  readonly updatedAt: string;
  readonly updatedByMemberId: string | null;
}

export interface MarkerV1 extends MarkerRecordBaseV1 {
  readonly kind: typeof MARKER_KIND;
  readonly position: MapPositionV1;
  readonly label: string;
  readonly color: string;
  readonly icon: string;
  readonly category: string;
  readonly createdAt: string;
}

export interface MarkerTombstoneV1 extends MarkerRecordBaseV1 {
  readonly kind: typeof MARKER_TOMBSTONE_KIND;
  readonly deletedAt: string;
}

export type MarkerRecordV1 = MarkerV1 | MarkerTombstoneV1;

