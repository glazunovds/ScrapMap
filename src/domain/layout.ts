import type {
  CellBoundsV1,
  QuarterTurnV1,
  RoadDirectionV1,
} from "./primitives.js";
import type { WorldReferenceV1 } from "./world.js";

export const LAYOUT_KIND = "scrapmap.layout" as const;
export const LAYOUT_SCHEMA_VERSION = 1 as const;

export interface PoiPlacementV1 {
  readonly poiId: string;
  readonly type: string;
  readonly category: string;
  readonly displayName: string | null;
  readonly groupId: string | null;
}

export interface LayoutCellV1 {
  readonly x: number;
  readonly y: number;
  readonly tileUuid: string;
  readonly terrain: string;
  readonly rotation: QuarterTurnV1;
  readonly roads: readonly RoadDirectionV1[];
  readonly poi: PoiPlacementV1 | null;
  readonly xOffset: number;
  readonly yOffset: number;
  readonly groupId: number | null;
  readonly flags: number;
}

export interface LayoutV1 {
  readonly kind: typeof LAYOUT_KIND;
  readonly schemaVersion: typeof LAYOUT_SCHEMA_VERSION;
  readonly world: WorldReferenceV1;
  /**
   * Stored as text because game seeds may eventually exceed JavaScript's safe
   * integer range.
   */
  readonly seed: string | null;
  readonly cellSize: number;
  readonly bounds: CellBoundsV1;
  readonly cells: readonly LayoutCellV1[];
}

