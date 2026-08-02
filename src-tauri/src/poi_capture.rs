//! Photographs points of interest by driving the game's own camera.
//!
//! The procedural atlas samples terrain, so it cannot show the crashed ship, a
//! warehouse or a ruin: those are placed objects and buildings. This walks the
//! in-game camera over each POI, captures the window, and stores the result as
//! a tile image that takes precedence over the generated one.
//!
//! The handshake is one-way by necessity. `sm.json.fileExists` cannot see files
//! written during the same session, so Lua can neither be signalled mid-run nor
//! poll for an acknowledgement. Instead Lua holds each pose for a fixed dwell
//! and announces it in the log; this side captures during that window.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::{json, Value};

use crate::window_capture::{capture_window, Frame};

/// Where the sweep request is left for Lua to find on the next world load.
const REQUEST_FILE: &str = "ScrapMapCapture.json";
const PHOTO_DIRECTORY: &str = "photo";
/// Stored edge length. Generous enough to stay sharp when a tile fills the
/// screen, small enough that a hundred of them stay a reasonable cache.
const PHOTO_EDGE: u32 = 512;
/// Below this the frame is almost certainly a loading screen or a black
/// capture rather than terrain, and storing it would look like a hole.
const MIN_LIT_FRACTION: f32 = 0.35;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTarget {
    pub uuid: String,
    pub x: i64,
    pub y: i64,
    pub size: u32,
    /// Ground height to add to the camera's altitude, so a clifftop POI is
    /// framed from the same distance as one at sea level.
    pub ground_height: f64,
}

/// Chooses one representative placement per POI tile.
///
/// A tile appears many times over -- 405 placements across 116 distinct tiles
/// in the test world -- but the photograph keys on the tile, so only one
/// instance of each is worth visiting. Filler is skipped: it restates terrain
/// the generated atlas already draws.
pub fn build_targets(layout: &Value) -> Vec<CaptureTarget> {
    let Some(cells) = layout.get("cells").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut chosen: BTreeMap<String, CaptureTarget> = BTreeMap::new();
    for cell in cells {
        let Some(poi) = cell.get("poi").filter(|value| !value.is_null()) else {
            continue;
        };
        let code = poi
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_uppercase();
        if code.contains("RANDOM") {
            continue;
        }
        // Multi-cell tiles report their origin only in the zero-offset cell;
        // aiming from any other corner would frame the neighbour.
        let offset_x = cell.get("xOffset").and_then(Value::as_f64).unwrap_or(0.0);
        let offset_y = cell.get("yOffset").and_then(Value::as_f64).unwrap_or(0.0);
        if offset_x != 0.0 || offset_y != 0.0 {
            continue;
        }
        let Some(uuid) = cell.get("uuid").and_then(Value::as_str) else {
            continue;
        };
        if chosen.contains_key(uuid) {
            continue;
        }
        let (Some(x), Some(y)) = (
            cell.get("x").and_then(Value::as_f64),
            cell.get("y").and_then(Value::as_f64),
        ) else {
            continue;
        };
        chosen.insert(
            uuid.to_owned(),
            CaptureTarget {
                uuid: uuid.to_owned(),
                x: x as i64,
                y: y as i64,
                size: cell
                    .get("tileSize")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0)
                    .max(1.0) as u32,
                ground_height: 0.0,
            },
        );
    }

    chosen.into_values().collect()
}

