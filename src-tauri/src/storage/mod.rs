use std::{
    collections::HashSet,
    env,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const CURRENT_SCHEMA_VERSION: i64 = 1;
const MAX_ID_CHARS: usize = 160;
const MAX_TILE_ID_CHARS: usize = 512;
const MAX_SESSION_ID_CHARS: usize = 128;
const MAX_LABEL_CHARS: usize = 160;
const MAX_PROFILE_NAME_CHARS: usize = 80;
const MAX_CATEGORY_CHARS: usize = 64;
const MAX_LAYOUT_CELLS: usize = 65_536;
const MAX_FOG_BATCH: usize = 4_096;
const MAX_MARKERS: usize = 16_384;
const MAX_ROUTE_POINTS: usize = 65_536;
const MAX_BREADCRUMB_BATCH: usize = 4_096;
const MAX_RECENT_TRAIL_POINTS: usize = 4_096;
const MAX_SETTINGS_JSON_BYTES: usize = 64 * 1024;
const MAX_LAYOUT_JSON_BYTES: usize = 64 * 1024 * 1024;
const MAX_MARKER_JSON_BYTES: usize = 64 * 1024;
const MAX_ROUTE_JSON_BYTES: usize = 8 * 1024 * 1024;
const MAX_CELL_ABS: i64 = 1_000_000;
const MAX_COORDINATE_ABS: f64 = 10_000_000.0;
const MAX_JAVASCRIPT_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const DEFAULT_POI_CATEGORIES: &[&str] = &[
    "schematic",
    "quest",
    "camp",
    "warehouse",
    "service",
    "dungeon",
    "landmark",
];

const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at_ms INTEGER NOT NULL
);

