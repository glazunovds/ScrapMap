use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::diagnostic_source::{TelemetryPlayer, TelemetrySnapshot, TelemetrySource};

const PREFIX: &str = "SCRAPMAP_TELEMETRY_V1|";
const PLAYER_STALE_AFTER: Duration = Duration::from_millis(1500);

struct SeenPlayer {
    player: TelemetryPlayer,
    seen_at: Instant,
}

#[derive(Default)]
pub(crate) struct GameLogSource {
    root: Option<PathBuf>,
    path: Option<PathBuf>,
    offset: u64,
    players: BTreeMap<String, SeenPlayer>,
    sequence: u64,
}

impl GameLogSource {
    pub(crate) fn set_root(&mut self, root: Option<PathBuf>) {
        if self.root != root {
            self.root = root;
            self.path = None;
            self.offset = 0;
            self.players.clear();
        }
    }

    pub(crate) fn poll(&mut self) -> Option<TelemetrySnapshot> {
        let latest = latest_log(self.root.as_deref()?)?;
        if self.path.as_ref() != Some(&latest) {
            self.path = Some(latest.clone());
            self.offset = 0;
            self.players.clear();
        }
        let mut file = File::open(latest).ok()?;
        let length = file.metadata().ok()?.len();
        if length < self.offset {
            self.offset = 0;
            self.players.clear();
        }
        file.seek(SeekFrom::Start(self.offset)).ok()?;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut updated = false;
        while reader.read_line(&mut line).ok()? > 0 {
            if let Some(player) = parse_line(&line) {
                self.players.insert(
                    player.id.clone().unwrap_or_default(),
                    SeenPlayer {
                        player,
                        seen_at: Instant::now(),
                    },
                );
                updated = true;
            }
            line.clear();
        }
        self.offset = reader.stream_position().ok()?;
        if !updated {
            return None;
        }
        self.snapshot(Instant::now())
    }

    fn snapshot(&mut self, now: Instant) -> Option<TelemetrySnapshot> {
        let local = self
            .players
            .values()
            .filter(|seen| now.saturating_duration_since(seen.seen_at) <= PLAYER_STALE_AFTER)
            .find(|seen| seen.player.is_local)?
            .player
            .clone();
        let local_world = local.world_id.clone();
        self.players.retain(|_, seen| {
            seen.player.world_id == local_world
                && now.saturating_duration_since(seen.seen_at) <= PLAYER_STALE_AFTER
        });
        let players = self
            .players
            .values()
            .map(|seen| seen.player.clone())
            .collect::<Vec<_>>();
        self.sequence = self.sequence.saturating_add(1);
        Some(TelemetrySnapshot {
            schema_version: 1,
            world_id: local_world.clone(),
            payload_world_id: local_world,
            timestamp: None,
            server_tick: None,
            sequence: Some(self.sequence),
            local_player_id: local.id.clone(),
            stale_after_ms: 1500,
            source: Some(TelemetrySource {
                source_type: "scrapmap-game-log".to_owned(),
                enumeration: "client-visible-players".to_owned(),
                is_host: false,
                compatibility: Some("survival-lua-v1".to_owned()),
            }),
            player: local,
            players,
        })
    }
}

fn latest_log(root: &Path) -> Option<PathBuf> {
    fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let is_log = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("log"));
            is_log
                .then(|| {
                    entry
                        .metadata()
                        .ok()
                        .and_then(|metadata| metadata.modified().ok())
                        .map(|time| (time, path))
                })
                .flatten()
        })
        .max_by_key(|(time, _)| *time)
        .map(|(_, path)| path)
}

fn parse_line(line: &str) -> Option<TelemetryPlayer> {
    let payload = line.split_once(PREFIX)?.1.trim();
    let fields = payload.split('|').collect::<Vec<_>>();
    if fields.len() != 10 {
        return None;
    }
    let id = fields[0].to_owned();
    let is_local = fields[1] == "1";
    let name = fields[2].chars().take(80).collect();
    let world_id = format!("world-{}", fields[3]);
    let values = fields[4..]
        .iter()
        .map(|value| value.parse::<f64>().ok())
        .collect::<Option<Vec<_>>>()?;
    if values
        .iter()
        .any(|value| !value.is_finite() || value.abs() > 10_000_000.0)
    {
        return None;
    }
    let heading = values[3].atan2(values[4]).to_degrees();
    Some(TelemetryPlayer {
        id: Some(id.clone()),
        name,
        is_local,
        active: true,
        has_character: true,
        same_world: true,
        character_id: Some(id),
        payload_world_id: Some(world_id.clone()),
        world_id: Some(world_id),
        x: Some(values[0]),
        y: Some(values[1]),
        z: Some(values[2]),
        heading: Some(heading),
        dx: Some(values[3]),
        dy: Some(values[4]),
        dz: Some(values[5]),
        vx: None,
        vy: None,
        vz: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_prefixed_player_line() {
        let player = parse_line(
            "12:00 [Lua] SCRAPMAP_TELEMETRY_V1|2|1|Alice|1|-12.5|8.0|3.25|0.0|1.0|0.0\n",
        )
        .unwrap();
        assert_eq!(player.name, "Alice");
        assert_eq!(player.x, Some(-12.5));
        assert!(player.is_local);
    }

    #[test]
    fn stale_remote_player_is_evicted_from_snapshot() {
        let now = Instant::now();
        let local = parse_line("[Lua] SCRAPMAP_TELEMETRY_V1|1|1|Local|7|1|2|3|0|1|0").unwrap();
        let remote = parse_line("[Lua] SCRAPMAP_TELEMETRY_V1|2|0|Friend|7|4|5|6|0|1|0").unwrap();
        let mut source = GameLogSource::default();
        source.players.insert(
            "1".to_owned(),
            SeenPlayer {
                player: local,
                seen_at: now,
            },
        );
        source.players.insert(
            "2".to_owned(),
            SeenPlayer {
                player: remote,
                seen_at: now - PLAYER_STALE_AFTER - Duration::from_millis(1),
            },
        );

        let snapshot = source.snapshot(now).unwrap();
        assert_eq!(snapshot.players.len(), 1);
        assert_eq!(snapshot.players[0].name, "Local");
        assert!(!source.players.contains_key("2"));
    }
}
