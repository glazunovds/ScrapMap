use std::{
    collections::hash_map::DefaultHasher,
    env,
    ffi::{OsStr, OsString},
    fmt,
    fs::File,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Serialize;
use serde_json::{Map, Value};

pub const TELEMETRY_FILE_ARGUMENT: &str = "--telemetry-file";
pub const TELEMETRY_FILE_ENVIRONMENT: &str = "SCRAPMAP_TELEMETRY_FILE";
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

const MAX_FILE_SIZE: u64 = 1024 * 1024;
const MAX_PLAYERS: usize = 64;
const MAX_NAME_CHARS: usize = 80;
const MAX_ID_CHARS: usize = 128;
const MAX_WORLD_ID_CHARS: usize = 160;
const MAX_COORDINATE_ABS: f64 = 10_000_000.0;
const MAX_VECTOR_ABS: f64 = 1_000_000.0;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySnapshot {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_world_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_tick: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_player_id: Option<String>,
    pub stale_after_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<TelemetrySource>,
    pub player: TelemetryPlayer,
    pub players: Vec<TelemetryPlayer>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub enumeration: String,
    pub is_host: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryPlayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub is_local: bool,
    pub active: bool,
    pub has_character: bool,
    pub same_world: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_world_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dy: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dz: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vy: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vz: Option<f64>,
}

impl TelemetryPlayer {
    fn has_position(&self) -> bool {
        self.x.is_some() && self.y.is_some() && self.z.is_some()
    }

    fn is_renderable_local_player(&self) -> bool {
        self.is_local && self.active && self.has_character && self.same_world && self.has_position()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticIssue {
    pub kind: DiagnosticIssueKind,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticIssueKind {
    Missing,
    Busy,
    TooLarge,
    InvalidJson,
    InvalidPayload,
    Unsupported,
    Io,
}

impl DiagnosticIssue {
    fn new(kind: DiagnosticIssueKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for DiagnosticIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiagnosticPoll {
    Updated(Box<TelemetrySnapshot>),
    Unchanged,
    Waiting(DiagnosticIssue),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TelemetryPathError {
    MissingArgumentValue,
    DuplicateArgument,
    EmptyOverride,
    MissingLocalAppData,
}

impl fmt::Display for TelemetryPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingArgumentValue => "--telemetry-file requires a path",
            Self::DuplicateArgument => "--telemetry-file may only be provided once",
            Self::EmptyOverride => "the telemetry file override is empty",
            Self::MissingLocalAppData => "LOCALAPPDATA is unavailable",
        })
    }
}

impl std::error::Error for TelemetryPathError {}

#[derive(Clone, Debug)]
struct FileStamp {
    length: u64,
    fingerprint: u64,
}

pub struct DiagnosticSource {
    path: PathBuf,
    last_stamp: Option<FileStamp>,
    last_snapshot: Option<TelemetrySnapshot>,
}

impl DiagnosticSource {
    pub fn from_process() -> Result<Self, TelemetryPathError> {
        resolve_telemetry_path().map(Self::new)
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            last_stamp: None,
            last_snapshot: None,
        }
    }

    #[cfg(test)]
    pub fn last_snapshot(&self) -> Option<&TelemetrySnapshot> {
        self.last_snapshot.as_ref()
    }

    pub fn poll(&mut self) -> DiagnosticPoll {
        let bytes = match read_stable_file(&self.path) {
            Ok(bytes) => bytes,
            Err(issue) => return DiagnosticPoll::Waiting(issue),
        };

        let fingerprint = fingerprint(&bytes);
        if self.last_stamp.as_ref().is_some_and(|stamp| {
            stamp.fingerprint == fingerprint && stamp.length == bytes.len() as u64
        }) {
            return DiagnosticPoll::Unchanged;
        }

        let value: Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                return DiagnosticPoll::Waiting(DiagnosticIssue::new(
                    DiagnosticIssueKind::InvalidJson,
                    format!(
                        "telemetry JSON is incomplete or invalid at line {}, column {}",
                        error.line(),
                        error.column()
                    ),
                ));
            }
        };

        let snapshot = match validate_snapshot(&value) {
            Ok(snapshot) => snapshot,
            Err(detail) => {
                return DiagnosticPoll::Waiting(DiagnosticIssue::new(
                    DiagnosticIssueKind::InvalidPayload,
                    detail,
                ));
            }
        };

        if snapshot
            .source
            .as_ref()
            .is_some_and(|source| source.compatibility.as_deref() == Some("unsupported"))
        {
            return DiagnosticPoll::Waiting(DiagnosticIssue::new(
                DiagnosticIssueKind::Unsupported,
                "the native read-only provider does not support this game build",
            ));
        }

        self.last_stamp = Some(FileStamp {
            length: bytes.len() as u64,
            fingerprint,
        });
        self.last_snapshot = Some(snapshot.clone());
        DiagnosticPoll::Updated(Box::new(snapshot))
    }
}

