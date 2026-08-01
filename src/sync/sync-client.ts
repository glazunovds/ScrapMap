import type {
  CellCoordinateV1,
  FogDeltaV1,
  MarkerRecordV1,
  MarkerTombstoneV1,
  MarkerV1,
  WorldIdentityV1,
} from "../domain/index.js";

export const SYNC_PROTOCOL_SCHEMA_VERSION = 1 as const;

export interface SyncRequestOptions {
  readonly signal?: AbortSignal;
}

export interface SyncMutationOptions extends SyncRequestOptions {
  readonly idempotencyKey: string;
}

export interface ResolvedSyncWorldV1 {
  readonly schemaVersion: typeof SYNC_PROTOCOL_SCHEMA_VERSION;
  readonly syncWorldId: string;
  readonly worldFingerprint: string;
  readonly cursor: string;
}

export interface SyncSnapshotV1 {
  readonly schemaVersion: typeof SYNC_PROTOCOL_SCHEMA_VERSION;
  readonly syncWorldId: string;
  readonly cursor: string;
  readonly revealedCells: readonly CellCoordinateV1[];
  readonly markers: readonly MarkerRecordV1[];
}

export interface SyncChangeBatchV1 {
  readonly schemaVersion: typeof SYNC_PROTOCOL_SCHEMA_VERSION;
  readonly syncWorldId: string;
  readonly cursor: string;
  readonly revealedCells: readonly CellCoordinateV1[];
  readonly markers: readonly MarkerRecordV1[];
}

export interface SyncMutationAckV1 {
  readonly schemaVersion: typeof SYNC_PROTOCOL_SCHEMA_VERSION;
  readonly cursor: string;
  readonly replayed: boolean;
}

/**
 * Transport-neutral sync API. Authentication and secrets stay in the native
 * host; implementations must not expose Bearer tokens to the WebView.
 */
export interface SyncClient {
  resolveWorld(
    identity: WorldIdentityV1,
    options?: SyncRequestOptions,
  ): Promise<ResolvedSyncWorldV1>;

  loadSnapshot(
    world: ResolvedSyncWorldV1,
    options?: SyncRequestOptions,
  ): Promise<SyncSnapshotV1>;

  pollChanges(
    world: ResolvedSyncWorldV1,
    afterCursor: string,
    options?: SyncRequestOptions,
  ): Promise<SyncChangeBatchV1>;

  pushFog(
    world: ResolvedSyncWorldV1,
    delta: FogDeltaV1,
    options: SyncMutationOptions,
  ): Promise<SyncMutationAckV1>;

  upsertMarker(
    world: ResolvedSyncWorldV1,
    marker: MarkerV1,
    options: SyncMutationOptions,
  ): Promise<SyncMutationAckV1>;

  deleteMarker(
    world: ResolvedSyncWorldV1,
    tombstone: MarkerTombstoneV1,
    options: SyncMutationOptions,
  ): Promise<SyncMutationAckV1>;

  dispose(): void | Promise<void>;
}

