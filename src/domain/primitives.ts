export interface CellCoordinateV1 {
  readonly x: number;
  readonly y: number;
}

export interface CellBoundsV1 {
  readonly minX: number;
  readonly maxX: number;
  readonly minY: number;
  readonly maxY: number;
}

export interface WorldPositionV1 {
  readonly x: number;
  readonly y: number;
  readonly z: number;
}

export interface MapPositionV1 {
  readonly cell: CellCoordinateV1;
  readonly world: WorldPositionV1;
}

export type QuarterTurnV1 = 0 | 1 | 2 | 3;

export type RoadDirectionV1 = "n" | "e" | "s" | "w";