pub fn resolve_telemetry_path() -> Result<PathBuf, TelemetryPathError> {
    resolve_telemetry_path_from(
        env::args_os(),
        env::var_os(TELEMETRY_FILE_ENVIRONMENT),
        env::var_os("LOCALAPPDATA"),
    )
}

pub fn resolve_telemetry_path_from<I>(
    arguments: I,
    environment_override: Option<OsString>,
    local_app_data: Option<OsString>,
) -> Result<PathBuf, TelemetryPathError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut arguments = arguments.into_iter();
    let mut cli_override = None;

    while let Some(argument) = arguments.next() {
        if argument == OsStr::new(TELEMETRY_FILE_ARGUMENT) {
            if cli_override.is_some() {
                return Err(TelemetryPathError::DuplicateArgument);
            }
            cli_override = Some(
                arguments
                    .next()
                    .ok_or(TelemetryPathError::MissingArgumentValue)?,
            );
            continue;
        }

        let argument_text = argument.to_string_lossy();
        if let Some(value) = argument_text.strip_prefix("--telemetry-file=") {
            if cli_override.is_some() {
                return Err(TelemetryPathError::DuplicateArgument);
            }
            cli_override = Some(OsString::from(value));
        }
    }

    if let Some(path) = cli_override.or(environment_override) {
        return non_empty_path(path);
    }

    let local_app_data =
        non_empty_path(local_app_data.ok_or(TelemetryPathError::MissingLocalAppData)?)?;
    Ok(local_app_data
        .join("ScrapMap")
        .join("diagnostic")
        .join("telemetry.json"))
}

fn non_empty_path(path: OsString) -> Result<PathBuf, TelemetryPathError> {
    if path.is_empty() {
        Err(TelemetryPathError::EmptyOverride)
    } else {
        Ok(PathBuf::from(path))
    }
}

