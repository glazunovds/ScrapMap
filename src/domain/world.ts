export const WORLD_IDENTITY_KIND = "scrapmap.world-identity" as const;
export const WORLD_IDENTITY_SCHEMA_VERSION = 1 as const;

export type GameModeV1 =
  | "survival"
  | "creative"
  | "challenge"
  | "unknown";

export type ServerIdentityV1 =
  | {
      readonly kind: "local" | "unknown";
      readonly stableId: null;
    }
  | {
      readonly kind: "peer-hosted" | "dedicated" | "steam-lobby";
      /**
       * An identifier is stable only when it survives leaving and rejoining
       * the same server across application and game sessions. A transient
       * lobby/session identifier must never be persisted here; use `null`
       * until a durable identity is available.
       */
      readonly stableId: string | null;
    };

export interface GameBuildIdentityV1 {
  readonly displayVersion: string | null;
  readonly executableSha256: string | null;
  readonly compatibilityId: string | null;
}

export interface WorldReferenceV1 {
  readonly worldFingerprint: string;
}

/**
 * Correlates volatile payloads inside one observed game session. Persistent
 * DTOs use `WorldReferenceV1`; only live data carries the changing session ID.
 */
export interface WorldSessionReferenceV1 extends WorldReferenceV1 {
  readonly sessionId: string;
}

export interface WorldIdentityV1 extends WorldSessionReferenceV1 {
  readonly kind: typeof WORLD_IDENTITY_KIND;
  readonly schemaVersion: typeof WORLD_IDENTITY_SCHEMA_VERSION;
  readonly gameMode: GameModeV1;
  readonly server: ServerIdentityV1;
  readonly gameBuild: GameBuildIdentityV1;
  readonly observedAt: string;
}
