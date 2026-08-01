import type {
  LayoutV1,
  TelemetryFrameV1,
  WorldIdentityV1,
} from "../domain/index.js";

export const DATA_SOURCE_STATUS_SCHEMA_VERSION = 1 as const;

export type Unsubscribe = () => void;

export type MapDataSourcePhase =
  | "idle"
  | "game-missing"
  | "waiting"
  | "loading-world"
  | "active"
  | "stale"
  | "unsupported"
  | "error";

export interface DataSourceReadOptions {
  readonly signal?: AbortSignal;
}

export interface MapDataSourceStatusV1 {
  readonly schemaVersion: typeof DATA_SOURCE_STATUS_SCHEMA_VERSION;
  readonly phase: MapDataSourcePhase;
  readonly changedAt: string;
  readonly message: string | null;
  readonly compatibilityId: string | null;
}

export interface MapDataSource {
  readonly sourceId: string;

  /** Returns the most recently observed world, if one is active. */
  loadWorldIdentity(
    options?: DataSourceReadOptions,
  ): Promise<WorldIdentityV1 | null>;

  /**
   * Loads the layout for the source's current world. Consumers must compare
   * the returned world reference with the latest identity to reject races.
   */
  loadLayout(options?: DataSourceReadOptions): Promise<LayoutV1 | null>;

  subscribeWorldIdentity(
    listener: (identity: WorldIdentityV1 | null) => void,
  ): Unsubscribe;

  subscribeTelemetry(
    listener: (frame: TelemetryFrameV1) => void,
  ): Unsubscribe;

  subscribeStatus(
    listener: (status: MapDataSourceStatusV1) => void,
  ): Unsubscribe;

  dispose(): void | Promise<void>;
}