fn read_stable_file(path: &Path) -> Result<Vec<u8>, DiagnosticIssue> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(DiagnosticIssue::new(
                DiagnosticIssueKind::Missing,
                "waiting for diagnostic telemetry",
            ));
        }
        Err(error) => {
            return Err(DiagnosticIssue::new(
                DiagnosticIssueKind::Io,
                format!(
                    "telemetry file is temporarily unreadable ({})",
                    error.kind()
                ),
            ));
        }
    };

    let before = file.metadata().map_err(|error| {
        DiagnosticIssue::new(
            DiagnosticIssueKind::Io,
            format!("telemetry metadata is unavailable ({})", error.kind()),
        )
    })?;
    if before.len() > MAX_FILE_SIZE {
        return Err(DiagnosticIssue::new(
            DiagnosticIssueKind::TooLarge,
            "telemetry file exceeds the 1 MiB safety limit",
        ));
    }

    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.by_ref()
        .take(MAX_FILE_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            DiagnosticIssue::new(
                DiagnosticIssueKind::Io,
                format!("telemetry read failed ({})", error.kind()),
            )
        })?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(DiagnosticIssue::new(
            DiagnosticIssueKind::TooLarge,
            "telemetry file exceeds the 1 MiB safety limit",
        ));
    }

    let after = file.metadata().map_err(|error| {
        DiagnosticIssue::new(
            DiagnosticIssueKind::Io,
            format!("telemetry metadata is unavailable ({})", error.kind()),
        )
    })?;
    let before_modified = before.modified().ok();
    let after_modified = after.modified().ok();
    if before.len() != after.len()
        || before_modified != after_modified
        || bytes.len() as u64 != after.len()
    {
        return Err(DiagnosticIssue::new(
            DiagnosticIssueKind::Busy,
            "telemetry is being updated; retrying",
        ));
    }

    Ok(bytes)
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn validate_snapshot(value: &Value) -> Result<TelemetrySnapshot, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "telemetry must be a JSON object".to_owned())?;

    let schema_version = optional_integer(object, "schemaVersion", 32)?
        .unwrap_or(1)
        .try_into()
        .map_err(|_| "schemaVersion is outside the supported range".to_owned())?;
    if !(1..=32).contains(&schema_version) {
        return Err("schemaVersion is outside the supported range".to_owned());
    }

    let world_id = optional_bounded_text(object, "worldId", MAX_WORLD_ID_CHARS)?;
    let payload_world_id = optional_id(object.get("payloadWorldId"), "payloadWorldId")?;
    let timestamp = optional_bounded_text(object, "timestamp", 80)?;
    let server_tick = optional_integer(object, "serverTick", MAX_SAFE_INTEGER as u64)?;
    let sequence = optional_integer(object, "sequence", MAX_SAFE_INTEGER as u64)?;
    let local_player_id = optional_id(object.get("localPlayerId"), "localPlayerId")?;
    let stale_after_ms = optional_integer(object, "staleAfterMs", 60_000)?
        .unwrap_or(2_000)
        .clamp(500, 30_000);
    let source = parse_source(object.get("source"))?;

    let raw_players = match object.get("players") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(players)) => players.as_slice(),
        Some(_) => return Err("players must be an array".to_owned()),
    };
    if raw_players.len() > MAX_PLAYERS {
        return Err(format!("players exceeds the safety limit of {MAX_PLAYERS}"));
    }

    let mut players = raw_players
        .iter()
        .enumerate()
        .map(|(index, player)| parse_player(player, index, payload_world_id.as_deref(), false))
        .collect::<Result<Vec<_>, _>>()?;
    let mut primary = match object.get("player") {
        None | Some(Value::Null) => None,
        Some(player) => Some(parse_player(player, 0, payload_world_id.as_deref(), true)?),
    };

    if let Some(player) = primary.as_mut() {
        if player.id.is_none() {
            player.id.clone_from(&local_player_id);
        }
    }

    let explicit_local = players.iter().position(|player| player.is_local);
    let id_local = local_player_id.as_ref().and_then(|id| {
        players
            .iter()
            .position(|player| player.id.as_ref() == Some(id))
    });
    let primary_match = primary.as_ref().and_then(|player| {
        player.id.as_ref().and_then(|id| {
            players
                .iter()
                .position(|candidate| candidate.id.as_ref() == Some(id))
        })
    });

    let local_index = if let Some(primary) = primary {
        if !primary.has_position() {
            return Err("player must contain finite x, y and z coordinates".to_owned());
        }
        let index = primary_match.or(id_local).or(explicit_local);
        match index {
            Some(index) => {
                players[index] = primary;
                index
            }
            None if players.len() < MAX_PLAYERS => {
                players.insert(0, primary);
                0
            }
            None => return Err("no room for the local player in players".to_owned()),
        }
    } else {
        id_local
            .or(explicit_local)
            .ok_or_else(|| "telemetry has no identifiable local player".to_owned())?
    };

    players
        .iter_mut()
        .enumerate()
        .for_each(|(index, player)| player.is_local = index == local_index);
    if !players[local_index].is_renderable_local_player() {
        return Err(
            "the local player must be active, in-world and contain finite x, y and z coordinates"
                .to_owned(),
        );
    }
    let player = players[local_index].clone();

    Ok(TelemetrySnapshot {
        schema_version,
        world_id,
        payload_world_id,
        timestamp,
        server_tick,
        sequence,
        local_player_id: player.id.clone().or(local_player_id),
        stale_after_ms,
        source,
        player,
        players,
    })
}

fn parse_source(value: Option<&Value>) -> Result<Option<TelemetrySource>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let object = value
        .as_object()
        .ok_or_else(|| "source must be an object".to_owned())?;
    Ok(Some(TelemetrySource {
        source_type: optional_bounded_text(object, "type", 96)?
            .unwrap_or_else(|| "diagnostic-json".to_owned()),
        enumeration: optional_bounded_text(object, "enumeration", 96)?
            .unwrap_or_else(|| "unknown".to_owned()),
        is_host: optional_bool(object, "isHost")?.unwrap_or(false),
        compatibility: optional_bounded_text(object, "compatibility", 32)?,
    }))
}