/// Leaves the sweep request where Lua will find it on the next world load.
pub fn write_request(game_root: &Path, targets: &[CaptureTarget]) -> Result<PathBuf, String> {
    let path = game_root.join("Survival").join(REQUEST_FILE);
    let document = json!({
        "schemaVersion": 1,
        "targets": targets,
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("could not write the capture request: {error}"))?;
    Ok(path)
}

/// Removes the request so the sweep runs once rather than on every world load.
pub fn clear_request(game_root: &Path) {
    let _ = fs::remove_file(game_root.join("Survival").join(REQUEST_FILE));
}

/// Parses `SCRAPMAP_SHOT_V1|ready|<uuid>|<x>|<y>|<size>|<metres>`.
pub fn parse_ready(line: &str) -> Option<(String, u32)> {
    let payload = line.split_once("SCRAPMAP_SHOT_V1|ready|")?.1.trim();
    let mut fields = payload.split('|');
    let uuid = fields.next()?.trim().to_owned();
    let _x = fields.next()?;
    let _y = fields.next()?;
    let size: u32 = fields.next()?.trim().parse().ok()?;
    (!uuid.is_empty() && size > 0).then_some((uuid, size))
}

pub fn is_sweep_finished(line: &str) -> bool {
    line.contains("SCRAPMAP_SHOT_V1|done|")
}

/// Follows the game log for sweep cues.
///
/// Deliberately separate from the telemetry tail: that one consumes lines as it
/// goes, and a sweep must not depend on which reader happened to see a line
/// first.
#[derive(Default)]
pub struct ShotWatcher {
    path: Option<PathBuf>,
    offset: u64,
}

#[derive(Debug, PartialEq)]
pub enum ShotEvent {
    Ready { uuid: String, size: u32 },
    Finished,
}

impl ShotWatcher {
    /// Reads whatever has been appended since the last call.
    pub fn poll(&mut self, log_directory: &Path) -> Vec<ShotEvent> {
        use std::io::{BufRead, BufReader, Seek, SeekFrom};

        let Some(latest) = newest_log(log_directory) else {
            return Vec::new();
        };
        if self.path.as_ref() != Some(&latest) {
            self.path = Some(latest.clone());
            self.offset = 0;
        }
        let Ok(mut file) = fs::File::open(&latest) else {
            return Vec::new();
        };
        // A restarted game writes a fresh, shorter log under the same name.
        if file.metadata().map(|data| data.len()).unwrap_or(0) < self.offset {
            self.offset = 0;
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }

        let mut reader = BufReader::new(file);
        let mut events = Vec::new();
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if let Some((uuid, size)) = parse_ready(&line) {
                events.push(ShotEvent::Ready { uuid, size });
            } else if is_sweep_finished(&line) {
                events.push(ShotEvent::Finished);
            }
            line.clear();
        }
        self.offset = reader.stream_position().unwrap_or(self.offset);
        events
    }
}

fn newest_log(directory: &Path) -> Option<PathBuf> {
    fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let is_log = path
                .extension()
                .is_some_and(|value| value.eq_ignore_ascii_case("log"));
            is_log.then(|| {
                entry
                    .metadata()
                    .ok()
                    .and_then(|data| data.modified().ok())
                    .map(|time| (time, path))
            })?
        })
        .max_by_key(|(time, _)| *time)
        .map(|(_, path)| path)
}

