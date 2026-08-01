import type { CellCoordinateV1 } from "./primitives.js";
import type { WorldReferenceV1 } from "./world.js";

export const FOG_DELTA_KIND = "scrapmap.fog-delta" as const;
export const FOG_DELTA_SCHEMA_VERSION = 1 as const;

/**
 * A grow-only set delta. Sending the same operation or cell more than once is
 * safe: clients and the sync server merge cells using set union.
 */
export interface FogDeltaV1 {
  readonly kind: typeof FOG_DELTA_KIND;
  readonly schemaVersion: typeof FOG_DELTA_SCHEMA_VERSION;
  readonly world: WorldReferenceV1;
  readonly operationId: string;
  readonly createdAt: string;
  readonly baseCursor: string | null;
  readonly revealedCells: readonly CellCoordinateV1[];
}