fn parse_player(
    value: &Value,
    index: usize,
    top_level_world_id: Option<&str>,
    primary: bool,
) -> Result<TelemetryPlayer, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("players[{index}] must be an object"))?;
    let id = optional_id(object.get("id"), "player.id")?;
    let default_name = if primary {
        "Player".to_owned()
    } else if let Some(id) = &id {
        format!("Player {id}")
    } else {
        format!("Player {}", index + 1)
    };
    let name = sanitized_player_name(object.get("name"), &default_name)?;

    let x = optional_number(object, "x", MAX_COORDINATE_ABS)?;
    let y = optional_number(object, "y", MAX_COORDINATE_ABS)?;
    let z = optional_number(object, "z", MAX_COORDINATE_ABS)?;
    let coordinate_count = [x, y, z].iter().filter(|value| value.is_some()).count();
    if coordinate_count != 0 && coordinate_count != 3 {
        return Err(format!(
            "players[{index}] must contain all of x, y and z or none of them"
        ));
    }
    let has_position = coordinate_count == 3;
    let has_character = optional_bool(object, "hasCharacter")?.unwrap_or(has_position);
    if has_character && !has_position {
        return Err(format!(
            "players[{index}] hasCharacter requires finite x, y and z"
        ));
    }

    let payload_world_id = optional_id(object.get("payloadWorldId"), "player.payloadWorldId")?
        .or_else(|| top_level_world_id.map(str::to_owned));
    let world_id = optional_bounded_text(object, "worldId", MAX_WORLD_ID_CHARS)?;
    let same_world = optional_bool(object, "sameWorld")?.unwrap_or_else(|| {
        payload_world_id
            .as_deref()
            .zip(top_level_world_id)
            .is_none_or(|(player_world, local_world)| player_world == local_world)
    });
    let raw_heading = optional_number(object, "heading", MAX_VECTOR_ABS)?.or(optional_number(
        object,
        "yaw",
        MAX_VECTOR_ABS,
    )?);
    let heading = raw_heading.map(|heading| heading.rem_euclid(360.0));

    Ok(TelemetryPlayer {
        id,
        name,
        is_local: optional_bool(object, "isLocal")?
            .or(optional_bool(object, "local")?)
            .unwrap_or(primary),
        active: optional_bool(object, "active")?.unwrap_or(true),
        has_character,
        same_world,
        character_id: optional_id(object.get("characterId"), "player.characterId")?,
        payload_world_id,
        world_id,
        x,
        y,
        z,
        heading,
        dx: optional_number(object, "dx", MAX_VECTOR_ABS)?,
        dy: optional_number(object, "dy", MAX_VECTOR_ABS)?,
        dz: optional_number(object, "dz", MAX_VECTOR_ABS)?,
        vx: optional_number(object, "vx", MAX_VECTOR_ABS)?,
        vy: optional_number(object, "vy", MAX_VECTOR_ABS)?,
        vz: optional_number(object, "vz", MAX_VECTOR_ABS)?,
    })
}

fn sanitized_player_name(value: Option<&Value>, fallback: &str) -> Result<String, String> {
    let Some(value) = value else {
        return Ok(fallback.to_owned());
    };
    if value.is_null() {
        return Ok(fallback.to_owned());
    }
    let raw = value
        .as_str()
        .ok_or_else(|| "player.name must be a string".to_owned())?;
    let name: String = raw
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    Ok(if name.is_empty() {
        fallback.to_owned()
    } else {
        name
    })
}

fn optional_bool(object: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("{key} must be a boolean")),
    }
}

fn optional_number(
    object: &Map<String, Value>,
    key: &str,
    maximum_absolute: f64,
) -> Result<Option<f64>, String> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let number = value
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or_else(|| format!("{key} must be a finite number"))?;
    if number.abs() > maximum_absolute {
        return Err(format!("{key} exceeds the safety limit"));
    }
    Ok(Some(number))
}

fn optional_integer(
    object: &Map<String, Value>,
    key: &str,
    maximum: u64,
) -> Result<Option<u64>, String> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let number = value
        .as_f64()
        .filter(|number| number.is_finite() && *number >= 0.0 && number.fract() == 0.0)
        .ok_or_else(|| format!("{key} must be a non-negative integer"))?;
    if number > maximum as f64 {
        return Err(format!("{key} exceeds the safety limit"));
    }
    Ok(Some(number as u64))
}

fn optional_id(value: Option<&Value>, field: &str) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::String(value) => {
            let value = value.trim();
            if value.is_empty()
                || value.chars().count() > MAX_ID_CHARS
                || value.chars().any(char::is_control)
            {
                return Err(format!("{field} is invalid"));
            }
            Ok(Some(value.to_owned()))
        }
        Value::Number(value) => {
            let value = value
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0 && *value <= MAX_SAFE_INTEGER)
                .ok_or_else(|| format!("{field} is outside the supported range"))?;
            Ok(Some(if value.fract() == 0.0 {
                format!("{value:.0}")
            } else {
                value.to_string()
            }))
        }
        _ => Err(format!("{field} must be a string or number")),
    }
}