/// Captures the framed tile and stores it beside the generated atlas.
///
/// The camera frames the tile to the full height of the client area, so the
/// centred square of that height is exactly the tile.
pub fn capture_tile(
    window_handle: isize,
    uuid: &str,
    atlas_root: &Path,
) -> Result<PathBuf, String> {
    let frame = capture_window(window_handle)?;
    let square = frame
        .centre_square(frame.height)
        .ok_or("the captured frame has no usable square")?;
    let lit = square.lit_fraction();
    if lit < MIN_LIT_FRACTION {
        return Err(format!(
            "frame is {:.0}% lit, which reads as a loading screen rather than terrain",
            lit * 100.0
        ));
    }
    let scaled = square
        .resize(PHOTO_EDGE)
        .ok_or("could not rescale the capture")?;

    let directory = atlas_root.join("tiles").join(PHOTO_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join(format!("{}.png", uuid.to_ascii_lowercase()));
    write_png(&path, &scaled)?;
    Ok(path)
}

fn write_png(path: &Path, frame: &Frame) -> Result<(), String> {
    let file = fs::File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(file, frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|error| error.to_string())?
        .write_image_data(&frame.pixels)
        .map_err(|error| error.to_string())
}

/// Points the named tiles at their photographs, so a real capture wins over the
/// generated terrain for that tile.
pub fn publish_photos(atlas_root: &Path, uuids: &[String]) -> Result<(), String> {
    if uuids.is_empty() {
        return Ok(());
    }
    let manifest_path = atlas_root.join("manifest.json");
    let bytes = fs::read(&manifest_path).map_err(|error| error.to_string())?;
    let mut manifest: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("manifest parse: {error}"))?;
    let entries = manifest
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .ok_or("manifest has no entries array")?;

    for uuid in uuids {
        let key = uuid.to_ascii_lowercase();
        let relative = format!("{PHOTO_DIRECTORY}/{key}.png");
        match entries
            .iter_mut()
            .find(|entry| entry.get("tileUuid").and_then(Value::as_str) == Some(key.as_str()))
        {
            Some(entry) => {
                entry["topDownRelativePath"] = json!(relative);
                entry["topDownSourceKind"] = json!("photo");
            }
            None => entries.push(json!({
                "tileUuid": key,
                "relativePath": relative,
                "topDownRelativePath": relative,
                "topDownSourceKind": "photo",
            })),
        }
    }

    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(uuid: &str, x: f64, y: f64, size: f64, code: Option<&str>, offset: (f64, f64)) -> Value {
        json!({
            "uuid": uuid,
            "x": x,
            "y": y,
            "tileSize": size,
            "xOffset": offset.0,
            "yOffset": offset.1,
            "poi": code.map(|value| json!({ "code": value })),
        })
    }

    #[test]
    fn one_target_per_tile_rather_than_per_placement() {
        // The same warehouse tile placed twice should be visited once.
        let layout = json!({ "cells": [
            cell("aa", 1.0, 1.0, 4.0, Some("POI_WAREHOUSE2_LARGE"), (0.0, 0.0)),
            cell("aa", 40.0, 9.0, 4.0, Some("POI_WAREHOUSE2_LARGE"), (0.0, 0.0)),
            cell("bb", 3.0, 4.0, 1.0, Some("POI_CAMP"), (0.0, 0.0)),
        ]});
        let targets = build_targets(&layout);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].uuid, "aa");
        assert_eq!(targets[0].size, 4);
        assert_eq!(targets[1].uuid, "bb");
    }

    #[test]
    fn generator_filler_and_plain_terrain_are_not_photographed() {
        let layout = json!({ "cells": [
            cell("lake", 0.0, 0.0, 1.0, Some("POI_LAKE_RANDOM"), (0.0, 0.0)),
            cell("road", 1.0, 0.0, 1.0, Some("POI_ROAD_RANDOM"), (0.0, 0.0)),
            cell("grass", 2.0, 0.0, 1.0, None, (0.0, 0.0)),
        ]});
        assert!(build_targets(&layout).is_empty());
    }

    #[test]
    fn multi_cell_tiles_are_aimed_from_their_origin_cell() {
        // Only the zero-offset cell carries the tile's true position.
        let layout = json!({ "cells": [
            cell("ship", 9.0, 9.0, 4.0, Some("POI_CRASHSITE_AREA"), (1.0, 2.0)),
            cell("ship", 5.0, 4.0, 4.0, Some("POI_CRASHSITE_AREA"), (0.0, 0.0)),
        ]});
        let targets = build_targets(&layout);
        assert_eq!(targets.len(), 1);
        assert_eq!((targets[0].x, targets[0].y), (5, 4));
    }

    #[test]
    fn ready_lines_parse_out_of_the_game_log() {
        let line = "12:00:01 [Lua] SCRAPMAP_SHOT_V1|ready|abc-123|-38|-42|4|256.00";
        assert_eq!(parse_ready(line), Some(("abc-123".to_owned(), 4)));
        assert_eq!(parse_ready("unrelated log line"), None);
        // A truncated line must not be taken as a cue to capture.
        assert_eq!(parse_ready("SCRAPMAP_SHOT_V1|ready|abc-123|-38"), None);
        assert!(is_sweep_finished("[Lua] SCRAPMAP_SHOT_V1|done|116"));
        assert!(!is_sweep_finished("[Lua] SCRAPMAP_SHOT_V1|ready|a|1|2|1|64"));
    }

    #[test]
    fn a_request_round_trips_to_disk() {
        let root = std::env::temp_dir().join(format!(
            "scrapmap-capture-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default()
        ));
        fs::create_dir_all(root.join("Survival")).unwrap();
        let targets = vec![CaptureTarget {
            uuid: "abc".to_owned(),
            x: -5,
            y: 7,
            size: 2,
            ground_height: 0.0,
        }];
        let path = write_request(&root, &targets).unwrap();
        let written: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(written["targets"][0]["uuid"], "abc");
        assert_eq!(written["targets"][0]["size"], 2);
        assert_eq!(written["targets"][0]["x"], -5);

        clear_request(&root);
        assert!(!path.exists(), "the sweep must not repeat on the next load");
        fs::remove_dir_all(&root).ok();
    }
}
