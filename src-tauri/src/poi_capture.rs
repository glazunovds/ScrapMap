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

use crate::{
    atlas_bake,
    window_capture::{capture_window, Frame},
};

/// Where the sweep request is left for Lua to find on the next world load.
const REQUEST_FILE: &str = "ScrapMapCapture.json";
const PHOTO_DIRECTORY: &str = "photo";
/// Stored edge length. Generous enough to stay sharp when a tile fills the
/// screen, small enough that a hundred of them stay a reasonable cache.
const PHOTO_EDGE: u32 = 512;
/// Below this the frame is almost certainly a loading screen or a black
/// capture rather than terrain, and storing it would look like a hole.
const MIN_LIT_FRACTION: f32 = 0.35;
/// Minimum luminance spread for a frame to count as terrain. A camera that ends
/// up inside a hill, under a lake or above the clouds returns an evenly lit
/// sheet: bright enough to pass a brightness test, and completely featureless.
const MIN_DETAIL: f32 = 6.0;
/// Floor on the fraction of the frame a shot may keep. A malformed or absurd
/// `covered` must not reduce the capture to a handful of pixels.
const MIN_CROP: f32 = 0.2;
/// How the photographs are framed and cropped. **Raise this whenever a change
/// would make an existing photograph wrong**, and every tile is retaken once.
///
/// Keyed on this rather than on file timestamps: a completed sweep clears its
/// request and re-applying the patch touches the Lua, so neither is a usable
/// signal for "this photograph was taken by the current code".
///
/// Raised to 2 because replayed `ready` lines overwrote an unknown subset of
/// generation 1 with screenshots of the main menu. Which ones is not
/// recoverable, so all of them are retaken.
const CAPTURE_GENERATION: u32 = 2;
const STATE_FILE: &str = "photo-state.json";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTarget {
    pub uuid: String,
    pub x: i64,
    pub y: i64,
    pub size: u32,
    /// Absolute world height of the plane to frame against, so a clifftop POI is
    /// photographed from the same distance as one at sea level.
    ///
    /// This comes from the baked atlas rather than from a raycast in the game.
    /// The sweep spent three runs framing everything from sea level because the
    /// in-game ground probe silently never hit anything.
    pub ground_height: f64,
    /// How far the tile's high ground rises above that plane, and the height of
    /// the tallest thing standing on it. The sweep pulls the camera back for
    /// whichever is larger, so that neither leans out over the tile's edges.
    pub relief_height: f64,
    pub structure_height: f64,
    /// Quarter turns the chosen placement is rotated by, 0..3.
    ///
    /// The generated atlas samples tiles unrotated and the renderer turns them
    /// per placement. A photograph is of a real placement, so it already carries
    /// that rotation and would be turned a second time. Where the world offers
    /// an unrotated placement this is 0 and the two conventions agree.
    pub rotation: u32,
}

/// Chooses one representative placement per POI tile.
///
/// A tile appears many times over -- 405 placements across 116 distinct tiles
/// in the test world -- but the photograph keys on the tile, so only one
/// instance of each is worth visiting. Filler is skipped: it restates terrain
/// the generated atlas already draws.
///
/// `terrain` supplies the height to frame each tile against. A tile missing from
/// it falls back to sea level, which is wrong for anything on high ground --
/// `poi_capture_prepare` reports the count so that is visible rather than
/// silent.
pub fn build_targets(
    layout: &Value,
    terrain: &BTreeMap<String, atlas_bake::TileTerrain>,
) -> Vec<CaptureTarget> {
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
        let rotation = cell
            .get("rotation")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .rem_euclid(4.0) as u32;
        // Prefer an unrotated placement: photographed there, the picture matches
        // the convention the generated tiles already use.
        match chosen.get(uuid) {
            Some(existing) if existing.rotation == 0 || rotation != 0 => continue,
            _ => {}
        }
        let (Some(x), Some(y)) = (
            cell.get("x").and_then(Value::as_f64),
            cell.get("y").and_then(Value::as_f64),
        ) else {
            continue;
        };
        let ground = terrain.get(&uuid.to_ascii_lowercase());
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
                ground_height: ground.map(|value| f64::from(value.ground)).unwrap_or(0.0),
                relief_height: ground.map(|value| f64::from(value.relief)).unwrap_or(0.0),
                structure_height: ground.map(|value| f64::from(value.structure)).unwrap_or(0.0),
                rotation,
            },
        );
    }

    chosen.into_values().collect()
}