fn optional_bounded_text(
    object: &Map<String, Value>,
    key: &str,
    maximum_chars: usize,
) -> Result<Option<String>, String> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let text = value
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))?
        .trim();
    if text.is_empty() || text.chars().count() > maximum_chars || text.chars().any(char::is_control)
    {
        return Err(format!("{key} is invalid"));
    }
    Ok(Some(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct FixtureDirectory(PathBuf);

    impl FixtureDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock is before the Unix epoch")
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "scrapmap-diagnostic-source-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("failed to create fixture directory");
            Self(path)
        }

        fn telemetry_path(&self) -> PathBuf {
            self.0.join("telemetry.json")
        }
    }

    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn valid_payload(sequence: u64) -> String {
        format!(
            r#"{{
              "schemaVersion": 2,
              "worldId": "survival-fixture-w1",
              "payloadWorldId": 1,
              "sequence": {sequence},
              "localPlayerId": 7,
              "staleAfterMs": 2000,
              "source": {{
                "type": "diagnostic-fixture",
                "enumeration": "getAllPlayers",
                "isHost": false
              }},
              "player": {{
                "id": 7,
                "name": " Local\u0000 Player ",
                "x": -1889.4,
                "y": -1684.2,
                "z": 7.7,
                "heading": -10
              }},
              "players": [
                {{
                  "id": 7,
                  "name": "Local Player",
                  "isLocal": true,
                  "hasCharacter": true,
                  "payloadWorldId": 1,
                  "x": -1889.4,
                  "y": -1684.2,
                  "z": 7.7,
                  "heading": 77.3
                }},
                {{
                  "id": 3,
                  "name": "Friend",
                  "isLocal": false,
                  "hasCharacter": false,
                  "payloadWorldId": 1
                }}
              ]
            }}"#
        )
    }

    #[test]
    fn path_resolution_prefers_cli_then_environment_then_default() {
        let cli = resolve_telemetry_path_from(
            [
                OsString::from("scrapmap.exe"),
                OsString::from("--telemetry-file"),
                OsString::from("D:\\fixture\\cli.json"),
            ],
            Some(OsString::from("D:\\fixture\\env.json")),
            Some(OsString::from("D:\\LocalAppData")),
        )
        .expect("CLI path should resolve");
        assert_eq!(cli, PathBuf::from("D:\\fixture\\cli.json"));

        let environment = resolve_telemetry_path_from(
            [OsString::from("scrapmap.exe")],
            Some(OsString::from("D:\\fixture\\env.json")),
            Some(OsString::from("D:\\LocalAppData")),
        )
        .expect("environment path should resolve");
        assert_eq!(environment, PathBuf::from("D:\\fixture\\env.json"));

        let default = resolve_telemetry_path_from(
            [OsString::from("scrapmap.exe")],
            None,
            Some(OsString::from("D:\\LocalAppData")),
        )
        .expect("default path should resolve");
        assert_eq!(
            default,
            PathBuf::from("D:\\LocalAppData")
                .join("ScrapMap")
                .join("diagnostic")
                .join("telemetry.json")
        );
    }

    #[test]
    fn polling_normalizes_and_deduplicates_valid_updates() {
        let fixture = FixtureDirectory::new();
        let path = fixture.telemetry_path();
        fs::write(&path, valid_payload(12)).expect("failed to write fixture");

        let mut source = DiagnosticSource::new(&path);
        let first_poll = source.poll();
        let DiagnosticPoll::Updated(snapshot) = first_poll else {
            panic!("first valid payload should produce an update: {first_poll:?}");
        };
        assert_eq!(snapshot.sequence, Some(12));
        assert_eq!(snapshot.player.id.as_deref(), Some("7"));
        assert_eq!(snapshot.player.name, "Local Player");
        assert_eq!(snapshot.player.heading, Some(350.0));
        assert_eq!(snapshot.players.len(), 2);
        assert!(snapshot.players[0].is_local);
        assert!(!snapshot.players[1].has_character);
        assert_eq!(source.poll(), DiagnosticPoll::Unchanged);
    }

    #[test]
    fn unsupported_native_provider_is_a_safe_noop_until_a_supported_frame_arrives() {
        let fixture = FixtureDirectory::new();
        let path = fixture.telemetry_path();
        let unsupported = valid_payload(1).replace(
            r#""isHost": false"#,
            r#""isHost": false, "compatibility": "unsupported""#,
        );
        fs::write(&path, unsupported).expect("failed to write unsupported fixture");

        let mut source = DiagnosticSource::new(&path);
        let DiagnosticPoll::Waiting(issue) = source.poll() else {
            panic!("unsupported provider must not publish telemetry");
        };
        assert_eq!(issue.kind, DiagnosticIssueKind::Unsupported);
        assert!(source.last_snapshot().is_none());

        fs::write(&path, valid_payload(2)).expect("failed to write supported fixture");
        assert!(matches!(source.poll(), DiagnosticPoll::Updated(_)));
    }

    #[test]
    fn partial_write_keeps_last_known_snapshot_and_recovers() {
        let fixture = FixtureDirectory::new();
        let path = fixture.telemetry_path();
        fs::write(&path, valid_payload(1)).expect("failed to write fixture");
        let mut source = DiagnosticSource::new(&path);
        let first_poll = source.poll();
        assert!(
            matches!(first_poll, DiagnosticPoll::Updated(_)),
            "first valid payload should produce an update: {first_poll:?}"
        );

        fs::write(&path, br#"{"schemaVersion":2,"player":"#)
            .expect("failed to write partial fixture");
        let DiagnosticPoll::Waiting(issue) = source.poll() else {
            panic!("partial JSON should wait for a later poll");
        };
        assert_eq!(issue.kind, DiagnosticIssueKind::InvalidJson);
        assert_eq!(
            source
                .last_snapshot()
                .and_then(|snapshot| snapshot.sequence),
            Some(1)
        );

        fs::write(&path, valid_payload(2)).expect("failed to repair fixture");
        let DiagnosticPoll::Updated(snapshot) = source.poll() else {
            panic!("repaired JSON should produce an update");
        };
        assert_eq!(snapshot.sequence, Some(2));
    }

    #[test]
    fn local_player_without_a_complete_position_is_not_published() {
        let fixture = FixtureDirectory::new();
        let path = fixture.telemetry_path();
        fs::write(
            &path,
            r#"{
              "schemaVersion": 2,
              "localPlayerId": 7,
              "players": [{
                "id": 7,
                "name": "Local Player",
                "isLocal": true,
                "hasCharacter": false
              }]
            }"#,
        )
        .expect("failed to write fixture");

        let mut source = DiagnosticSource::new(path);
        let DiagnosticPoll::Waiting(issue) = source.poll() else {
            panic!("a local player without coordinates should not be published");
        };
        assert_eq!(issue.kind, DiagnosticIssueKind::InvalidPayload);
        assert!(source.last_snapshot().is_none());
    }

    #[test]
    fn inactive_local_player_keeps_the_previous_renderable_snapshot() {
        let fixture = FixtureDirectory::new();
        let path = fixture.telemetry_path();
        fs::write(&path, valid_payload(1)).expect("failed to write fixture");
        let mut source = DiagnosticSource::new(&path);
        assert!(matches!(source.poll(), DiagnosticPoll::Updated(_)));

        fs::write(
            &path,
            r#"{
              "schemaVersion": 2,
              "player": {
                "id": 7,
                "name": "Local Player",
                "active": false,
                "hasCharacter": true,
                "x": 1,
                "y": 2,
                "z": 3
              }
            }"#,
        )
        .expect("failed to write inactive fixture");

        let DiagnosticPoll::Waiting(issue) = source.poll() else {
            panic!("an inactive local player should not replace the last snapshot");
        };
        assert_eq!(issue.kind, DiagnosticIssueKind::InvalidPayload);
        assert_eq!(
            source
                .last_snapshot()
                .and_then(|snapshot| snapshot.sequence),
            Some(1)
        );
    }

    #[test]
    fn unreasonable_coordinates_are_rejected() {
        let fixture = FixtureDirectory::new();
        let path = fixture.telemetry_path();
        fs::write(
            &path,
            r#"{
              "schemaVersion": 2,
              "player": {
                "id": 1,
                "name": "Local",
                "x": 10000001,
                "y": 0,
                "z": 0
              }
            }"#,
        )
        .expect("failed to write fixture");

        let mut source = DiagnosticSource::new(path);
        let DiagnosticPoll::Waiting(issue) = source.poll() else {
            panic!("an unreasonable coordinate should not be published");
        };
        assert_eq!(issue.kind, DiagnosticIssueKind::InvalidPayload);
    }
}
