import type { WorldPositionV1 } from "./primitives.js";
import type { WorldSessionReferenceV1 } from "./world.js";

export const TELEMETRY_KIND = "scrapmap.telemetry" as const;
export const TELEMETRY_SCHEMA_VERSION = 1 as const;

export type PlayerPresenceV1 =
  | "active"
  | "no-character"
  | "other-world"
  | "disconnected";

export interface PlayerTelemetryV1 {
  readonly playerId: string;
  readonly displayName: string | null;
  readonly isLocal: boolean;
  readonly presence: PlayerPresenceV1;
  readonly position: WorldPositionV1 | null;
  readonly headingDegrees: number | null;
}

export interface TelemetryFrameV1 {
  readonly kind: typeof TELEMETRY_KIND;
  readonly schemaVersion: typeof TELEMETRY_SCHEMA_VERSION;
  readonly world: WorldSessionReferenceV1;
  readonly sequence: number;
  readonly capturedAt: string;
  readonly staleAfterMs: number;
  readonly localPlayerId: string | null;
  readonly players: readonly PlayerTelemetryV1[];
}
