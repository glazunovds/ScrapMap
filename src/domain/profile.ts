import type { CellCoordinateV1 } from "./primitives.js";
import type { RouteV1 } from "./route.js";
import type { RecentTrailV1 } from "./trail.js";
import type { GameModeV1, ServerIdentityV1 } from "./world.js";

export const PROFILE_SCHEMA_VERSION = 1 as const;

/**
 * `fallback` is deliberately distinct from a stable server scope. It keeps an
 * unknown server isolated from local worlds and remains visible to the UI as
 * an identity that may need manual disambiguation.
 */
export type ProfileScopeKindV1 = "local" | "server" | "fallback";

export type ProfileIdentityQualityV1 =
  | "stable"
  | "fingerprint-only"
  | "manual";

export interface WorldProfileV1 {
  readonly schemaVersion: typeof PROFILE_SCHEMA_VERSION;
  readonly profileKey: string;
  readonly worldFingerprint: string;
  readonly scopeKind: ProfileScopeKindV1;
  readonly scopeId: string;
  readonly identityQuality: ProfileIdentityQualityV1;
  readonly gameMode: GameModeV1;
  readonly serverKind: ServerIdentityV1["kind"];
  readonly serverStableId: string | null;
  readonly displayName: string | null;
  readonly needsManualDisambiguation: boolean;
}

export interface ManualProfileCandidateV1 {
  readonly schemaVersion: typeof PROFILE_SCHEMA_VERSION;
  readonly profileKey: string;
  readonly worldFingerprint: string;
  readonly fallbackProfileId: string;
  readonly displayName: string | null;
  readonly lastOpenedAtMs: number;
}

export interface ManualProfileCandidatesV1 {
  readonly schemaVersion: typeof PROFILE_SCHEMA_VERSION;
  readonly worldFingerprint: string;
  readonly candidates: readonly ManualProfileCandidateV1[];
}

export interface ProfileCameraV1 {
  readonly x: number;
  readonly y: number;
  readonly zoom: number;
}

export interface ProfileSettingsV1 {
  readonly schemaVersion: typeof PROFILE_SCHEMA_VERSION;
  readonly fogEnabled: boolean;
  readonly poiEnabled: readonly string[];
  readonly camera?: ProfileCameraV1;
}

/**
 * Every mutating storage command carries this context. The native host rejects
 * it unless all three values match the currently active profile/session.
 */
export interface ProfileWriteContextV1 {
  readonly profileKey: string;
  readonly worldFingerprint: string;
  readonly sessionId: string;
}

export interface ProfileVisitedPayloadV1 {
  readonly schemaVersion: typeof PROFILE_SCHEMA_VERSION;
  readonly worldId: string;
  readonly visited: readonly CellCoordinateV1[];
}

/**
 * Compatibility shape used by the current dependency-free Canvas renderer.
 * A later renderer migration can replace this at a new schema boundary.
 */
export interface ProfileMarkerV1 {
  readonly id: string;
  readonly cellX: number;
  readonly cellY: number;
  readonly kind: string;
  readonly label: string;
  readonly createdAt: string | null;
}

export interface ProfileMarkersPayloadV1 {
  readonly schemaVersion: typeof PROFILE_SCHEMA_VERSION;
  readonly worldId: string;
  readonly markers: readonly ProfileMarkerV1[];
}

export interface ProfileSnapshotV1 {
  readonly schemaVersion: typeof PROFILE_SCHEMA_VERSION;
  readonly profile: WorldProfileV1;
  readonly sessionId: string;
  readonly settings: ProfileSettingsV1;
  readonly visited: ProfileVisitedPayloadV1;
  readonly markers: ProfileMarkersPayloadV1;
  readonly activeRoute: RouteV1 | null;
  readonly recentTrail: RecentTrailV1 | null;
}