/// Drops targets already photographed since the outstanding request was written.
///
/// A sweep is fifteen minutes with the player's controls locked, and dying part
/// way through used to cost the whole run: the request is only cleared when Lua
/// reports `done`, so the next load started again from the first tile. Comparing
/// each photograph against the request it was taken for turns a second run into
/// a resumption -- and leaves a completed sweep alone, because by then the
/// request is gone and every target is offered afresh.
pub fn remaining_targets(atlas_root: &Path, targets: Vec<CaptureTarget>) -> Vec<CaptureTarget> {
    let done = photo_state(atlas_root);
    let directory = atlas_root.join("tiles").join(PHOTO_DIRECTORY);
    targets
        .into_iter()
        .filter(|target| {
            let key = target.uuid.to_ascii_lowercase();
            // The record and the file have to agree: a photograph deleted from
            // the cache must be retaken even if the state file remembers it.
            let current = done.get(&key).copied() == Some(CAPTURE_GENERATION);
            !(current && directory.join(format!("{key}.png")).exists())
        })
        .collect()
}

fn photo_state(atlas_root: &Path) -> BTreeMap<String, u32> {
    fs::read(atlas_root.join(STATE_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Records that a tile now has a photograph taken by the current generation.
pub fn note_capture(atlas_root: &Path, uuid: &str) {
    let mut state = photo_state(atlas_root);
    state.insert(uuid.to_ascii_lowercase(), CAPTURE_GENERATION);
    if let Ok(bytes) = serde_json::to_vec_pretty(&state) {
        let _ = fs::write(atlas_root.join(STATE_FILE), bytes);
    }
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

/// Parses `SCRAPMAP_SHOT_V1|ready|<uuid>|<x>|<y>|<size>|<metres>|<covered>|...`.
///
/// `covered` is the ground distance the full frame height spans. It exceeds the
/// tile whenever the camera pulled back to stop a tall building leaning out over
/// the tile's edges, and their ratio is the fraction of the frame to keep.
///
/// Everything after it is diagnostic and deliberately not read here, so Lua can
/// add to it without breaking capture. A line that stops early -- or one from an
/// older patch that never pulled back -- yields a whole-frame crop.
pub fn parse_ready(line: &str) -> Option<(String, u32, f32)> {
    let payload = line.split_once("SCRAPMAP_SHOT_V1|ready|")?.1.trim();
    let mut fields = payload.split('|');
    let uuid = fields.next()?.trim().to_owned();
    let _x = fields.next()?;
    let _y = fields.next()?;
    let size: u32 = fields.next()?.trim().parse().ok()?;
    let metres = fields.next().and_then(|value| value.trim().parse::<f32>().ok());
    let covered = fields.next().and_then(|value| value.trim().parse::<f32>().ok());
    let crop = match (metres, covered) {
        (Some(metres), Some(covered)) if metres > 0.0 && covered >= metres => metres / covered,
        _ => 1.0,
    };
    (!uuid.is_empty() && size > 0).then_some((uuid, size, crop.clamp(MIN_CROP, 1.0)))
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
    Ready {
        uuid: String,
        size: u32,
        /// Fraction of the frame height that is the tile itself.
        crop: f32,
    },
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
            // Start at the end of a log the first time it is seen, never at the
            // beginning. A `ready` line describes a pose the game held at some
            // moment; replaying an old one photographs whatever happens to be on
            // screen now. Restarting the overlay used to do exactly that -- it
            // re-read a whole session and overwrote good photographs with the
            // main menu, an exit dialog, and first-person views.
            self.path = Some(latest.clone());
            self.offset = fs::metadata(&latest).map(|data| data.len()).unwrap_or(0);
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
            if let Some((uuid, size, crop)) = parse_ready(&line) {
                events.push(ShotEvent::Ready { uuid, size, crop });
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
/// `crop` is the fraction of the frame height the tile occupies. It is 1.0 when
/// the camera framed the tile exactly, and less when it pulled back so that a
/// tall building would stand up rather than lean out; the surplus is discarded
/// here so the stored image still maps onto the tile's own footprint.
pub fn capture_tile(
    window_handle: isize,
    uuid: &str,
    crop: f32,
    atlas_root: &Path,
) -> Result<PathBuf, String> {
    let frame = capture_window(window_handle)?;
    let kept = ((frame.height as f32) * crop.clamp(MIN_CROP, 1.0)).round() as u32;
    let square = frame
        .centre_square(kept.max(1))
        .ok_or("the captured frame has no usable square")?;
    let lit = square.lit_fraction();
    if lit < MIN_LIT_FRACTION {
        return Err(format!(
            "frame is {:.0}% lit, which reads as a loading screen rather than terrain",
            lit * 100.0
        ));
    }
    let detail = square.detail();
    if detail < MIN_DETAIL {
        return Err(format!(
            "frame is featureless (detail {detail:.1}); the camera is probably inside terrain,              under water or above the cloud layer"
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

/// Records why a frame was thrown away.
///
/// Rejections used to go only to stderr, which nothing reads. A sweep could
/// therefore report `done` having quietly kept the previous run's photograph for
/// a third of the map, and the only way to notice was to measure the images.
pub fn note_rejection(atlas_root: &Path, uuid: &str, reason: &str) {
    let path = atlas_root.join("photo-rejects.log");
    let line = format!("{uuid} {reason}
");
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
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
        let targets = build_targets(&layout, &BTreeMap::new());
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].uuid, "aa");
        assert_eq!(targets[0].size, 4);
        assert_eq!(targets[1].uuid, "bb");
    }

    fn rotated_cell(uuid: &str, x: f64, rotation: f64) -> Value {
        let mut value = cell(uuid, x, 0.0, 1.0, Some("POI_RUIN"), (0.0, 0.0));
        value["rotation"] = json!(rotation);
        value
    }

    #[test]
    fn an_unrotated_placement_is_preferred_over_a_turned_one() {
        // A photograph carries the placement's rotation, and the renderer turns
        // tiles again when it draws them. Shooting an unrotated placement is
        // what keeps a photograph in the same convention as a generated tile.
        let layout = json!({ "cells": [
            rotated_cell("ruin", 0.0, 2.0),
            rotated_cell("ruin", 9.0, 0.0),
            rotated_cell("ruin", 4.0, 1.0),
            // This tile is only ever placed turned, so there is nothing to
            // prefer; the rotation is carried so the caller can see that.
            rotated_cell("turned", 1.0, 3.0),
            rotated_cell("turned", 6.0, 3.0),
        ]});
        let targets = build_targets(&layout, &BTreeMap::new());
        let ruin = targets.iter().find(|t| t.uuid == "ruin").unwrap();
        assert_eq!(ruin.rotation, 0, "the unrotated placement should win");
        assert_eq!(ruin.x, 9, "and it should be that placement's position");
        assert_eq!(targets.iter().find(|t| t.uuid == "turned").unwrap().rotation, 3);
    }

    #[test]
    fn a_target_is_framed_against_its_own_ground_when_the_atlas_knows_it() {
        let layout = json!({ "cells": [
            cell("HIGH", 0.0, 0.0, 1.0, Some("POI_TOWER"), (0.0, 0.0)),
            cell("unbaked", 1.0, 0.0, 1.0, Some("POI_CAMP"), (0.0, 0.0)),
        ]});
        let mut terrain = BTreeMap::new();
        terrain.insert(
            "high".to_owned(),
            atlas_bake::TileTerrain {
                ground: 47.5,
                relief: 12.0,
                structure: 31.0,
            },
        );

        let targets = build_targets(&layout, &terrain);
        let high = targets.iter().find(|t| t.uuid == "HIGH").unwrap();
        assert_eq!(high.ground_height, 47.5);
        assert_eq!(high.relief_height, 12.0);
        assert_eq!(high.structure_height, 31.0);
        // A tile the atlas has never baked falls back to sea level. That is
        // wrong for high ground, which is why the count is reported to the user
        // rather than swallowed.
        let unbaked = targets.iter().find(|t| t.uuid == "unbaked").unwrap();
        assert_eq!(unbaked.ground_height, 0.0);
    }

    #[test]
    fn generator_filler_and_plain_terrain_are_not_photographed() {
        let layout = json!({ "cells": [
            cell("lake", 0.0, 0.0, 1.0, Some("POI_LAKE_RANDOM"), (0.0, 0.0)),
            cell("road", 1.0, 0.0, 1.0, Some("POI_ROAD_RANDOM"), (0.0, 0.0)),
            cell("grass", 2.0, 0.0, 1.0, None, (0.0, 0.0)),
        ]});
        assert!(build_targets(&layout, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn multi_cell_tiles_are_aimed_from_their_origin_cell() {
        // Only the zero-offset cell carries the tile's true position.
        let layout = json!({ "cells": [
            cell("ship", 9.0, 9.0, 4.0, Some("POI_CRASHSITE_AREA"), (1.0, 2.0)),
            cell("ship", 5.0, 4.0, 4.0, Some("POI_CRASHSITE_AREA"), (0.0, 0.0)),
        ]});
        let targets = build_targets(&layout, &BTreeMap::new());
        assert_eq!(targets.len(), 1);
        assert_eq!((targets[0].x, targets[0].y), (5, 4));
    }

    #[test]
    fn ready_lines_parse_out_of_the_game_log() {
        // A camera that framed the tile exactly keeps the whole frame.
        let line = "12:00:01 [Lua] SCRAPMAP_SHOT_V1|ready|abc-123|-38|-42|4|256.00|256.00|221.7|hit|148.3|0.0";
        assert_eq!(parse_ready(line), Some(("abc-123".to_owned(), 4, 1.0)));
        assert_eq!(parse_ready("unrelated log line"), None);
        // A truncated line must not be taken as a cue to capture.
        assert_eq!(parse_ready("SCRAPMAP_SHOT_V1|ready|abc-123|-38"), None);
        assert!(is_sweep_finished("[Lua] SCRAPMAP_SHOT_V1|done|116"));
        assert!(!is_sweep_finished("[Lua] SCRAPMAP_SHOT_V1|ready|a|1|2|1|64"));
    }

    #[test]
    fn a_pulled_back_camera_reports_the_fraction_of_the_frame_to_keep() {
        // 256 m of tile inside 640 m of view: keep two fifths of the frame.
        let (_, _, crop) =
            parse_ready("SCRAPMAP_SHOT_V1|ready|ship|-38|-42|4|256.00|640.00|554.3|hit|150.0|62.0")
                .unwrap();
        assert!((crop - 0.4).abs() < 1e-6, "crop was {crop}");

        // Lines from a patch that predates the pull-back keep the whole frame,
        // as do nonsensical ones -- a bad number must not shrink the capture to
        // nothing.
        let old = parse_ready("SCRAPMAP_SHOT_V1|ready|camp|1|2|1|64.00").unwrap();
        assert_eq!(old.2, 1.0);
        let absurd = parse_ready("SCRAPMAP_SHOT_V1|ready|camp|1|2|1|64.00|100000.0").unwrap();
        assert_eq!(absurd.2, MIN_CROP);
        let backwards = parse_ready("SCRAPMAP_SHOT_V1|ready|camp|1|2|1|64.00|8.0").unwrap();
        assert_eq!(backwards.2, 1.0);
    }

    #[test]
    fn a_watcher_ignores_everything_written_before_it_attached() {
        // A `ready` line describes a pose the game held at the time. Acting on
        // an old one photographs whatever is on screen now -- which is how a
        // restart of the overlay replaced good photographs with the main menu.
        let root = std::env::temp_dir().join(format!(
            "scrapmap-watcher-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default()
        ));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("game-20260101-010101.log");
        fs::write(
            &log,
            "SCRAPMAP_SHOT_V1|ready|old-tile|1|2|1|64.00|64.00|55.4|atlas|9|0
",
        )
        .unwrap();

        let mut watcher = ShotWatcher::default();
        assert!(
            watcher.poll(&root).is_empty(),
            "history must not be replayed"
        );

        // Anything appended afterwards is live and must be acted on.
        use std::io::Write as _;
        let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
        file.write_all(b"SCRAPMAP_SHOT_V1|ready|new-tile|3|4|2|128.00|128.00|110.9|atlas|9|0
")
            .unwrap();
        drop(file);

        let events = watcher.poll(&root);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], ShotEvent::Ready { uuid, .. } if uuid == "new-tile"));

        fs::remove_dir_all(&root).ok();
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
            relief_height: 0.0,
            structure_height: 0.0,
            rotation: 0,
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