CREATE TABLE world_layouts (
    world_fingerprint TEXT PRIMARY KEY,
    fingerprint_version INTEGER NOT NULL CHECK (fingerprint_version = 1),
    layout_schema_version INTEGER NOT NULL,
    game_mode TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    payload_bytes INTEGER NOT NULL CHECK (payload_bytes >= 2),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE world_profiles (
    profile_key TEXT PRIMARY KEY,
    profile_key_version INTEGER NOT NULL CHECK (profile_key_version = 1),
    world_fingerprint TEXT NOT NULL
        REFERENCES world_layouts(world_fingerprint) ON DELETE RESTRICT,
    scope_kind TEXT NOT NULL
        CHECK (scope_kind IN ('local', 'server', 'fallback')),
    scope_id TEXT NOT NULL,
    identity_quality TEXT NOT NULL
        CHECK (identity_quality IN ('stable', 'fingerprint-only', 'manual')),
    game_mode TEXT NOT NULL,
    server_kind TEXT NOT NULL,
    server_stable_id TEXT,
    display_name TEXT,
    needs_manual_disambiguation INTEGER NOT NULL DEFAULT 0
        CHECK (needs_manual_disambiguation IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    last_opened_at_ms INTEGER NOT NULL,
    UNIQUE (world_fingerprint, scope_kind, scope_id)
);

CREATE INDEX world_profiles_recent
    ON world_profiles(last_opened_at_ms DESC);

CREATE TABLE profile_settings (
    profile_key TEXT PRIMARY KEY
        REFERENCES world_profiles(profile_key) ON DELETE CASCADE,
    schema_version INTEGER NOT NULL,
    fog_enabled INTEGER NOT NULL DEFAULT 1 CHECK (fog_enabled IN (0, 1)),
    poi_enabled_json TEXT NOT NULL CHECK (json_valid(poi_enabled_json)),
    camera_json TEXT CHECK (camera_json IS NULL OR json_valid(camera_json)),
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE fog_cells (
    profile_key TEXT NOT NULL
        REFERENCES world_profiles(profile_key) ON DELETE CASCADE,
    cell_x INTEGER NOT NULL,
    cell_y INTEGER NOT NULL,
    revealed_at_ms INTEGER NOT NULL,
    origin TEXT NOT NULL CHECK (origin IN ('local', 'shared', 'import')),
    PRIMARY KEY (profile_key, cell_x, cell_y)
) WITHOUT ROWID;

CREATE TABLE marker_records (
    profile_key TEXT NOT NULL
        REFERENCES world_profiles(profile_key) ON DELETE CASCADE,
    marker_id TEXT NOT NULL,
    record_kind TEXT NOT NULL CHECK (record_kind IN ('marker', 'tombstone')),
    scope TEXT NOT NULL CHECK (scope IN ('local', 'shared')),
    version INTEGER NOT NULL CHECK (version >= 0),
    server_revision TEXT,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (profile_key, marker_id)
) WITHOUT ROWID;

CREATE TABLE routes (
    profile_key TEXT NOT NULL
        REFERENCES world_profiles(profile_key) ON DELETE CASCADE,
    route_id TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    generated_at_ms INTEGER NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK (active IN (0, 1)),
    PRIMARY KEY (profile_key, route_id)
) WITHOUT ROWID;

CREATE UNIQUE INDEX one_active_route_per_profile
    ON routes(profile_key) WHERE active = 1;

CREATE TABLE trails (
    profile_key TEXT NOT NULL
        REFERENCES world_profiles(profile_key) ON DELETE CASCADE,
    trail_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    PRIMARY KEY (profile_key, trail_id)
) WITHOUT ROWID;

CREATE TABLE breadcrumb_points (
    profile_key TEXT NOT NULL,
    trail_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    captured_at_ms INTEGER NOT NULL,
    world_x REAL NOT NULL,
    world_y REAL NOT NULL,
    world_z REAL NOT NULL,
    break_before INTEGER NOT NULL DEFAULT 0 CHECK (break_before IN (0, 1)),
    PRIMARY KEY (profile_key, trail_id, sequence),
    FOREIGN KEY (profile_key, trail_id)
        REFERENCES trails(profile_key, trail_id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE TABLE app_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    last_profile_key TEXT
        REFERENCES world_profiles(profile_key) ON DELETE SET NULL,
    updated_at_ms INTEGER NOT NULL
);
"#;

#[derive(Debug)]
pub enum StorageError {
    Database(rusqlite::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    Validation(String),
    NoActiveProfile,
    StaleSession,
    ProfileResolutionRequired,
    UnsupportedSchema(i64),
    Clock,
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "storage database error: {error}"),
            Self::Io(error) => write!(formatter, "storage I/O error: {error}"),
            Self::Json(error) => write!(formatter, "storage JSON error: {error}"),
            Self::Validation(message) => write!(formatter, "invalid storage request: {message}"),
            Self::NoActiveProfile => formatter.write_str("no world profile is active"),
            Self::StaleSession => formatter.write_str("request belongs to a stale world session"),
            Self::ProfileResolutionRequired => {
                formatter.write_str("select a manual server profile before saving data")
            }
            Self::UnsupportedSchema(version) => {
                write!(
                    formatter,
                    "database schema version {version} is newer than supported"
                )
            }
            Self::Clock => formatter.write_str("system clock is before the Unix epoch"),
        }
    }
}

impl Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateProfileRequestV1 {
    pub schema_version: u32,
    pub session_id: String,
    pub game_mode: String,
    pub server: ServerIdentityInputV1,
    #[serde(default)]
    pub fallback_profile_id: Option<String>,
    #[serde(default)]
    pub fallback_profile_name: Option<String>,
    pub layout: LayoutInputV1,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerIdentityInputV1 {
    pub kind: String,
    #[serde(default)]
    pub stable_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutInputV1 {
    pub schema_version: u32,
    #[serde(default)]
    pub world_id: Option<String>,
    pub cell_size: f64,
    pub bounds: LayoutBoundsInputV1,
    pub cells: Vec<LayoutCellInputV1>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutBoundsInputV1 {
    pub min_x: i64,
    pub max_x: i64,
    pub min_y: i64,
    pub max_y: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutCellInputV1 {
    pub x: i64,
    pub y: i64,
    pub tile_uuid: String,
    pub rotation: u8,
    #[serde(default)]
    pub x_offset: f64,
    #[serde(default)]
    pub y_offset: f64,
    #[serde(default)]
    pub flags: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileWriteContextV1 {
    pub profile_key: String,
    pub world_fingerprint: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSettingsV1 {
    pub schema_version: u32,
    pub fog_enabled: bool,
    pub poi_enabled: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<Value>,
}

impl Default for ProfileSettingsV1 {
    fn default() -> Self {
        Self {
            schema_version: 1,
            fog_enabled: true,
            poi_enabled: DEFAULT_POI_CATEGORIES
                .iter()
                .map(|category| (*category).to_owned())
                .collect(),
            camera: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProfileSettingsRequestV1 {
    pub schema_version: u32,
    pub context: ProfileWriteContextV1,
    pub settings: ProfileSettingsV1,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptTelemetrySequenceRequestV1 {
    pub schema_version: u32,
    pub context: ProfileWriteContextV1,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldPositionV1 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldReferenceV1 {
    pub world_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteEndpointV1 {
    pub kind: String,
    pub reference_id: Option<String>,
    pub label: Option<String>,
    pub position: WorldPositionV1,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteV1 {
    pub kind: String,
    pub schema_version: u32,
    pub id: String,
    pub world: WorldReferenceV1,
    pub generated_at: String,
    pub strategy: String,
    pub status: String,
    pub start: RouteEndpointV1,
    pub destination: RouteEndpointV1,
    pub path: Vec<WorldPositionV1>,
    pub path_cells: Vec<CellCoordinateV1>,
    pub direct_distance_world_units: f64,
    pub route_distance_world_units: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetActiveRouteRequestV1 {
    pub schema_version: u32,
    pub context: ProfileWriteContextV1,
    pub route: Option<RouteV1>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BreadcrumbPointV1 {
    pub sequence: u64,
    pub captured_at_ms: i64,
    pub world: WorldPositionV1,
    pub break_before: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteTrailBatchRequestV1 {
    pub schema_version: u32,
    pub context: ProfileWriteContextV1,
    pub trail_id: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub points: Vec<BreadcrumbPointV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecentTrailV1 {
    pub schema_version: u32,
    pub trail_id: String,
    pub session_id: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub point_count: usize,
    pub points: Vec<BreadcrumbPointV1>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrailWriteResultV1 {
    pub trail_id: String,
    pub appended: usize,
    pub total: usize,
    pub ended_at_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CellCoordinateV1 {
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeFogRequestV1 {
    pub schema_version: u32,
    pub context: ProfileWriteContextV1,
    pub cells: Vec<CellCoordinateV1>,
    #[serde(default = "default_fog_origin")]
    pub origin: String,
}

fn default_fog_origin() -> String {
    "local".to_owned()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMarkerV1 {
    pub id: String,
    pub cell_x: i64,
    pub cell_y: i64,
    #[serde(default = "default_marker_kind")]
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

fn default_marker_kind() -> String {
    "x".to_owned()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceLocalMarkersRequestV1 {
    pub schema_version: u32,
    pub context: ProfileWriteContextV1,
    pub markers: Vec<LegacyMarkerV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldProfileV1 {
    pub schema_version: u32,
    pub profile_key: String,
    pub world_fingerprint: String,
    pub scope_kind: String,
    pub scope_id: String,
    pub identity_quality: String,
    pub game_mode: String,
    pub server_kind: String,
    pub server_stable_id: Option<String>,
    pub display_name: Option<String>,
    pub needs_manual_disambiguation: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListManualProfilesRequestV1 {
    pub schema_version: u32,
    pub world_fingerprint: String,
    pub server_kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManualProfileCandidateV1 {
    pub schema_version: u32,
    pub profile_key: String,
    pub world_fingerprint: String,
    pub fallback_profile_id: String,
    pub display_name: Option<String>,
    pub last_opened_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListManualProfilesResultV1 {
    pub schema_version: u32,
    pub world_fingerprint: String,
    pub candidates: Vec<ManualProfileCandidateV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyVisitedPayloadV1 {
    pub schema_version: u32,
    pub world_id: String,
    pub visited: Vec<CellCoordinateV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMarkersPayloadV1 {
    pub schema_version: u32,
    pub world_id: String,
    pub markers: Vec<LegacyMarkerV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSnapshotV1 {
    pub schema_version: u32,
    pub profile: WorldProfileV1,
    pub session_id: String,
    pub settings: ProfileSettingsV1,
    pub visited: LegacyVisitedPayloadV1,
    pub markers: LegacyMarkersPayloadV1,
    pub active_route: Option<RouteV1>,
    pub recent_trail: Option<RecentTrailV1>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FogMergeResultV1 {
    pub inserted: usize,
    pub total: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarkerReplaceResultV1 {
    pub stored: usize,
}

#[derive(Clone, Debug)]
struct ActiveSession {
    profile_key: String,
    world_fingerprint: String,
    session_id: String,
    writable: bool,
    last_telemetry_sequence: Option<u64>,
}

pub struct StorageState {
    operation: Mutex<()>,
    connection: Mutex<Connection>,
    active: Mutex<Option<ActiveSession>>,
    #[allow(dead_code)]
    database_path: Option<PathBuf>,
}

impl StorageState {
    pub fn open_default() -> Result<Self, StorageError> {
        let local_app_data = env::var_os("LOCALAPPDATA")
            .ok_or_else(|| StorageError::Validation("LOCALAPPDATA is not available".to_owned()))?;
        Self::open(
            PathBuf::from(local_app_data)
                .join("ScrapMap")
                .join("scrapmap.sqlite3"),
        )
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&path)?;
        Self::from_connection(connection, Some(path))
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, StorageError> {
        Self::from_connection(Connection::open_in_memory()?, None)
    }

    fn from_connection(
        mut connection: Connection,
        database_path: Option<PathBuf>,
    ) -> Result<Self, StorageError> {
        configure_connection(&connection, database_path.is_some())?;
        migrate(&mut connection)?;
        Ok(Self {
            operation: Mutex::new(()),
            connection: Mutex::new(connection),
            active: Mutex::new(None),
            database_path,
        })
    }

    #[allow(dead_code)]
    pub fn database_path(&self) -> Option<&Path> {
        self.database_path.as_deref()
    }

    pub fn activate_profile(
        &self,
        mut request: ActivateProfileRequestV1,
    ) -> Result<ProfileSnapshotV1, StorageError> {
        let _operation = self.lock_operation();
        validate_activate_request(&request)?;
        normalize_layout(&mut request.layout);
        request.fallback_profile_name = request
            .fallback_profile_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        let world_fingerprint = compute_world_fingerprint(&request.layout)?;
        let scope = resolve_scope(&request.server, request.fallback_profile_id.as_deref())?;
        let profile_key = compute_profile_key(&scope.kind, &scope.id, &world_fingerprint);
        let now = now_ms()?;
        let layout_json = serde_json::to_string(&request.layout)?;
        if layout_json.len() > MAX_LAYOUT_JSON_BYTES {
            return Err(StorageError::Validation("layout is too large".to_owned()));
        }

        {
            let mut connection = self.lock_connection();
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            store_layout(
                &transaction,
                &world_fingerprint,
                &request.game_mode,
                &layout_json,
                request.layout.schema_version,
                now,
            )?;
            upsert_profile(
                &transaction,
                &profile_key,
                &world_fingerprint,
                &scope,
                &request,
                now,
            )?;
            ensure_settings(&transaction, &profile_key, now)?;
            transaction.execute(
                "INSERT INTO app_state(singleton, last_profile_key, updated_at_ms)
                 VALUES(1, ?1, ?2)
                 ON CONFLICT(singleton) DO UPDATE SET
                    last_profile_key = excluded.last_profile_key,
                    updated_at_ms = excluded.updated_at_ms",
                params![profile_key, now],
            )?;
            transaction.commit()?;
        }

        *self.lock_active() = Some(ActiveSession {
            profile_key: profile_key.clone(),
            world_fingerprint,
            session_id: request.session_id,
            writable: !scope.needs_manual_disambiguation,
            last_telemetry_sequence: None,
        });

        self.get_active_snapshot_locked()?
            .ok_or(StorageError::NoActiveProfile)
    }

    pub fn get_active_snapshot(&self) -> Result<Option<ProfileSnapshotV1>, StorageError> {
        let _operation = self.lock_operation();
        self.get_active_snapshot_locked()
    }

    pub fn list_manual_profiles(
        &self,
        request: ListManualProfilesRequestV1,
    ) -> Result<ListManualProfilesResultV1, StorageError> {
        let _operation = self.lock_operation();
        validate_command_schema(request.schema_version)?;
        validate_world_fingerprint(&request.world_fingerprint)?;
        validate_token("serverKind", &request.server_kind, MAX_CATEGORY_CHARS)?;

        let connection = self.lock_connection();
        let mut statement = connection.prepare(
            "SELECT profile_key, scope_id, display_name, last_opened_at_ms
             FROM world_profiles
             WHERE world_fingerprint = ?1
               AND server_kind = ?2
               AND scope_kind = 'fallback'
               AND identity_quality = 'manual'
             ORDER BY last_opened_at_ms DESC, profile_key
             LIMIT 64",
        )?;
        let candidates = statement
            .query_map(
                params![request.world_fingerprint, request.server_kind],
                |row| {
                    Ok(ManualProfileCandidateV1 {
                        schema_version: 1,
                        profile_key: row.get(0)?,
                        world_fingerprint: request.world_fingerprint.clone(),
                        fallback_profile_id: row.get(1)?,
                        display_name: row.get(2)?,
                        last_opened_at_ms: row.get(3)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ListManualProfilesResultV1 {
            schema_version: 1,
            world_fingerprint: request.world_fingerprint,
            candidates,
        })
    }

    fn get_active_snapshot_locked(&self) -> Result<Option<ProfileSnapshotV1>, StorageError> {
        let active = self.lock_active().clone();
        let Some(active) = active else {
            return Ok(None);
        };
        let connection = self.lock_connection();
        load_snapshot(&connection, &active).map(Some)
    }

    pub fn merge_fog(&self, request: MergeFogRequestV1) -> Result<FogMergeResultV1, StorageError> {
        let _operation = self.lock_operation();
        validate_command_schema(request.schema_version)?;
        if request.cells.len() > MAX_FOG_BATCH {
            return Err(StorageError::Validation(format!(
                "fog batch exceeds {MAX_FOG_BATCH} cells"
            )));
        }
        if !matches!(request.origin.as_str(), "local" | "shared" | "import") {
            return Err(StorageError::Validation("unknown fog origin".to_owned()));
        }
        let active = self.require_active(&request.context)?;
        let unique_cells = validate_and_deduplicate_cells(&request.cells)?;
        let now = now_ms()?;

        let mut connection = self.lock_connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let valid_cells = load_valid_cells(&transaction, &active.world_fingerprint)?;
        for cell in &unique_cells {
            validate_cell_exists(*cell, &valid_cells)?;
        }

        let mut inserted = 0;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT OR IGNORE INTO fog_cells(
                    profile_key, cell_x, cell_y, revealed_at_ms, origin
                 ) VALUES(?1, ?2, ?3, ?4, ?5)",
            )?;
            for cell in &unique_cells {
                inserted += statement.execute(params![
                    active.profile_key,
                    cell.x,
                    cell.y,
                    now,
                    request.origin
                ])?;
            }
        }
        let total = transaction.query_row(
            "SELECT COUNT(*) FROM fog_cells WHERE profile_key = ?1",
            params![active.profile_key],
            |row| row.get::<_, i64>(0),
        )? as usize;
        transaction.commit()?;
        Ok(FogMergeResultV1 { inserted, total })
    }

    pub fn replace_local_markers(
        &self,
        request: ReplaceLocalMarkersRequestV1,
    ) -> Result<MarkerReplaceResultV1, StorageError> {
        let _operation = self.lock_operation();
        validate_command_schema(request.schema_version)?;
        if request.markers.len() > MAX_MARKERS {
            return Err(StorageError::Validation(format!(
                "marker count exceeds {MAX_MARKERS}"
            )));
        }
        let active = self.require_active(&request.context)?;
        let mut ids = HashSet::with_capacity(request.markers.len());
        for marker in &request.markers {
            validate_marker(marker)?;
            if !ids.insert(marker.id.as_str()) {
                return Err(StorageError::Validation(format!(
                    "duplicate marker id {}",
                    marker.id
                )));
            }
        }

        let now = now_ms()?;
        let mut connection = self.lock_connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let valid_cells = load_valid_cells(&transaction, &active.world_fingerprint)?;
        transaction.execute(
            "DELETE FROM marker_records
             WHERE profile_key = ?1 AND scope = 'local'",
            params![active.profile_key],
        )?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO marker_records(
                    profile_key, marker_id, record_kind, scope, version,
                    server_revision, payload_json, updated_at_ms
                 ) VALUES(?1, ?2, 'marker', 'local', 0, NULL, ?3, ?4)",
            )?;
            for marker in &request.markers {
                validate_cell_exists(
                    CellCoordinateV1 {
                        x: marker.cell_x,
                        y: marker.cell_y,
                    },
                    &valid_cells,
                )?;
                let payload = serde_json::to_string(marker)?;
                if payload.len() > MAX_MARKER_JSON_BYTES {
                    return Err(StorageError::Validation(format!(
                        "marker {} is too large",
                        marker.id
                    )));
                }
                statement.execute(params![active.profile_key, marker.id, payload, now])?;
            }
        }
        transaction.commit()?;
        Ok(MarkerReplaceResultV1 {
            stored: request.markers.len(),
        })
    }

    pub fn save_settings(
        &self,
        request: SaveProfileSettingsRequestV1,
    ) -> Result<ProfileSettingsV1, StorageError> {
        let _operation = self.lock_operation();
        validate_command_schema(request.schema_version)?;
        let active = self.require_active(&request.context)?;
        validate_settings(&request.settings)?;
        let poi_json = serde_json::to_string(&request.settings.poi_enabled)?;
        let camera_json = request
            .settings
            .camera
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let serialized = serde_json::to_vec(&request.settings)?;
        if serialized.len() > MAX_SETTINGS_JSON_BYTES {
            return Err(StorageError::Validation(
                "settings are too large".to_owned(),
            ));
        }
        let now = now_ms()?;
        let connection = self.lock_connection();
        connection.execute(
            "INSERT INTO profile_settings(
                profile_key, schema_version, fog_enabled, poi_enabled_json,
                camera_json, updated_at_ms
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(profile_key) DO UPDATE SET
                schema_version = excluded.schema_version,
                fog_enabled = excluded.fog_enabled,
                poi_enabled_json = excluded.poi_enabled_json,
                camera_json = excluded.camera_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                active.profile_key,
                request.settings.schema_version,
                request.settings.fog_enabled,
                poi_json,
                camera_json,
                now
            ],
        )?;
        Ok(request.settings)
    }

    pub fn set_active_route(
        &self,
        request: SetActiveRouteRequestV1,
    ) -> Result<Option<RouteV1>, StorageError> {
        let _operation = self.lock_operation();
        validate_command_schema(request.schema_version)?;
        let active = self.require_active(&request.context)?;

        let mut connection = self.lock_connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let payload = if let Some(route) = request.route.as_ref() {
            let valid_cells = load_valid_cells(&transaction, &active.world_fingerprint)?;
            validate_route(route, &active.world_fingerprint, &valid_cells)?;
            let payload = serde_json::to_string(route)?;
            if payload.len() > MAX_ROUTE_JSON_BYTES {
                return Err(StorageError::Validation(format!(
                    "route payload exceeds {MAX_ROUTE_JSON_BYTES} bytes"
                )));
            }
            Some(payload)
        } else {
            None
        };

        transaction.execute(
            "UPDATE routes SET active = 0 WHERE profile_key = ?1 AND active = 1",
            params![active.profile_key],
        )?;
        if let (Some(route), Some(payload)) = (request.route.as_ref(), payload.as_deref()) {
            transaction.execute(
                "INSERT INTO routes(
                    profile_key, route_id, schema_version, payload_json,
                    generated_at_ms, active
                 ) VALUES(?1, ?2, ?3, ?4, ?5, 1)
                 ON CONFLICT(profile_key, route_id) DO UPDATE SET
                    schema_version = excluded.schema_version,
                    payload_json = excluded.payload_json,
                    generated_at_ms = excluded.generated_at_ms,
                    active = 1",
                params![
                    active.profile_key,
                    route.id,
                    route.schema_version,
                    payload,
                    now_ms()?
                ],
            )?;
        }
        transaction.commit()?;
        Ok(request.route)
    }

    pub fn write_trail_batch(
        &self,
        request: WriteTrailBatchRequestV1,
    ) -> Result<TrailWriteResultV1, StorageError> {
        let _operation = self.lock_operation();
        validate_command_schema(request.schema_version)?;
        let active = self.require_active(&request.context)?;
        validate_trail_batch(&request)?;

        let mut connection = self.lock_connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored_trail = transaction
            .query_row(
                "SELECT session_id, started_at_ms, ended_at_ms
                 FROM trails
                 WHERE profile_key = ?1 AND trail_id = ?2",
                params![active.profile_key, request.trail_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()?;

        let stored_ended_at = if let Some((session_id, started_at_ms, ended_at_ms)) = stored_trail {
            if session_id != active.session_id {
                return Err(StorageError::StaleSession);
            }
            if started_at_ms != request.started_at_ms {
                return Err(StorageError::Validation(
                    "trail startedAtMs cannot change".to_owned(),
                ));
            }
            ended_at_ms
        } else {
            transaction.execute(
                "INSERT INTO trails(
                    profile_key, trail_id, session_id, started_at_ms, ended_at_ms
                 ) VALUES(?1, ?2, ?3, ?4, NULL)",
                params![
                    active.profile_key,
                    request.trail_id,
                    active.session_id,
                    request.started_at_ms
                ],
            )?;
            None
        };

        let last_point = transaction
            .query_row(
                "SELECT sequence, captured_at_ms
                 FROM breadcrumb_points
                 WHERE profile_key = ?1 AND trail_id = ?2
                 ORDER BY sequence DESC
                 LIMIT 1",
                params![active.profile_key, request.trail_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let mut next_sequence = last_point
            .map(|(sequence, _)| {
                u64::try_from(sequence)
                    .map_err(|_| {
                        StorageError::Validation("invalid stored trail sequence".to_owned())
                    })
                    .and_then(|sequence| {
                        sequence.checked_add(1).ok_or_else(|| {
                            StorageError::Validation("trail sequence overflow".to_owned())
                        })
                    })
            })
            .transpose()?
            .unwrap_or(0);
        let mut last_captured_at_ms = last_point.map(|(_, captured_at_ms)| captured_at_ms);
        let mut appended = 0;

        for point in &request.points {
            let sequence = i64::try_from(point.sequence)
                .map_err(|_| StorageError::Validation("trail sequence is too large".to_owned()))?;
            let stored_point = transaction
                .query_row(
                    "SELECT captured_at_ms, world_x, world_y, world_z, break_before
                     FROM breadcrumb_points
                     WHERE profile_key = ?1 AND trail_id = ?2 AND sequence = ?3",
                    params![active.profile_key, request.trail_id, sequence],
                    |row| {
                        Ok(BreadcrumbPointV1 {
                            sequence: point.sequence,
                            captured_at_ms: row.get(0)?,
                            world: WorldPositionV1 {
                                x: row.get(1)?,
                                y: row.get(2)?,
                                z: row.get(3)?,
                            },
                            break_before: row.get(4)?,
                        })
                    },
                )
                .optional()?;

            if let Some(stored_point) = stored_point {
                if stored_point != *point {
                    return Err(StorageError::Validation(format!(
                        "breadcrumb sequence {} conflicts with stored data",
                        point.sequence
                    )));
                }
                continue;
            }
            if stored_ended_at.is_some() {
                return Err(StorageError::Validation(
                    "cannot append to an ended trail".to_owned(),
                ));
            }
            if point.sequence != next_sequence {
                return Err(StorageError::Validation(format!(
                    "breadcrumb sequence {} does not continue at {next_sequence}",
                    point.sequence
                )));
            }
            if last_captured_at_ms.is_some_and(|last| point.captured_at_ms < last) {
                return Err(StorageError::Validation(
                    "breadcrumb capturedAtMs must be monotonic".to_owned(),
                ));
            }
            transaction.execute(
                "INSERT INTO breadcrumb_points(
                    profile_key, trail_id, sequence, captured_at_ms,
                    world_x, world_y, world_z, break_before
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    active.profile_key,
                    request.trail_id,
                    sequence,
                    point.captured_at_ms,
                    point.world.x,
                    point.world.y,
                    point.world.z,
                    point.break_before
                ],
            )?;
            appended += 1;
            next_sequence = next_sequence
                .checked_add(1)
                .ok_or_else(|| StorageError::Validation("trail sequence overflow".to_owned()))?;
            last_captured_at_ms = Some(point.captured_at_ms);
        }

        if let Some(ended_at_ms) = request.ended_at_ms {
            if stored_ended_at.is_some_and(|stored| stored != ended_at_ms) {
                return Err(StorageError::Validation(
                    "trail endedAtMs cannot change".to_owned(),
                ));
            }
            if last_captured_at_ms.is_some_and(|captured| ended_at_ms < captured) {
                return Err(StorageError::Validation(
                    "trail endedAtMs precedes its latest breadcrumb".to_owned(),
                ));
            }
            transaction.execute(
                "UPDATE trails
                 SET ended_at_ms = ?3
                 WHERE profile_key = ?1 AND trail_id = ?2",
                params![active.profile_key, request.trail_id, ended_at_ms],
            )?;
        }

        let (total, ended_at_ms) = transaction.query_row(
            "SELECT
                (SELECT COUNT(*) FROM breadcrumb_points
                 WHERE profile_key = ?1 AND trail_id = ?2),
                ended_at_ms
             FROM trails
             WHERE profile_key = ?1 AND trail_id = ?2",
            params![active.profile_key, request.trail_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
        )?;
        let total = usize::try_from(total)
            .map_err(|_| StorageError::Validation("invalid stored trail size".to_owned()))?;
        transaction.commit()?;
        Ok(TrailWriteResultV1 {
            trail_id: request.trail_id,
            appended,
            total,
            ended_at_ms,
        })
    }

    pub fn accept_telemetry_sequence(
        &self,
        request: AcceptTelemetrySequenceRequestV1,
    ) -> Result<bool, StorageError> {
        let _operation = self.lock_operation();
        validate_command_schema(request.schema_version)?;
        if request.sequence == 0 {
            return Err(StorageError::Validation(
                "telemetry sequence must be positive".to_owned(),
            ));
        }
        let mut active = self.lock_active();
        let Some(active) = active.as_mut() else {
            return Err(StorageError::NoActiveProfile);
        };
        ensure_context_matches(active, &request.context)?;
        if active
            .last_telemetry_sequence
            .is_some_and(|last_sequence| request.sequence <= last_sequence)
        {
            return Ok(false);
        }
        active.last_telemetry_sequence = Some(request.sequence);
        Ok(true)
    }

    fn require_active(
        &self,
        context: &ProfileWriteContextV1,
    ) -> Result<ActiveSession, StorageError> {
        let active = self
            .lock_active()
            .clone()
            .ok_or(StorageError::NoActiveProfile)?;
        ensure_context_matches(&active, context)?;
        ensure_profile_is_writable(&active)?;
        Ok(active)
    }

    fn lock_connection(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_operation(&self) -> std::sync::MutexGuard<'_, ()> {
        self.operation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_active(&self) -> std::sync::MutexGuard<'_, Option<ActiveSession>> {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug)]
struct ResolvedScope {
    kind: String,
    id: String,
    identity_quality: String,
    needs_manual_disambiguation: bool,
}

#[derive(Serialize)]
struct CanonicalLayout<'a> {
    domain: &'static str,
    cell_size: f64,
    bounds: LayoutBoundsInputV1,
    cells: Vec<CanonicalCell<'a>>,
}

#[derive(Serialize)]
struct CanonicalCell<'a> {
    x: i64,
    y: i64,
    tile_uuid: &'a str,
    rotation: u8,
    x_offset: f64,
    y_offset: f64,
    flags: i64,
}

fn configure_connection(connection: &Connection, file_backed: bool) -> Result<(), StorageError> {
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    if file_backed {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    Ok(())
}

fn migrate(connection: &mut Connection) -> Result<(), StorageError> {
    let version =
        connection.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(StorageError::UnsupportedSchema(version));
    }
    if version == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }

    let now = now_ms()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(MIGRATION_V1)?;
    transaction.execute(
        "INSERT INTO schema_migrations(version, name, applied_at_ms)
         VALUES(1, 'profile-storage', ?1)",
        params![now],
    )?;
    transaction.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

fn validate_activate_request(request: &ActivateProfileRequestV1) -> Result<(), StorageError> {
    if request.schema_version != 1 || request.layout.schema_version != 1 {
        return Err(StorageError::Validation(
            "only profile/layout schema version 1 is supported".to_owned(),
        ));
    }
    validate_token("sessionId", &request.session_id, MAX_SESSION_ID_CHARS)?;
    validate_game_mode(&request.game_mode)?;
    validate_token("server.kind", &request.server.kind, MAX_CATEGORY_CHARS)?;
    if let Some(stable_id) = request.server.stable_id.as_deref() {
        validate_token("server.stableId", stable_id, MAX_ID_CHARS)?;
    }
    if let Some(fallback_id) = request.fallback_profile_id.as_deref() {
        validate_manual_profile_id(fallback_id)?;
    }
    if let Some(profile_name) = request.fallback_profile_name.as_deref() {
        if request.fallback_profile_id.is_none()
            || request.server.kind == "local"
            || request.server.stable_id.is_some()
        {
            return Err(StorageError::Validation(
                "fallbackProfileName requires a manual fallback profile".to_owned(),
            ));
        }
        validate_profile_name(profile_name)?;
    }
    validate_layout(&request.layout)
}

fn validate_command_schema(schema_version: u32) -> Result<(), StorageError> {
    if schema_version == 1 {
        Ok(())
    } else {
        Err(StorageError::Validation(
            "only command schema version 1 is supported".to_owned(),
        ))
    }
}

fn validate_game_mode(game_mode: &str) -> Result<(), StorageError> {
    if matches!(game_mode, "survival" | "creative" | "challenge" | "unknown") {
        Ok(())
    } else {
        Err(StorageError::Validation("unknown game mode".to_owned()))
    }
}

fn validate_layout(layout: &LayoutInputV1) -> Result<(), StorageError> {
    if layout.cells.is_empty() || layout.cells.len() > MAX_LAYOUT_CELLS {
        return Err(StorageError::Validation(format!(
            "layout must contain 1..={MAX_LAYOUT_CELLS} cells"
        )));
    }
    if !layout.cell_size.is_finite()
        || layout.cell_size <= 0.0
        || layout.cell_size > MAX_COORDINATE_ABS
    {
        return Err(StorageError::Validation(
            "invalid layout cellSize".to_owned(),
        ));
    }
    validate_bounds(layout.bounds)?;
    let mut coordinates = HashSet::with_capacity(layout.cells.len());
    for cell in &layout.cells {
        validate_cell_coordinate(cell.x, cell.y)?;
        if cell.x < layout.bounds.min_x
            || cell.x > layout.bounds.max_x
            || cell.y < layout.bounds.min_y
            || cell.y > layout.bounds.max_y
        {
            return Err(StorageError::Validation(format!(
                "layout cell {},{} is outside bounds",
                cell.x, cell.y
            )));
        }
        if !coordinates.insert((cell.x, cell.y)) {
            return Err(StorageError::Validation(format!(
                "duplicate layout cell {},{}",
                cell.x, cell.y
            )));
        }
        validate_tile_identity(&cell.tile_uuid)?;
        if cell.rotation > 3 {
            return Err(StorageError::Validation(format!(
                "invalid rotation at {},{}",
                cell.x, cell.y
            )));
        }
        if !cell.x_offset.is_finite()
            || !cell.y_offset.is_finite()
            || cell.x_offset.abs() > MAX_COORDINATE_ABS
            || cell.y_offset.abs() > MAX_COORDINATE_ABS
        {
            return Err(StorageError::Validation(format!(
                "invalid offsets at {},{}",
                cell.x, cell.y
            )));
        }
    }
    Ok(())
}

fn validate_bounds(bounds: LayoutBoundsInputV1) -> Result<(), StorageError> {
    validate_cell_coordinate(bounds.min_x, bounds.min_y)?;
    validate_cell_coordinate(bounds.max_x, bounds.max_y)?;
    if bounds.min_x > bounds.max_x || bounds.min_y > bounds.max_y {
        return Err(StorageError::Validation("invalid layout bounds".to_owned()));
    }
    Ok(())
}

fn validate_settings(settings: &ProfileSettingsV1) -> Result<(), StorageError> {
    if settings.schema_version != 1 {
        return Err(StorageError::Validation(
            "only settings schema version 1 is supported".to_owned(),
        ));
    }
    if settings.poi_enabled.len() > 64 {
        return Err(StorageError::Validation(
            "too many POI categories".to_owned(),
        ));
    }
    let mut categories = HashSet::with_capacity(settings.poi_enabled.len());
    for category in &settings.poi_enabled {
        validate_token("POI category", category, MAX_CATEGORY_CHARS)?;
        if !categories.insert(category.as_str()) {
            return Err(StorageError::Validation(format!(
                "duplicate POI category {category}"
            )));
        }
    }
    if let Some(camera) = settings.camera.as_ref() {
        validate_finite_json(camera)?;
    }
    Ok(())
}

fn validate_finite_json(value: &Value) -> Result<(), StorageError> {
    match value {
        Value::Array(values) => {
            for value in values {
                validate_finite_json(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_finite_json(value)?;
            }
        }
        Value::Number(number) if number.as_f64().is_some_and(|value| !value.is_finite()) => {
            return Err(StorageError::Validation(
                "settings contain a non-finite number".to_owned(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn validate_route(
    route: &RouteV1,
    world_fingerprint: &str,
    valid_cells: &HashSet<CellCoordinateV1>,
) -> Result<(), StorageError> {
    if route.kind != "scrapmap.route" || route.schema_version != 1 {
        return Err(StorageError::Validation(
            "only scrapmap.route schema version 1 is supported".to_owned(),
        ));
    }
    validate_token("route.id", &route.id, MAX_ID_CHARS)?;
    if route.world.world_fingerprint != world_fingerprint {
        return Err(StorageError::StaleSession);
    }
    validate_bounded_text("route.generatedAt", &route.generated_at, MAX_ID_CHARS)?;
    if !matches!(route.strategy.as_str(), "roads" | "direct") {
        return Err(StorageError::Validation(
            "unknown route strategy".to_owned(),
        ));
    }
    if !matches!(route.status.as_str(), "ready" | "partial" | "unreachable") {
        return Err(StorageError::Validation("unknown route status".to_owned()));
    }
    validate_route_endpoint("route.start", &route.start)?;
    validate_route_endpoint("route.destination", &route.destination)?;
    if route.path.len() > MAX_ROUTE_POINTS || route.path_cells.len() > MAX_ROUTE_POINTS {
        return Err(StorageError::Validation(format!(
            "route path exceeds {MAX_ROUTE_POINTS} points"
        )));
    }
    for position in &route.path {
        validate_world_position("route.path", *position)?;
    }
    for cell in &route.path_cells {
        validate_cell_coordinate(cell.x, cell.y)?;
        validate_cell_exists(*cell, valid_cells)?;
    }
    validate_distance(
        "route.directDistanceWorldUnits",
        route.direct_distance_world_units,
    )?;
    if let Some(distance) = route.route_distance_world_units {
        validate_distance("route.routeDistanceWorldUnits", distance)?;
    }
    Ok(())
}

fn validate_route_endpoint(name: &str, endpoint: &RouteEndpointV1) -> Result<(), StorageError> {
    if !matches!(endpoint.kind.as_str(), "poi" | "marker" | "point") {
        return Err(StorageError::Validation(format!(
            "unknown {name} target kind"
        )));
    }
    if let Some(reference_id) = endpoint.reference_id.as_deref() {
        validate_token(&format!("{name}.referenceId"), reference_id, MAX_ID_CHARS)?;
    }
    if let Some(label) = endpoint.label.as_deref() {
        validate_bounded_text(&format!("{name}.label"), label, MAX_LABEL_CHARS)?;
    }
    validate_world_position(&format!("{name}.position"), endpoint.position)
}

fn validate_trail_batch(request: &WriteTrailBatchRequestV1) -> Result<(), StorageError> {
    validate_token("trailId", &request.trail_id, MAX_ID_CHARS)?;
    validate_timestamp("trail.startedAtMs", request.started_at_ms)?;
    if request.points.len() > MAX_BREADCRUMB_BATCH {
        return Err(StorageError::Validation(format!(
            "breadcrumb batch exceeds {MAX_BREADCRUMB_BATCH} points"
        )));
    }
    if let Some(ended_at_ms) = request.ended_at_ms {
        validate_timestamp("trail.endedAtMs", ended_at_ms)?;
        if ended_at_ms < request.started_at_ms {
            return Err(StorageError::Validation(
                "trail endedAtMs precedes startedAtMs".to_owned(),
            ));
        }
    }

    let mut previous_sequence = None;
    let mut previous_captured_at_ms = None;
    for point in &request.points {
        if point.sequence > MAX_JAVASCRIPT_SAFE_INTEGER {
            return Err(StorageError::Validation(
                "breadcrumb sequence exceeds the safe integer range".to_owned(),
            ));
        }
        if previous_sequence.is_some_and(|previous| point.sequence != previous + 1) {
            return Err(StorageError::Validation(
                "breadcrumb batch sequences must be contiguous".to_owned(),
            ));
        }
        validate_timestamp("breadcrumb.capturedAtMs", point.captured_at_ms)?;
        if point.captured_at_ms < request.started_at_ms {
            return Err(StorageError::Validation(
                "breadcrumb capturedAtMs precedes trail start".to_owned(),
            ));
        }
        if request
            .ended_at_ms
            .is_some_and(|ended_at_ms| point.captured_at_ms > ended_at_ms)
        {
            return Err(StorageError::Validation(
                "breadcrumb capturedAtMs follows trail end".to_owned(),
            ));
        }
        if previous_captured_at_ms.is_some_and(|previous| point.captured_at_ms < previous) {
            return Err(StorageError::Validation(
                "breadcrumb capturedAtMs must be monotonic".to_owned(),
            ));
        }
        validate_world_position("breadcrumb.world", point.world)?;
        previous_sequence = Some(point.sequence);
        previous_captured_at_ms = Some(point.captured_at_ms);
    }
    Ok(())
}

fn validate_timestamp(name: &str, value: i64) -> Result<(), StorageError> {
    if value < 0 || u64::try_from(value).is_ok_and(|value| value > MAX_JAVASCRIPT_SAFE_INTEGER) {
        return Err(StorageError::Validation(format!("invalid {name}")));
    }
    Ok(())
}

fn validate_world_position(name: &str, position: WorldPositionV1) -> Result<(), StorageError> {
    if [position.x, position.y, position.z]
        .into_iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_ABS)
    {
        return Err(StorageError::Validation(format!("invalid {name}")));
    }
    Ok(())
}

fn validate_distance(name: &str, value: f64) -> Result<(), StorageError> {
    if !value.is_finite() || !(0.0..=MAX_COORDINATE_ABS * 4.0).contains(&value) {
        return Err(StorageError::Validation(format!("invalid {name}")));
    }
    Ok(())
}

fn validate_bounded_text(name: &str, value: &str, max_chars: usize) -> Result<(), StorageError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > max_chars
        || trimmed.chars().any(char::is_control)
    {
        return Err(StorageError::Validation(format!("invalid {name}")));
    }
    Ok(())
}

fn validate_marker(marker: &LegacyMarkerV1) -> Result<(), StorageError> {
    validate_token("marker.id", &marker.id, MAX_ID_CHARS)?;
    validate_cell_coordinate(marker.cell_x, marker.cell_y)?;
    validate_token("marker.kind", &marker.kind, MAX_CATEGORY_CHARS)?;
    let label = marker.label.trim();
    if label.is_empty()
        || label.chars().count() > MAX_LABEL_CHARS
        || label.chars().any(char::is_control)
    {
        return Err(StorageError::Validation(format!(
            "invalid label for marker {}",
            marker.id
        )));
    }
    if marker
        .created_at
        .as_deref()
        .is_some_and(|timestamp| timestamp.chars().count() > MAX_ID_CHARS)
    {
        return Err(StorageError::Validation(format!(
            "createdAt is too long for marker {}",
            marker.id
        )));
    }
    Ok(())
}

fn validate_token(name: &str, value: &str, max_chars: usize) -> Result<(), StorageError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > max_chars
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:-/".contains(character))
    {
        return Err(StorageError::Validation(format!("invalid {name}")));
    }
    Ok(())
}

fn validate_manual_profile_id(value: &str) -> Result<(), StorageError> {
    const PREFIX: &str = "manual:";
    const UUID_LENGTH: usize = 36;
    const HYPHEN_POSITIONS: [usize; 4] = [8, 13, 18, 23];

    let Some(uuid) = value.strip_prefix(PREFIX) else {
        return Err(StorageError::Validation(
            "fallbackProfileId must be manual:<UUID v4>".to_owned(),
        ));
    };
    let bytes = uuid.as_bytes();
    if bytes.len() != UUID_LENGTH {
        return Err(StorageError::Validation(
            "fallbackProfileId must be manual:<UUID v4>".to_owned(),
        ));
    }

    for (index, byte) in bytes.iter().copied().enumerate() {
        if HYPHEN_POSITIONS.contains(&index) {
            if byte != b'-' {
                return Err(StorageError::Validation(
                    "fallbackProfileId must be manual:<UUID v4>".to_owned(),
                ));
            }
        } else if !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte) {
            return Err(StorageError::Validation(
                "fallbackProfileId must be manual:<UUID v4>".to_owned(),
            ));
        }
    }

    if bytes[14] != b'4' || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b') {
        return Err(StorageError::Validation(
            "fallbackProfileId must be manual:<UUID v4>".to_owned(),
        ));
    }
    Ok(())
}

fn validate_profile_name(value: &str) -> Result<(), StorageError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_PROFILE_NAME_CHARS
        || trimmed.chars().any(char::is_control)
    {
        return Err(StorageError::Validation(
            "invalid manual profile name".to_owned(),
        ));
    }
    Ok(())
}

fn validate_world_fingerprint(value: &str) -> Result<(), StorageError> {
    validate_token("worldFingerprint", value, 80)?;
    if value.len() != 70
        || !value.starts_with("smwf1_")
        || !value[6..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(StorageError::Validation(
            "invalid worldFingerprint".to_owned(),
        ));
    }
    Ok(())
}

fn validate_tile_identity(value: &str) -> Result<(), StorageError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_TILE_ID_CHARS
        || trimmed.chars().any(char::is_control)
    {
        return Err(StorageError::Validation("invalid tile identity".to_owned()));
    }
    Ok(())
}

fn validate_cell_coordinate(x: i64, y: i64) -> Result<(), StorageError> {
    if x.abs() > MAX_CELL_ABS || y.abs() > MAX_CELL_ABS {
        return Err(StorageError::Validation(format!(
            "cell {x},{y} exceeds coordinate limits"
        )));
    }
    Ok(())
}

fn normalize_layout(layout: &mut LayoutInputV1) {
    if layout.cell_size == -0.0 {
        layout.cell_size = 0.0;
    }
    for cell in &mut layout.cells {
        let tile_identity = cell.tile_uuid.trim();
        cell.tile_uuid = if looks_like_uuid(tile_identity) {
            tile_identity.to_ascii_lowercase()
        } else {
            tile_identity.to_owned()
        };
        if cell.x_offset == -0.0 {
            cell.x_offset = 0.0;
        }
        if cell.y_offset == -0.0 {
            cell.y_offset = 0.0;
        }
    }
    layout.cells.sort_unstable_by(|left, right| {
        (left.x, left.y, left.tile_uuid.as_str()).cmp(&(right.x, right.y, right.tile_uuid.as_str()))
    });
}

fn looks_like_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn compute_world_fingerprint(layout: &LayoutInputV1) -> Result<String, StorageError> {
    let canonical = CanonicalLayout {
        domain: "scrapmap-world-v1",
        cell_size: layout.cell_size,
        bounds: layout.bounds,
        cells: layout
            .cells
            .iter()
            .map(|cell| CanonicalCell {
                x: cell.x,
                y: cell.y,
                tile_uuid: &cell.tile_uuid,
                rotation: cell.rotation,
                x_offset: cell.x_offset,
                y_offset: cell.y_offset,
                flags: cell.flags,
            })
            .collect(),
    };
    Ok(format!(
        "smwf1_{}",
        hex_sha256(&serde_json::to_vec(&canonical)?)
    ))
}

fn compute_profile_key(scope_kind: &str, scope_id: &str, world_fingerprint: &str) -> String {
    let material = serde_json::json!([
        "scrapmap-profile-v1",
        scope_kind,
        scope_id,
        world_fingerprint
    ]);
    format!(
        "smp1_{}",
        hex_sha256(&serde_json::to_vec(&material).expect("profile material is serializable"))
    )
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn resolve_scope(
    server: &ServerIdentityInputV1,
    fallback_profile_id: Option<&str>,
) -> Result<ResolvedScope, StorageError> {
    if server.kind == "local" {
        return Ok(ResolvedScope {
            kind: "local".to_owned(),
            id: "default".to_owned(),
            identity_quality: "stable".to_owned(),
            needs_manual_disambiguation: false,
        });
    }
    if let Some(stable_id) = server.stable_id.as_deref() {
        return Ok(ResolvedScope {
            kind: "server".to_owned(),
            id: format!("{}:{stable_id}", server.kind),
            identity_quality: "stable".to_owned(),
            needs_manual_disambiguation: false,
        });
    }
    if let Some(fallback_id) = fallback_profile_id {
        return Ok(ResolvedScope {
            kind: "fallback".to_owned(),
            id: fallback_id.to_owned(),
            identity_quality: "manual".to_owned(),
            needs_manual_disambiguation: false,
        });
    }
    Ok(ResolvedScope {
        kind: "fallback".to_owned(),
        id: format!("{}:default", server.kind),
        identity_quality: "fingerprint-only".to_owned(),
        needs_manual_disambiguation: true,
    })
}

fn store_layout(
    transaction: &Transaction<'_>,
    world_fingerprint: &str,
    game_mode: &str,
    layout_json: &str,
    schema_version: u32,
    now: i64,
) -> Result<(), StorageError> {
    transaction.execute(
        "INSERT INTO world_layouts(
            world_fingerprint, fingerprint_version, layout_schema_version,
            game_mode, payload_json, payload_bytes, created_at_ms, updated_at_ms
         ) VALUES(?1, 1, ?2, ?3, ?4, ?5, ?6, ?6)
         ON CONFLICT(world_fingerprint) DO UPDATE SET
            layout_schema_version = excluded.layout_schema_version,
            game_mode = CASE
                WHEN excluded.game_mode = 'unknown' THEN world_layouts.game_mode
                ELSE excluded.game_mode
            END,
            payload_json = excluded.payload_json,
            payload_bytes = excluded.payload_bytes,
            updated_at_ms = excluded.updated_at_ms",
        params![
            world_fingerprint,
            schema_version,
            game_mode,
            layout_json,
            layout_json.len() as i64,
            now
        ],
    )?;
    Ok(())
}

fn upsert_profile(
    transaction: &Transaction<'_>,
    profile_key: &str,
    world_fingerprint: &str,
    scope: &ResolvedScope,
    request: &ActivateProfileRequestV1,
    now: i64,
) -> Result<(), StorageError> {
    transaction.execute(
        "INSERT INTO world_profiles(
            profile_key, profile_key_version, world_fingerprint,
            scope_kind, scope_id, identity_quality, game_mode, server_kind,
            server_stable_id, display_name, needs_manual_disambiguation,
            created_at_ms, last_opened_at_ms
         ) VALUES(?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
         ON CONFLICT(profile_key) DO UPDATE SET
            last_opened_at_ms = excluded.last_opened_at_ms,
            game_mode = CASE
                WHEN excluded.game_mode = 'unknown' THEN world_profiles.game_mode
                ELSE excluded.game_mode
            END,
            server_kind = excluded.server_kind,
            server_stable_id = excluded.server_stable_id,
            display_name = COALESCE(excluded.display_name, world_profiles.display_name),
            needs_manual_disambiguation = excluded.needs_manual_disambiguation",
        params![
            profile_key,
            world_fingerprint,
            scope.kind,
            scope.id,
            scope.identity_quality,
            request.game_mode,
            request.server.kind,
            request.server.stable_id,
            request.fallback_profile_name,
            scope.needs_manual_disambiguation,
            now
        ],
    )?;
    Ok(())
}

fn ensure_settings(
    transaction: &Transaction<'_>,
    profile_key: &str,
    now: i64,
) -> Result<(), StorageError> {
    let poi_enabled_json = serde_json::to_string(DEFAULT_POI_CATEGORIES)?;
    transaction.execute(
        "INSERT OR IGNORE INTO profile_settings(
            profile_key, schema_version, fog_enabled, poi_enabled_json,
            camera_json, updated_at_ms
         ) VALUES(?1, 1, 1, ?2, NULL, ?3)",
        params![profile_key, poi_enabled_json, now],
    )?;
    Ok(())
}

fn load_snapshot(
    connection: &Connection,
    active: &ActiveSession,
) -> Result<ProfileSnapshotV1, StorageError> {
    let profile = connection.query_row(
        "SELECT
            profile_key, world_fingerprint, scope_kind, scope_id,
            identity_quality, game_mode, server_kind, server_stable_id,
            display_name, needs_manual_disambiguation
         FROM world_profiles WHERE profile_key = ?1",
        params![active.profile_key],
        |row| {
            Ok(WorldProfileV1 {
                schema_version: 1,
                profile_key: row.get(0)?,
                world_fingerprint: row.get(1)?,
                scope_kind: row.get(2)?,
                scope_id: row.get(3)?,
                identity_quality: row.get(4)?,
                game_mode: row.get(5)?,
                server_kind: row.get(6)?,
                server_stable_id: row.get(7)?,
                display_name: row.get(8)?,
                needs_manual_disambiguation: row.get(9)?,
            })
        },
    )?;
    let settings = connection.query_row(
        "SELECT schema_version, fog_enabled, poi_enabled_json, camera_json
         FROM profile_settings WHERE profile_key = ?1",
        params![active.profile_key],
        |row| {
            let poi_json: String = row.get(2)?;
            let camera_json: Option<String> = row.get(3)?;
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, bool>(1)?,
                poi_json,
                camera_json,
            ))
        },
    )?;
    let settings = ProfileSettingsV1 {
        schema_version: settings.0,
        fog_enabled: settings.1,
        poi_enabled: serde_json::from_str(&settings.2)?,
        camera: settings
            .3
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?,
    };

    let mut fog_statement = connection.prepare(
        "SELECT cell_x, cell_y FROM fog_cells
         WHERE profile_key = ?1 ORDER BY cell_x, cell_y",
    )?;
    let visited = fog_statement
        .query_map(params![active.profile_key], |row| {
            Ok(CellCoordinateV1 {
                x: row.get(0)?,
                y: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut marker_statement = connection.prepare(
        "SELECT payload_json FROM marker_records
         WHERE profile_key = ?1 AND record_kind = 'marker' AND scope = 'local'
         ORDER BY updated_at_ms, marker_id",
    )?;
    let marker_json = marker_statement
        .query_map(params![active.profile_key], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let markers = marker_json
        .iter()
        .map(|payload| serde_json::from_str(payload))
        .collect::<Result<Vec<_>, _>>()?;
    let active_route =
        load_active_route(connection, &profile.world_fingerprint, &active.profile_key)?;
    let recent_trail = load_recent_trail(connection, &active.profile_key)?;

    Ok(ProfileSnapshotV1 {
        schema_version: 1,
        profile: profile.clone(),
        session_id: active.session_id.clone(),
        settings,
        visited: LegacyVisitedPayloadV1 {
            schema_version: 1,
            world_id: profile.world_fingerprint.clone(),
            visited,
        },
        markers: LegacyMarkersPayloadV1 {
            schema_version: 1,
            world_id: profile.world_fingerprint,
            markers,
        },
        active_route,
        recent_trail,
    })
}

fn load_active_route(
    connection: &Connection,
    world_fingerprint: &str,
    profile_key: &str,
) -> Result<Option<RouteV1>, StorageError> {
    let payload = connection
        .query_row(
            "SELECT payload_json
             FROM routes
             WHERE profile_key = ?1 AND active = 1",
            params![profile_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(payload) = payload else {
        return Ok(None);
    };
    if payload.len() > MAX_ROUTE_JSON_BYTES {
        return Err(StorageError::Validation(
            "stored active route is too large".to_owned(),
        ));
    }
    let route: RouteV1 = serde_json::from_str(&payload)?;
    let valid_cells = load_valid_cells_from_connection(connection, world_fingerprint)?;
    validate_route(&route, world_fingerprint, &valid_cells)?;
    Ok(Some(route))
}

fn load_recent_trail(
    connection: &Connection,
    profile_key: &str,
) -> Result<Option<RecentTrailV1>, StorageError> {
    let metadata = connection
        .query_row(
            "SELECT trail_id, session_id, started_at_ms, ended_at_ms
             FROM trails
             WHERE profile_key = ?1
             ORDER BY started_at_ms DESC, trail_id DESC
             LIMIT 1",
            params![profile_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((trail_id, session_id, started_at_ms, ended_at_ms)) = metadata else {
        return Ok(None);
    };
    validate_token("stored trailId", &trail_id, MAX_ID_CHARS)?;
    validate_token("stored trail sessionId", &session_id, MAX_SESSION_ID_CHARS)?;
    validate_timestamp("stored trail startedAtMs", started_at_ms)?;
    if let Some(ended_at_ms) = ended_at_ms {
        validate_timestamp("stored trail endedAtMs", ended_at_ms)?;
        if ended_at_ms < started_at_ms {
            return Err(StorageError::Validation(
                "stored trail ends before it starts".to_owned(),
            ));
        }
    }

    let point_count = connection.query_row(
        "SELECT COUNT(*)
         FROM breadcrumb_points
         WHERE profile_key = ?1 AND trail_id = ?2",
        params![profile_key, trail_id],
        |row| row.get::<_, i64>(0),
    )?;
    let point_count = usize::try_from(point_count)
        .map_err(|_| StorageError::Validation("invalid stored trail size".to_owned()))?;
    let mut statement = connection.prepare(
        "SELECT sequence, captured_at_ms, world_x, world_y, world_z, break_before
         FROM breadcrumb_points
         WHERE profile_key = ?1 AND trail_id = ?2
         ORDER BY sequence DESC
         LIMIT ?3",
    )?;
    let rows = statement
        .query_map(
            params![
                profile_key,
                trail_id,
                i64::try_from(MAX_RECENT_TRAIL_POINTS)
                    .expect("recent trail limit fits in SQLite INTEGER")
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, bool>(5)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut points = Vec::with_capacity(rows.len());
    for (sequence, captured_at_ms, x, y, z, break_before) in rows.into_iter().rev() {
        let sequence = u64::try_from(sequence)
            .map_err(|_| StorageError::Validation("invalid stored trail sequence".to_owned()))?;
        if sequence > MAX_JAVASCRIPT_SAFE_INTEGER {
            return Err(StorageError::Validation(
                "stored trail sequence exceeds the safe integer range".to_owned(),
            ));
        }
        validate_timestamp("stored breadcrumb capturedAtMs", captured_at_ms)?;
        let world = WorldPositionV1 { x, y, z };
        validate_world_position("stored breadcrumb world", world)?;
        points.push(BreadcrumbPointV1 {
            sequence,
            captured_at_ms,
            world,
            break_before,
        });
    }

    Ok(Some(RecentTrailV1 {
        schema_version: 1,
        trail_id,
        session_id,
        started_at_ms,
        ended_at_ms,
        point_count,
        truncated: point_count > points.len(),
        points,
    }))
}

fn load_valid_cells(
    transaction: &Transaction<'_>,
    world_fingerprint: &str,
) -> Result<HashSet<CellCoordinateV1>, StorageError> {
    let payload: String = transaction.query_row(
        "SELECT payload_json FROM world_layouts WHERE world_fingerprint = ?1",
        params![world_fingerprint],
        |row| row.get(0),
    )?;
    valid_cells_from_layout_payload(&payload)
}

fn load_valid_cells_from_connection(
    connection: &Connection,
    world_fingerprint: &str,
) -> Result<HashSet<CellCoordinateV1>, StorageError> {
    let payload: String = connection.query_row(
        "SELECT payload_json FROM world_layouts WHERE world_fingerprint = ?1",
        params![world_fingerprint],
        |row| row.get(0),
    )?;
    valid_cells_from_layout_payload(&payload)
}

fn valid_cells_from_layout_payload(
    payload: &str,
) -> Result<HashSet<CellCoordinateV1>, StorageError> {
    let layout: LayoutInputV1 = serde_json::from_str(payload)?;
    Ok(layout
        .cells
        .into_iter()
        .map(|cell| CellCoordinateV1 {
            x: cell.x,
            y: cell.y,
        })
        .collect())
}

fn validate_and_deduplicate_cells(
    cells: &[CellCoordinateV1],
) -> Result<Vec<CellCoordinateV1>, StorageError> {
    let mut unique = HashSet::with_capacity(cells.len());
    let mut result = Vec::with_capacity(cells.len());
    for cell in cells {
        validate_cell_coordinate(cell.x, cell.y)?;
        if unique.insert(*cell) {
            result.push(*cell);
        }
    }
    Ok(result)
}

fn validate_cell_exists(
    cell: CellCoordinateV1,
    valid_cells: &HashSet<CellCoordinateV1>,
) -> Result<(), StorageError> {
    if !valid_cells.contains(&cell) {
        return Err(StorageError::Validation(format!(
            "cell {},{} is outside the active layout",
            cell.x, cell.y
        )));
    }
    Ok(())
}

fn ensure_context_matches(
    active: &ActiveSession,
    context: &ProfileWriteContextV1,
) -> Result<(), StorageError> {
    if active.profile_key != context.profile_key
        || active.world_fingerprint != context.world_fingerprint
        || active.session_id != context.session_id
    {
        return Err(StorageError::StaleSession);
    }
    Ok(())
}

fn ensure_profile_is_writable(active: &ActiveSession) -> Result<(), StorageError> {
    if active.writable {
        Ok(())
    } else {
        Err(StorageError::ProfileResolutionRequired)
    }
}

fn now_ms() -> Result<i64, StorageError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StorageError::Clock)?
        .as_millis();
    i64::try_from(millis).map_err(|_| StorageError::Clock)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(x: i64, y: i64, tile_uuid: &str) -> LayoutCellInputV1 {
        LayoutCellInputV1 {
            x,
            y,
            tile_uuid: tile_uuid.to_owned(),
            rotation: 0,
            x_offset: 0.0,
            y_offset: 0.0,
            flags: 0,
        }
    }

    fn request(
        session_id: &str,
        server_kind: &str,
        stable_id: Option<&str>,
        cells: Vec<LayoutCellInputV1>,
    ) -> ActivateProfileRequestV1 {
        ActivateProfileRequestV1 {
            schema_version: 1,
            session_id: session_id.to_owned(),
            game_mode: "survival".to_owned(),
            server: ServerIdentityInputV1 {
                kind: server_kind.to_owned(),
                stable_id: stable_id.map(str::to_owned),
            },
            fallback_profile_id: None,
            fallback_profile_name: None,
            layout: LayoutInputV1 {
                schema_version: 1,
                world_id: Some("legacy-world".to_owned()),
                cell_size: 64.0,
                bounds: LayoutBoundsInputV1 {
                    min_x: 0,
                    max_x: 2,
                    min_y: 0,
                    max_y: 2,
                },
                cells,
            },
        }
    }

    fn context(snapshot: &ProfileSnapshotV1) -> ProfileWriteContextV1 {
        ProfileWriteContextV1 {
            profile_key: snapshot.profile.profile_key.clone(),
            world_fingerprint: snapshot.profile.world_fingerprint.clone(),
            session_id: snapshot.session_id.clone(),
        }
    }

    fn position(x: f64, y: f64) -> WorldPositionV1 {
        WorldPositionV1 { x, y, z: 0.0 }
    }

    fn route(snapshot: &ProfileSnapshotV1, id: &str, destination: CellCoordinateV1) -> RouteV1 {
        let destination_position = position(
            (destination.x as f64 + 0.5) * 64.0,
            (destination.y as f64 + 0.5) * 64.0,
        );
        RouteV1 {
            kind: "scrapmap.route".to_owned(),
            schema_version: 1,
            id: id.to_owned(),
            world: WorldReferenceV1 {
                world_fingerprint: snapshot.profile.world_fingerprint.clone(),
            },
            generated_at: "2026-07-30T12:00:00.000Z".to_owned(),
            strategy: "direct".to_owned(),
            status: "ready".to_owned(),
            start: RouteEndpointV1 {
                kind: "point".to_owned(),
                reference_id: None,
                label: Some("Current position".to_owned()),
                position: position(32.0, 32.0),
            },
            destination: RouteEndpointV1 {
                kind: "point".to_owned(),
                reference_id: None,
                label: Some("Destination".to_owned()),
                position: destination_position,
            },
            path: vec![position(32.0, 32.0), destination_position],
            path_cells: vec![CellCoordinateV1 { x: 0, y: 0 }, destination],
            direct_distance_world_units: 64.0,
            route_distance_world_units: Some(64.0),
        }
    }

    fn breadcrumb(sequence: u64, captured_at_ms: i64, x: f64) -> BreadcrumbPointV1 {
        BreadcrumbPointV1 {
            sequence,
            captured_at_ms,
            world: position(x, 32.0),
            break_before: false,
        }
    }

    #[test]
    fn migration_is_idempotent_and_complete() {
        let storage = StorageState::open_in_memory().unwrap();
        let connection = storage.lock_connection();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN (
                    'world_layouts', 'world_profiles', 'profile_settings',
                    'fog_cells', 'marker_records', 'routes', 'trails',
                    'breadcrumb_points', 'app_state'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 1);
        assert_eq!(tables, 9);
    }

    #[test]
    fn fingerprint_ignores_cell_order_uuid_case_and_game_mode() {
        let storage = StorageState::open_in_memory().unwrap();
        let first = storage
            .activate_profile(request(
                "session-a",
                "local",
                None,
                vec![
                    cell(0, 0, "00000000-0000-0000-0000-00000000000A"),
                    cell(1, 0, "00000000-0000-0000-0000-00000000000B"),
                ],
            ))
            .unwrap();
        let mut reordered = request(
            "session-b",
            "local",
            None,
            vec![
                cell(1, 0, "00000000-0000-0000-0000-00000000000b"),
                cell(0, 0, "00000000-0000-0000-0000-00000000000a"),
            ],
        );
        reordered.game_mode = "creative".to_owned();
        let second = storage.activate_profile(reordered).unwrap();
        assert_eq!(
            first.profile.world_fingerprint,
            second.profile.world_fingerprint
        );
        assert_eq!(first.profile.profile_key, second.profile.profile_key);
    }

    #[test]
    fn content_data_tile_paths_are_valid_identities() {
        let storage = StorageState::open_in_memory().unwrap();
        let snapshot = storage
            .activate_profile(request(
                "session-a",
                "local",
                None,
                vec![cell(
                    0,
                    0,
                    "$CONTENT_DATA/Terrain/Database/Tile Sets/Forest.tile",
                )],
            ))
            .unwrap();
        assert!(snapshot.profile.world_fingerprint.starts_with("smwf1_"));
    }

    #[test]
    fn profile_switch_a_b_a_restores_isolated_state() {
        let storage = StorageState::open_in_memory().unwrap();
        let a = storage
            .activate_profile(request(
                "a-1",
                "dedicated",
                Some("server-a"),
                vec![cell(0, 0, "a")],
            ))
            .unwrap();
        storage
            .merge_fog(MergeFogRequestV1 {
                schema_version: 1,
                context: context(&a),
                cells: vec![CellCoordinateV1 { x: 0, y: 0 }],
                origin: "local".to_owned(),
            })
            .unwrap();
        storage
            .save_settings(SaveProfileSettingsRequestV1 {
                schema_version: 1,
                context: context(&a),
                settings: ProfileSettingsV1 {
                    schema_version: 1,
                    fog_enabled: false,
                    poi_enabled: vec!["schematic".to_owned()],
                    camera: None,
                },
            })
            .unwrap();
        storage
            .replace_local_markers(ReplaceLocalMarkersRequestV1 {
                schema_version: 1,
                context: context(&a),
                markers: vec![LegacyMarkerV1 {
                    id: "marker-a".to_owned(),
                    cell_x: 0,
                    cell_y: 0,
                    kind: "x".to_owned(),
                    label: "Only A".to_owned(),
                    created_at: None,
                }],
            })
            .unwrap();

        let b = storage
            .activate_profile(request(
                "b-1",
                "dedicated",
                Some("server-b"),
                vec![cell(0, 0, "a")],
            ))
            .unwrap();
        assert_ne!(a.profile.profile_key, b.profile.profile_key);
        assert!(b.visited.visited.is_empty());
        assert!(b.settings.fog_enabled);
        assert!(b.markers.markers.is_empty());

        let a_again = storage
            .activate_profile(request(
                "a-2",
                "dedicated",
                Some("server-a"),
                vec![cell(0, 0, "a")],
            ))
            .unwrap();
        assert_eq!(a.profile.profile_key, a_again.profile.profile_key);
        assert_eq!(
            a_again.visited.visited,
            vec![CellCoordinateV1 { x: 0, y: 0 }]
        );
        assert!(!a_again.settings.fog_enabled);
        assert_eq!(a_again.settings.poi_enabled, vec!["schematic"]);
        assert_eq!(a_again.markers.markers.len(), 1);
    }

    #[test]
    fn unresolved_remote_profile_is_read_only_until_manually_split() {
        let storage = StorageState::open_in_memory().unwrap();
        let snapshot = storage
            .activate_profile(request(
                "unresolved",
                "unknown",
                None,
                vec![cell(0, 0, "a")],
            ))
            .unwrap();
        assert!(snapshot.profile.needs_manual_disambiguation);
        assert_eq!(snapshot.profile.identity_quality, "fingerprint-only");

        assert!(matches!(
            storage.merge_fog(MergeFogRequestV1 {
                schema_version: 1,
                context: context(&snapshot),
                cells: vec![CellCoordinateV1 { x: 0, y: 0 }],
                origin: "local".to_owned(),
            }),
            Err(StorageError::ProfileResolutionRequired)
        ));
        assert!(matches!(
            storage.save_settings(SaveProfileSettingsRequestV1 {
                schema_version: 1,
                context: context(&snapshot),
                settings: ProfileSettingsV1::default(),
            }),
            Err(StorageError::ProfileResolutionRequired)
        ));
        assert!(matches!(
            storage.replace_local_markers(ReplaceLocalMarkersRequestV1 {
                schema_version: 1,
                context: context(&snapshot),
                markers: vec![],
            }),
            Err(StorageError::ProfileResolutionRequired)
        ));

        let restored = storage.get_active_snapshot().unwrap().unwrap();
        assert!(restored.visited.visited.is_empty());
        assert!(restored.markers.markers.is_empty());
    }

    #[test]
    fn manual_profiles_are_listed_and_restore_isolated_state() {
        let storage = StorageState::open_in_memory().unwrap();
        let mut manual_a = request("manual-a-1", "unknown", None, vec![cell(0, 0, "a")]);
        manual_a.fallback_profile_id =
            Some("manual:00000000-0000-4000-8000-00000000000a".to_owned());
        manual_a.fallback_profile_name = Some("Сервер друга".to_owned());
        let a = storage.activate_profile(manual_a).unwrap();
        storage
            .merge_fog(MergeFogRequestV1 {
                schema_version: 1,
                context: context(&a),
                cells: vec![CellCoordinateV1 { x: 0, y: 0 }],
                origin: "local".to_owned(),
            })
            .unwrap();

        let mut manual_b = request("manual-b-1", "unknown", None, vec![cell(0, 0, "a")]);
        manual_b.fallback_profile_id =
            Some("manual:00000000-0000-4000-8000-00000000000b".to_owned());
        manual_b.fallback_profile_name = Some("Другой сервер".to_owned());
        let b = storage.activate_profile(manual_b).unwrap();
        assert_ne!(a.profile.profile_key, b.profile.profile_key);
        assert!(b.visited.visited.is_empty());

        storage
            .activate_profile(request(
                "local-same-layout",
                "local",
                None,
                vec![cell(0, 0, "a")],
            ))
            .unwrap();
        let mut other_world = request("manual-other", "unknown", None, vec![cell(0, 0, "other")]);
        other_world.fallback_profile_id =
            Some("manual:00000000-0000-4000-8000-00000000000c".to_owned());
        other_world.fallback_profile_name = Some("Другой мир".to_owned());
        storage.activate_profile(other_world).unwrap();

        let listed = storage
            .list_manual_profiles(ListManualProfilesRequestV1 {
                schema_version: 1,
                world_fingerprint: a.profile.world_fingerprint.clone(),
                server_kind: "unknown".to_owned(),
            })
            .unwrap();
        assert_eq!(listed.candidates.len(), 2);
        assert!(listed
            .candidates
            .iter()
            .any(|candidate| candidate.fallback_profile_id
                == "manual:00000000-0000-4000-8000-00000000000a"
                && candidate.display_name.as_deref() == Some("Сервер друга")));

        let mut manual_a_again = request("manual-a-2", "unknown", None, vec![cell(0, 0, "a")]);
        manual_a_again.fallback_profile_id =
            Some("manual:00000000-0000-4000-8000-00000000000a".to_owned());
        let restored = storage.activate_profile(manual_a_again).unwrap();
        assert_eq!(restored.profile.profile_key, a.profile.profile_key);
        assert_eq!(
            restored.visited.visited,
            vec![CellCoordinateV1 { x: 0, y: 0 }]
        );
        assert_eq!(
            restored.profile.display_name.as_deref(),
            Some("Сервер друга")
        );
    }

    #[test]
    fn manual_fallback_id_rejects_reserved_aliases_and_non_v4_uuids() {
        let storage = StorageState::open_in_memory().unwrap();
        let unresolved = storage
            .activate_profile(request(
                "unresolved",
                "unknown",
                None,
                vec![cell(0, 0, "a")],
            ))
            .unwrap();

        for invalid_id in [
            "unknown:default",
            "manual:a",
            "manual:00000000-0000-1000-8000-000000000001",
            "manual:00000000-0000-4000-7000-000000000001",
            "manual:00000000-0000-4000-8000-00000000000A",
        ] {
            let mut invalid = request("invalid-manual", "unknown", None, vec![cell(0, 0, "a")]);
            invalid.fallback_profile_id = Some(invalid_id.to_owned());
            assert!(matches!(
                storage.activate_profile(invalid),
                Err(StorageError::Validation(_))
            ));
        }

        let active = storage.get_active_snapshot().unwrap().unwrap();
        assert_eq!(active.session_id, unresolved.session_id);
        assert!(active.profile.needs_manual_disambiguation);
        assert!(matches!(
            storage.merge_fog(MergeFogRequestV1 {
                schema_version: 1,
                context: context(&active),
                cells: vec![CellCoordinateV1 { x: 0, y: 0 }],
                origin: "local".to_owned(),
            }),
            Err(StorageError::ProfileResolutionRequired)
        ));
    }

    #[test]
    fn stale_session_and_out_of_order_telemetry_sequences_are_rejected() {
        let storage = StorageState::open_in_memory().unwrap();
        let old = storage
            .activate_profile(request("session-old", "local", None, vec![cell(0, 0, "a")]))
            .unwrap();
        assert!(storage
            .accept_telemetry_sequence(AcceptTelemetrySequenceRequestV1 {
                schema_version: 1,
                context: context(&old),
                sequence: 4,
            })
            .unwrap());
        assert!(!storage
            .accept_telemetry_sequence(AcceptTelemetrySequenceRequestV1 {
                schema_version: 1,
                context: context(&old),
                sequence: 4,
            })
            .unwrap());

        let new = storage
            .activate_profile(request("session-new", "local", None, vec![cell(0, 0, "a")]))
            .unwrap();
        assert!(matches!(
            storage.accept_telemetry_sequence(AcceptTelemetrySequenceRequestV1 {
                schema_version: 1,
                context: context(&old),
                sequence: 5,
            }),
            Err(StorageError::StaleSession)
        ));
        assert!(storage
            .accept_telemetry_sequence(AcceptTelemetrySequenceRequestV1 {
                schema_version: 1,
                context: context(&new),
                sequence: 1,
            })
            .unwrap());
    }

    #[test]
    fn telemetry_sequence_gate_remains_available_in_read_only_quarantine() {
        let storage = StorageState::open_in_memory().unwrap();
        let unresolved = storage
            .activate_profile(request(
                "unresolved",
                "unknown",
                None,
                vec![cell(0, 0, "a")],
            ))
            .unwrap();

        assert!(storage
            .accept_telemetry_sequence(AcceptTelemetrySequenceRequestV1 {
                schema_version: 1,
                context: context(&unresolved),
                sequence: 1,
            })
            .unwrap());
        assert!(!storage
            .accept_telemetry_sequence(AcceptTelemetrySequenceRequestV1 {
                schema_version: 1,
                context: context(&unresolved),
                sequence: 1,
            })
            .unwrap());
    }

    #[test]
    fn queued_mutation_rechecks_session_after_serialized_switch() {
        use std::sync::{mpsc, Arc};

        let storage = Arc::new(StorageState::open_in_memory().unwrap());
        let old = storage
            .activate_profile(request("session-old", "local", None, vec![cell(0, 0, "a")]))
            .unwrap();
        let operation = storage.lock_operation();
        let (started_sender, started_receiver) = mpsc::channel();
        let worker_storage = Arc::clone(&storage);
        let old_context = context(&old);
        let worker = std::thread::spawn(move || {
            started_sender.send(()).unwrap();
            worker_storage.merge_fog(MergeFogRequestV1 {
                schema_version: 1,
                context: old_context,
                cells: vec![CellCoordinateV1 { x: 0, y: 0 }],
                origin: "local".to_owned(),
            })
        });

        started_receiver.recv().unwrap();
        storage
            .lock_active()
            .as_mut()
            .expect("profile is active")
            .session_id = "session-new".to_owned();
        drop(operation);

        assert!(matches!(
            worker.join().unwrap(),
            Err(StorageError::StaleSession)
        ));
        assert!(storage
            .get_active_snapshot()
            .unwrap()
            .unwrap()
            .visited
            .visited
            .is_empty());
    }

    #[test]
    fn fog_merge_is_a_bounded_idempotent_union() {
        let storage = StorageState::open_in_memory().unwrap();
        let snapshot = storage
            .activate_profile(request(
                "session-a",
                "local",
                None,
                vec![cell(0, 0, "a"), cell(1, 0, "b")],
            ))
            .unwrap();
        let write = MergeFogRequestV1 {
            schema_version: 1,
            context: context(&snapshot),
            cells: vec![
                CellCoordinateV1 { x: 0, y: 0 },
                CellCoordinateV1 { x: 0, y: 0 },
                CellCoordinateV1 { x: 1, y: 0 },
            ],
            origin: "local".to_owned(),
        };
        assert_eq!(
            storage.merge_fog(write.clone()).unwrap(),
            FogMergeResultV1 {
                inserted: 2,
                total: 2
            }
        );
        assert_eq!(
            storage.merge_fog(write).unwrap(),
            FogMergeResultV1 {
                inserted: 0,
                total: 2
            }
        );
    }

    #[test]
    fn local_marker_replace_is_transactional() {
        let storage = StorageState::open_in_memory().unwrap();
        let snapshot = storage
            .activate_profile(request(
                "session-a",
                "local",
                None,
                vec![cell(0, 0, "a"), cell(1, 0, "b")],
            ))
            .unwrap();
        let marker = LegacyMarkerV1 {
            id: "marker-a".to_owned(),
            cell_x: 0,
            cell_y: 0,
            kind: "x".to_owned(),
            label: "Home".to_owned(),
            created_at: Some("2026-01-01T00:00:00Z".to_owned()),
        };
        storage
            .replace_local_markers(ReplaceLocalMarkersRequestV1 {
                schema_version: 1,
                context: context(&snapshot),
                markers: vec![marker.clone()],
            })
            .unwrap();
        assert_eq!(
            storage
                .get_active_snapshot()
                .unwrap()
                .unwrap()
                .markers
                .markers,
            vec![marker]
        );

        let invalid = LegacyMarkerV1 {
            id: "marker-invalid".to_owned(),
            cell_x: 2,
            cell_y: 2,
            kind: "x".to_owned(),
            label: "Outside".to_owned(),
            created_at: None,
        };
        assert!(storage
            .replace_local_markers(ReplaceLocalMarkersRequestV1 {
                schema_version: 1,
                context: context(&snapshot),
                markers: vec![invalid],
            })
            .is_err());
        assert_eq!(
            storage
                .get_active_snapshot()
                .unwrap()
                .unwrap()
                .markers
                .markers
                .len(),
            1
        );
    }

    #[test]
    fn active_route_is_isolated_across_profiles_and_can_be_cleared() {
        let storage = StorageState::open_in_memory().unwrap();
        let a = storage
            .activate_profile(request(
                "a-1",
                "dedicated",
                Some("server-a"),
                vec![cell(0, 0, "a"), cell(1, 0, "b")],
            ))
            .unwrap();
        let route_a = route(&a, "route-a", CellCoordinateV1 { x: 1, y: 0 });
        assert_eq!(
            storage
                .set_active_route(SetActiveRouteRequestV1 {
                    schema_version: 1,
                    context: context(&a),
                    route: Some(route_a.clone()),
                })
                .unwrap(),
            Some(route_a.clone())
        );

        let b = storage
            .activate_profile(request(
                "b-1",
                "dedicated",
                Some("server-b"),
                vec![cell(0, 0, "a"), cell(1, 0, "b")],
            ))
            .unwrap();
        assert!(b.active_route.is_none());

        let a_again = storage
            .activate_profile(request(
                "a-2",
                "dedicated",
                Some("server-a"),
                vec![cell(0, 0, "a"), cell(1, 0, "b")],
            ))
            .unwrap();
        assert_eq!(a_again.active_route, Some(route_a));
        storage
            .set_active_route(SetActiveRouteRequestV1 {
                schema_version: 1,
                context: context(&a_again),
                route: None,
            })
            .unwrap();
        assert!(storage
            .get_active_snapshot()
            .unwrap()
            .unwrap()
            .active_route
            .is_none());
    }

    #[test]
    fn route_validation_is_atomic_and_route_writes_obey_profile_gates() {
        let storage = StorageState::open_in_memory().unwrap();
        let old = storage
            .activate_profile(request(
                "session-old",
                "local",
                None,
                vec![cell(0, 0, "a"), cell(1, 0, "b")],
            ))
            .unwrap();
        let valid = route(&old, "valid", CellCoordinateV1 { x: 1, y: 0 });
        storage
            .set_active_route(SetActiveRouteRequestV1 {
                schema_version: 1,
                context: context(&old),
                route: Some(valid.clone()),
            })
            .unwrap();

        let mut invalid = route(&old, "invalid", CellCoordinateV1 { x: 1, y: 0 });
        invalid.path_cells.push(CellCoordinateV1 { x: 2, y: 2 });
        assert!(matches!(
            storage.set_active_route(SetActiveRouteRequestV1 {
                schema_version: 1,
                context: context(&old),
                route: Some(invalid),
            }),
            Err(StorageError::Validation(_))
        ));
        let mut non_finite = route(&old, "non-finite", CellCoordinateV1 { x: 1, y: 0 });
        non_finite.path[0].x = f64::NAN;
        assert!(matches!(
            storage.set_active_route(SetActiveRouteRequestV1 {
                schema_version: 1,
                context: context(&old),
                route: Some(non_finite),
            }),
            Err(StorageError::Validation(_))
        ));
        assert_eq!(
            storage.get_active_snapshot().unwrap().unwrap().active_route,
            Some(valid)
        );

        let current = storage
            .activate_profile(request(
                "session-current",
                "local",
                None,
                vec![cell(0, 0, "a"), cell(1, 0, "b")],
            ))
            .unwrap();
        assert!(matches!(
            storage.set_active_route(SetActiveRouteRequestV1 {
                schema_version: 1,
                context: context(&old),
                route: None,
            }),
            Err(StorageError::StaleSession)
        ));

        let unresolved = storage
            .activate_profile(request(
                "unresolved",
                "unknown",
                None,
                vec![cell(0, 0, "a"), cell(1, 0, "b")],
            ))
            .unwrap();
        assert!(matches!(
            storage.set_active_route(SetActiveRouteRequestV1 {
                schema_version: 1,
                context: context(&unresolved),
                route: Some(route(
                    &unresolved,
                    "quarantine",
                    CellCoordinateV1 { x: 1, y: 0 },
                )),
            }),
            Err(StorageError::ProfileResolutionRequired)
        ));
        assert_ne!(current.profile.profile_key, unresolved.profile.profile_key);
    }

    #[test]
    fn trail_batches_append_idempotently_and_reject_gaps_conflicts_and_old_sessions() {
        let storage = StorageState::open_in_memory().unwrap();
        let first = storage
            .activate_profile(request("session-a", "local", None, vec![cell(0, 0, "a")]))
            .unwrap();
        let initial = WriteTrailBatchRequestV1 {
            schema_version: 1,
            context: context(&first),
            trail_id: "trail-a".to_owned(),
            started_at_ms: 1_000,
            ended_at_ms: None,
            points: vec![breadcrumb(0, 1_000, 32.0), breadcrumb(1, 1_100, 40.0)],
        };
        assert_eq!(
            storage.write_trail_batch(initial.clone()).unwrap(),
            TrailWriteResultV1 {
                trail_id: "trail-a".to_owned(),
                appended: 2,
                total: 2,
                ended_at_ms: None,
            }
        );
        assert_eq!(
            storage.write_trail_batch(initial.clone()).unwrap().appended,
            0
        );

        let mut conflict = initial.clone();
        conflict.points = vec![breadcrumb(1, 1_100, 99.0)];
        assert!(matches!(
            storage.write_trail_batch(conflict),
            Err(StorageError::Validation(_))
        ));
        let mut gap = initial.clone();
        gap.points = vec![breadcrumb(3, 1_200, 48.0)];
        assert!(matches!(
            storage.write_trail_batch(gap),
            Err(StorageError::Validation(_))
        ));
        let mut non_finite = breadcrumb(2, 1_200, 48.0);
        non_finite.world.x = f64::INFINITY;
        assert!(matches!(
            storage.write_trail_batch(WriteTrailBatchRequestV1 {
                schema_version: 1,
                context: context(&first),
                trail_id: "trail-a".to_owned(),
                started_at_ms: 1_000,
                ended_at_ms: None,
                points: vec![non_finite],
            }),
            Err(StorageError::Validation(_))
        ));
        assert_eq!(
            storage
                .get_active_snapshot()
                .unwrap()
                .unwrap()
                .recent_trail
                .unwrap()
                .point_count,
            2
        );

        let ended = storage
            .write_trail_batch(WriteTrailBatchRequestV1 {
                schema_version: 1,
                context: context(&first),
                trail_id: "trail-a".to_owned(),
                started_at_ms: 1_000,
                ended_at_ms: Some(1_200),
                points: vec![],
            })
            .unwrap();
        assert_eq!(ended.ended_at_ms, Some(1_200));
        assert!(matches!(
            storage.write_trail_batch(WriteTrailBatchRequestV1 {
                schema_version: 1,
                context: context(&first),
                trail_id: "trail-a".to_owned(),
                started_at_ms: 1_000,
                ended_at_ms: Some(1_300),
                points: vec![],
            }),
            Err(StorageError::Validation(_))
        ));

        let second = storage
            .activate_profile(request("session-b", "local", None, vec![cell(0, 0, "a")]))
            .unwrap();
        assert!(matches!(
            storage.write_trail_batch(WriteTrailBatchRequestV1 {
                schema_version: 1,
                context: context(&second),
                trail_id: "trail-a".to_owned(),
                started_at_ms: 1_000,
                ended_at_ms: None,
                points: vec![],
            }),
            Err(StorageError::StaleSession)
        ));
        assert!(matches!(
            storage.write_trail_batch(WriteTrailBatchRequestV1 {
                schema_version: 1,
                context: context(&first),
                trail_id: "stale-context".to_owned(),
                started_at_ms: 2_000,
                ended_at_ms: None,
                points: vec![],
            }),
            Err(StorageError::StaleSession)
        ));
    }

    #[test]
    fn trail_writes_are_rejected_in_quarantine() {
        let storage = StorageState::open_in_memory().unwrap();
        let unresolved = storage
            .activate_profile(request(
                "unresolved",
                "unknown",
                None,
                vec![cell(0, 0, "a")],
            ))
            .unwrap();
        assert!(matches!(
            storage.write_trail_batch(WriteTrailBatchRequestV1 {
                schema_version: 1,
                context: context(&unresolved),
                trail_id: "trail-a".to_owned(),
                started_at_ms: 1_000,
                ended_at_ms: None,
                points: vec![breadcrumb(0, 1_000, 32.0)],
            }),
            Err(StorageError::ProfileResolutionRequired)
        ));
        assert!(storage
            .get_active_snapshot()
            .unwrap()
            .unwrap()
            .recent_trail
            .is_none());
    }

    #[test]
    fn recent_trail_snapshot_is_bounded_to_the_latest_points() {
        let storage = StorageState::open_in_memory().unwrap();
        let snapshot = storage
            .activate_profile(request("session-a", "local", None, vec![cell(0, 0, "a")]))
            .unwrap();
        let points = (0..MAX_BREADCRUMB_BATCH)
            .map(|sequence| {
                breadcrumb(
                    u64::try_from(sequence).unwrap(),
                    1_000 + i64::try_from(sequence).unwrap(),
                    32.0,
                )
            })
            .collect();
        storage
            .write_trail_batch(WriteTrailBatchRequestV1 {
                schema_version: 1,
                context: context(&snapshot),
                trail_id: "long-trail".to_owned(),
                started_at_ms: 1_000,
                ended_at_ms: None,
                points,
            })
            .unwrap();
        storage
            .write_trail_batch(WriteTrailBatchRequestV1 {
                schema_version: 1,
                context: context(&snapshot),
                trail_id: "long-trail".to_owned(),
                started_at_ms: 1_000,
                ended_at_ms: None,
                points: vec![breadcrumb(
                    u64::try_from(MAX_BREADCRUMB_BATCH).unwrap(),
                    1_000 + i64::try_from(MAX_BREADCRUMB_BATCH).unwrap(),
                    40.0,
                )],
            })
            .unwrap();

        let trail = storage
            .get_active_snapshot()
            .unwrap()
            .unwrap()
            .recent_trail
            .unwrap();
        assert_eq!(trail.point_count, MAX_RECENT_TRAIL_POINTS + 1);
        assert_eq!(trail.points.len(), MAX_RECENT_TRAIL_POINTS);
        assert!(trail.truncated);
        assert_eq!(trail.points.first().unwrap().sequence, 1);
        assert_eq!(
            trail.points.last().unwrap().sequence,
            u64::try_from(MAX_RECENT_TRAIL_POINTS).unwrap()
        );
    }

    #[test]
    fn file_database_restores_profile_data_after_reopen() {
        let unique = format!(
            "scrapmap-storage-{}-{}.sqlite3",
            std::process::id(),
            now_ms().unwrap()
        );
        let path = env::temp_dir().join(unique);
        let profile_key;
        {
            let storage = StorageState::open(&path).unwrap();
            let snapshot = storage
                .activate_profile(request("session-a", "local", None, vec![cell(0, 0, "a")]))
                .unwrap();
            profile_key = snapshot.profile.profile_key.clone();
            storage
                .merge_fog(MergeFogRequestV1 {
                    schema_version: 1,
                    context: context(&snapshot),
                    cells: vec![CellCoordinateV1 { x: 0, y: 0 }],
                    origin: "local".to_owned(),
                })
                .unwrap();
            storage
                .set_active_route(SetActiveRouteRequestV1 {
                    schema_version: 1,
                    context: context(&snapshot),
                    route: Some(route(
                        &snapshot,
                        "persisted-route",
                        CellCoordinateV1 { x: 0, y: 0 },
                    )),
                })
                .unwrap();
            storage
                .write_trail_batch(WriteTrailBatchRequestV1 {
                    schema_version: 1,
                    context: context(&snapshot),
                    trail_id: "persisted-trail".to_owned(),
                    started_at_ms: 1_000,
                    ended_at_ms: Some(1_100),
                    points: vec![breadcrumb(0, 1_000, 32.0), breadcrumb(1, 1_100, 40.0)],
                })
                .unwrap();
        }
        {
            let storage = StorageState::open(&path).unwrap();
            let snapshot = storage
                .activate_profile(request("session-b", "local", None, vec![cell(0, 0, "a")]))
                .unwrap();
            assert_eq!(snapshot.profile.profile_key, profile_key);
            assert_eq!(snapshot.visited.visited.len(), 1);
            assert_eq!(
                snapshot
                    .active_route
                    .as_ref()
                    .map(|route| route.id.as_str()),
                Some("persisted-route")
            );
            let trail = snapshot.recent_trail.unwrap();
            assert_eq!(trail.trail_id, "persisted-trail");
            assert_eq!(trail.point_count, 2);
            assert_eq!(trail.points.len(), 2);
            assert_eq!(trail.ended_at_ms, Some(1_100));
        }
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn newer_database_schema_fails_closed() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(
            migrate(&mut connection),
            Err(StorageError::UnsupportedSchema(99))
        ));
    }

    #[test]
    fn absent_active_profile_returns_none() {
        let storage = StorageState::open_in_memory().unwrap();
        assert!(storage.get_active_snapshot().unwrap().is_none());
    }

    #[test]
    fn last_profile_key_is_written() {
        let storage = StorageState::open_in_memory().unwrap();
        let snapshot = storage
            .activate_profile(request("session-a", "local", None, vec![cell(0, 0, "a")]))
            .unwrap();
        let stored: Option<String> = storage
            .lock_connection()
            .query_row(
                "SELECT last_profile_key FROM app_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            stored.as_deref(),
            Some(snapshot.profile.profile_key.as_str())
        );
    }
}
