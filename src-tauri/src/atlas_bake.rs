//! Converts the Lua-baked terrain rasters into map tile images.
//!
//! The game-side baker (`Survival/Scripts/terrain/ScrapMapAtlasBake.lua`) samples
//! every registered tile through `sm.terrainTile` and writes one JSON per tile
//! UUID. This module decodes those rasters, shades them, and emits PNGs into the
//! existing atlas cache as `generated/<uuid>.png`, then points each manifest
//! entry's `topDownRelativePath` at the result so the renderer picks them up
//! without any frontend change.
//!
//! Tiles are sampled unrotated, so the output depends only on the game build.

use std::{
    collections::{BTreeMap, HashMap},
    env,
    fs,
    path::{Path, PathBuf},
};

use crate::asset_catalogue::{self, AssetInfo, AssetKind, GroundCover};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

pub(crate) const GENERATED_DIRECTORY: &str = "generated";
/// Where the POI photography sweep leaves its captures. Named here as well as in
/// `poi_capture` because the manifest merge has to know a photograph outranks
/// the tile it would otherwise point at.
const PHOTO_DIRECTORY: &str = "photo";
const MAX_BAKED_TILE_BYTES: u64 = 32 * 1024 * 1024;
/// Survival tiles run from 1x1 up to 16x16 cells, so at 64 samples per cell the
/// largest legitimate raster is 1024x1024. Allow one size step beyond that.
const MAX_SPAN: usize = 2048;

/// How many output pixels are drawn per sampled one.
///
/// The game is asked for 64 material samples per cell and that is all the
/// terrain detail there is; upscaling invents none. What gains is everything
/// drawn *into* the tile -- at one pixel per metre a tree is a three-pixel disc,
/// which is where "blobby" comes from. Large tiles are capped so no image
/// exceeds `MAX_RENDER_SPAN`.
const RENDER_SCALE: usize = 4;
const MAX_RENDER_SPAN: usize = 2048;

fn render_scale_for(span: usize) -> usize {
    if span == 0 {
        return 1;
    }
    (MAX_RENDER_SPAN / span).clamp(1, RENDER_SCALE)
}

/// Strength of the relief shading, as a peak +/- multiplier on the base colour.
/// Terrain colour already carries roads and biome, so this only needs to hint
/// at slope without washing the palette out.
const HILLSHADE_STRENGTH: f32 = 0.28;
/// Metres of rise per sample step that saturates the shading ramp.
const HILLSHADE_SCALE: f32 = 6.0;

/// Depth at which terrain reads as water. Ordinary tiles only graze a few
/// centimetres below zero at their edges, while lake beds cut well past this.
/// The burnt crash trench bottoms out around -3 m, so the threshold sits below
/// it; the material gate below rejects whatever else comes close.
const WATER_LEVEL: f32 = -2.5;
/// Metres below the waterline at which the water is fully opaque.
const WATER_OPAQUE_DEPTH: f32 = 6.0;
const WATER_SHALLOW: [f32; 3] = [104.0, 170.0, 198.0];
const WATER_DEEP: [f32; 3] = [38.0, 86.0, 140.0];

/// Depth alone mistakes gouges for rivers: the crash-site trench and sunken
/// roads cut several metres down but are dirt, while lake beds sample as rock
/// or sand. Requiring a lakebed material keeps trenches dry.
fn holds_water(surface_class: u8) -> bool {
    surface_class == 1 || surface_class == 3 // sand or rock
}

/// Lua has one numeric type, so `sm.json.save` writes every count as a float
/// (`512.0`). Serde will not coerce that into an integer, so read it as a
/// double and round.
fn lua_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let value = f64::deserialize(deserializer)?;
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return Err(serde::de::Error::custom(format!(
            "expected a non-negative count, found {value}"
        )));
    }
    Ok(value.round() as u32)
}

fn lua_span<'de, D: Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    lua_number(deserializer).map(|value| value as usize)
}

fn default_size() -> u32 {
    1
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BakedTileFile {
    uuid: String,
    #[serde(default = "default_size", deserialize_with = "lua_number")]
    size: u32,
    #[serde(default, deserialize_with = "lua_span")]
    material_span: usize,
    #[serde(default, deserialize_with = "lua_span")]
    color_span: usize,
    #[serde(default, deserialize_with = "lua_span")]
    height_span: usize,
    #[serde(default)]
    material: String,
    #[serde(default)]
    color: String,
    #[serde(default)]
    height: String,
    #[serde(default, deserialize_with = "lua_span")]
    clutter_span: usize,
    #[serde(default)]
    clutter: Option<String>,
    /// Lua writes an empty table as `null` rather than `[]`, and serde's
    /// `default` only covers a missing field, so this has to tolerate null.
    #[serde(default)]
    asset_palette: Option<Vec<String>>,
    #[serde(default)]
    assets: Option<String>,
    #[serde(default, deserialize_with = "lua_span")]
    asset_stride: usize,
}

/// Decodes the clutter raster: one two-digit index per sample into the game's
/// clutter list, with 0xFF meaning "nothing here".
fn decode_clutter(
    text: &str,
    span: usize,
    table: &[GroundCover],
) -> Result<Vec<GroundCover>, String> {
    if text.is_empty() || span == 0 {
        return Ok(Vec::new());
    }
    if span > MAX_SPAN {
        return Err(format!("implausible clutter span {span}"));
    }
    let expected = span * span;
    if text.len() != expected * 2 {
        return Err(format!(
            "clutter raster expected {} characters, found {}",
            expected * 2,
            text.len()
        ));
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let slice = std::str::from_utf8(chunk).map_err(|_| "clutter is not ASCII")?;
            let index = usize::from_str_radix(slice, 16)
                .map_err(|_| "clutter contains a non-hex character".to_owned())?;
            // Unknown indices fall back to bare ground rather than inventing a
            // tint, so a clutter list we do not recognise simply does nothing.
            Ok(table.get(index).copied().unwrap_or(GroundCover::Bare))
        })
        .collect()
}

/// One placed object, in tile-local metres measured from the south-west corner.
#[derive(Clone, Copy, Debug)]
pub struct PlacedAsset {
    pub palette_index: usize,
    pub x: f32,
    pub y: f32,
    /// Yaw in radians. Zero for a stream baked before the angle was recorded.
    pub yaw: f32,
}

/// Decodes the baker's asset stream: palette index, x and y, three hex digits
/// each, with the coordinates in quarter-metres.
fn decode_assets(text: &str, stride: usize) -> Result<Vec<PlacedAsset>, String> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    // The baker names its stride rather than leaving it to be inferred: 9 and
    // 11 both divide plausible stream lengths, so guessing is ambiguous.
    // Prefer what the baker says, but accept the other width if that is the one
    // that actually divides -- a stream written by a baker that records the
    // stride and a reader that does not agree on it should still be legible.
    let stride = [if stride == 11 { 11 } else { 9 }, 9, 11]
        .into_iter()
        .find(|candidate| text.len().is_multiple_of(*candidate))
        .ok_or_else(|| format!("asset stream length {} fits no known stride", text.len()))?;
    let field = |slice: &str| usize::from_str_radix(slice, 16).map_err(|_| "bad hex".to_owned());
    let mut placed = Vec::with_capacity(text.len() / stride);
    for chunk in text.as_bytes().chunks_exact(stride) {
        let chunk = std::str::from_utf8(chunk).map_err(|_| "asset stream is not ASCII")?;
        placed.push(PlacedAsset {
            palette_index: field(&chunk[0..3])?,
            x: field(&chunk[3..6])? as f32 / 4.0,
            y: field(&chunk[6..9])? as f32 / 4.0,
            yaw: if stride == 11 {
                field(&chunk[9..11])? as f32 / 256.0 * std::f32::consts::TAU
            } else {
                0.0
            },
        });
    }
    Ok(placed)
}

#[cfg(test)]
fn decode_assets_legacy(text: &str) -> Result<Vec<PlacedAsset>, String> {
    decode_assets(text, 9)
}

/// Paints placed objects over the finished terrain.
///
/// Larger objects are drawn first so that undergrowth reads on top of the tree
/// it sits beneath, and each blob fades at its rim so a forest looks like
/// canopy rather than a field of hard discs.
pub fn draw_assets(
    rgba: &mut [u8],
    span: usize,
    assets: &[PlacedAsset],
    palette: &[Option<AssetInfo>],
    pixels_per_metre: f32,
    height: &[f32],
    height_span: usize,
    tile_metres: f32,
) {
    // Lakes carry seaplants, lily pads and sprouts. They are really there, but
    // at map scale they read as trees floating in open water, so submerged
    // vegetation is dropped. Ruins and rocks in a lake are landmarks and stay.
    let submerged = |x: f32, y: f32| -> bool {
        if height_span == 0 || height.len() != height_span * height_span || tile_metres <= 0.0 {
            return false;
        }
        let to_index = |value: f32| {
            ((value / tile_metres * height_span as f32) as isize)
                .clamp(0, height_span as isize - 1) as usize
        };
        height[to_index(y) * height_span + to_index(x)] < WATER_LEVEL
    };

    let mut order: Vec<&PlacedAsset> = assets.iter().collect();
    order.sort_by(|a, b| {
        let radius = |item: &PlacedAsset| {
            palette
                .get(item.palette_index)
                .and_then(|entry| entry.as_ref())
                .map_or(0.0, |info| info.radius)
        };
        radius(b)
            .partial_cmp(&radius(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for asset in order {
        let Some(Some(info)) = palette.get(asset.palette_index) else {
            continue;
        };
        if info.kind == AssetKind::Skip {
            continue;
        }
        if matches!(info.kind, AssetKind::Foliage | AssetKind::Debris)
            && submerged(asset.x, asset.y)
        {
            continue;
        }
        let radius = info.radius * pixels_per_metre;
        if !(radius > 0.4) {
            continue;
        }
        let centre_x = asset.x * pixels_per_metre;
        // Rows run north to south while asset coordinates count northward.
        let centre_y = span as f32 - asset.y * pixels_per_metre;
        let opacity = info.kind.opacity();

        // A building is a building shape. A disc is right for a tree canopy and
        // wrong for a warehouse, and the collision mesh knows the difference --
        // it is where the radius came from in the first place.
        if draws_as_a_shape(info.kind) && info.footprint.len() >= 3 {
            fill_footprint(
                rgba,
                span,
                &info.footprint,
                [centre_x, centre_y],
                asset.yaw,
                pixels_per_metre,
                info.color,
                opacity,
            );
            continue;
        }

        let min_x = ((centre_x - radius).floor().max(0.0)) as usize;
        let max_x = ((centre_x + radius).ceil().min(span as f32 - 1.0)) as usize;
        let min_y = ((centre_y - radius).floor().max(0.0)) as usize;
        let max_y = ((centre_y + radius).ceil().min(span as f32 - 1.0)) as usize;

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = x as f32 + 0.5 - centre_x;
                let dy = y as f32 + 0.5 - centre_y;
                let distance = (dx * dx + dy * dy).sqrt();
                if distance > radius {
                    continue;
                }
                // Solid to about 60% of the radius, then fading to the rim.
                let edge = ((radius - distance) / (radius * 0.4)).clamp(0.0, 1.0);
                let alpha = opacity * edge;
                let offset = (y * span + x) * 4;
                for channel in 0..3 {
                    let existing = f32::from(rgba[offset + channel]);
                    let target = f32::from(info.color[channel]);
                    rgba[offset + channel] =
                        (existing * (1.0 - alpha) + target * alpha).clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// Whether this kind reads better as its outline than as a blob.
///
/// Foliage does not: a canopy really is a soft circle from above, and a tree's
/// collision hull is a scrappy polygon that would look worse.
fn draws_as_a_shape(kind: AssetKind) -> bool {
    matches!(kind, AssetKind::Building | AssetKind::Wreck | AssetKind::Rock)
}

/// Fills a rotated outline, with a darker edge so adjoining buildings stay
/// legible as separate ones rather than merging into a slab.
#[allow(clippy::too_many_arguments)]
fn fill_footprint(
    rgba: &mut [u8],
    span: usize,
    footprint: &[[f32; 2]],
    centre: [f32; 2],
    yaw: f32,
    pixels_per_metre: f32,
    color: [u8; 3],
    opacity: f32,
) {
    let (sin, cos) = yaw.sin_cos();
    let points: Vec<[f32; 2]> = footprint
        .iter()
        .map(|[x, y]| {
            let rx = x * cos - y * sin;
            let ry = x * sin + y * cos;
            // Rows run north to south while asset coordinates count northward.
            [
                centre[0] + rx * pixels_per_metre,
                centre[1] - ry * pixels_per_metre,
            ]
        })
        .collect();

    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for point in &points {
        min_x = min_x.min(point[0]);
        max_x = max_x.max(point[0]);
        min_y = min_y.min(point[1]);
        max_y = max_y.max(point[1]);
    }
    if !(max_x - min_x > 1.0 && max_y - min_y > 1.0) {
        return;
    }

    let inside = |px: f32, py: f32| {
        // Even-odd crossing count. The hull is convex, but this does not care
        // and costs nothing extra at these sizes.
        let mut hit = false;
        let mut j = points.len() - 1;
        for i in 0..points.len() {
            let (a, b) = (points[i], points[j]);
            if (a[1] > py) != (b[1] > py) {
                let cut = (b[0] - a[0]) * (py - a[1]) / (b[1] - a[1]) + a[0];
                if px < cut {
                    hit = !hit;
                }
            }
            j = i;
        }
        hit
    };

    let x0 = min_x.floor().max(0.0) as usize;
    let x1 = (max_x.ceil().min(span as f32 - 1.0)).max(0.0) as usize;
    let y0 = min_y.floor().max(0.0) as usize;
    let y1 = (max_y.ceil().min(span as f32 - 1.0)).max(0.0) as usize;

    for y in y0..=y1 {
        for x in x0..=x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            if !inside(px, py) {
                continue;
            }
            // A pixel with a neighbour outside is a rim pixel, and gets a
            // darker shade so two buildings side by side still read as two.
            let rim = !inside(px - 1.0, py)
                || !inside(px + 1.0, py)
                || !inside(px, py - 1.0)
                || !inside(px, py + 1.0);
            let shade = if rim { 0.72 } else { 1.0 };
            let offset = (y * span + x) * 4;
            for channel in 0..3 {
                let existing = f32::from(rgba[offset + channel]);
                let target = f32::from(color[channel]) * shade;
                rgba[offset + channel] =
                    (existing * (1.0 - opacity) + target * opacity).clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Base colours for the four surface classes the baker records. These are what
/// make roads and shorelines legible; the sampled tint only nudges them.
const SURFACE_PALETTE: [[f32; 3]; 4] = [
    [104.0, 142.0, 68.0],  // grass
    [214.0, 192.0, 143.0], // sand
    [146.0, 118.0, 84.0],  // dirt
    [143.0, 142.0, 138.0], // rock
];

#[derive(Clone, Debug, Serialize)]
pub struct AtlasBakeReportV1 {
    pub converted: usize,
    pub skipped: usize,
    pub failed: usize,
    pub total_baked: usize,
}

/// Decodes a run of big-endian `u16` values written as fixed-width hex.
fn decode_hex_u16(text: &str, expected: usize) -> Result<Vec<u16>, String> {
    let bytes = text.as_bytes();
    if bytes.len() != expected * 4 {
        return Err(format!(
            "expected {} hex characters, found {}",
            expected * 4,
            bytes.len()
        ));
    }
    let mut values = Vec::with_capacity(expected);
    for chunk in bytes.chunks_exact(4) {
        let mut value: u16 = 0;
        for &byte in chunk {
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err("raster contains a non-hex character".to_owned()),
            };
            value = (value << 4) | u16::from(digit);
        }
        values.push(value);
    }
    Ok(values)
}

/// Decodes the surface-class raster: one hex digit per sample, 0..3.
fn decode_surface(text: &str, expected: usize) -> Result<Vec<u8>, String> {
    let bytes = text.as_bytes();
    if bytes.len() != expected {
        return Err(format!(
            "expected {expected} characters, found {}",
            bytes.len()
        ));
    }
    bytes
        .iter()
        .map(|&byte| match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            b'A'..=b'F' => Ok(byte - b'A' + 10),
            _ => Err("raster contains a non-hex character".to_owned()),
        })
        .collect()
}

fn rgb565_to_rgb888(value: u16) -> [u8; 3] {
    let r5 = ((value >> 11) & 0x1F) as u32;
    let g6 = ((value >> 5) & 0x3F) as u32;
    let b5 = (value & 0x1F) as u32;
    // Replicate the high bits into the low ones so full-scale stays full-scale.
    [
        ((r5 * 255 + 15) / 31) as u8,
        ((g6 * 255 + 31) / 63) as u8,
        ((b5 * 255 + 15) / 31) as u8,
    ]
}

fn decode_height(value: u16) -> f32 {
    (f32::from(value) - 32768.0) / 10.0
}

fn sample_height(height: &[f32], span: usize, x: isize, y: isize) -> f32 {
    if span == 0 {
        return 0.0;
    }
    let cx = x.clamp(0, span as isize - 1) as usize;
    let cy = y.clamp(0, span as isize - 1) as usize;
    height[cy * span + cx]
}

/// Renders one baked tile to an RGBA buffer at the colour raster's resolution.
///
/// Row 0 of the baked raster is the tile's south edge, but image rows run top
/// down, so rows are emitted in reverse to put north at the top.
#[derive(Clone, Copy, Default)]
pub struct GroundCoverLayer<'a> {
    pub cover: &'a [GroundCover],
    pub span: usize,
}

/// One output pixel per sampled one. Only the tests want that now -- the bake
/// itself renders at `RENDER_SCALE` -- but they read far better for it.
#[cfg(test)]
pub fn render_tile_rgba(
    surface: &[u8],
    span: usize,
    color: &[u16],
    color_span: usize,
    height: &[f32],
    height_span: usize,
    ground: GroundCoverLayer<'_>,
) -> Vec<u8> {
    render_tile_rgba_scaled(surface, span, color, color_span, height, height_span, ground, 1)
}

/// As `render_tile_rgba`, drawing `scale` output pixels per sampled one.
#[allow(clippy::too_many_arguments)]
pub fn render_tile_rgba_scaled(
    surface: &[u8],
    span: usize,
    color: &[u16],
    color_span: usize,
    height: &[f32],
    height_span: usize,
    ground: GroundCoverLayer<'_>,
    scale: usize,
) -> Vec<u8> {
    let scale = scale.max(1);
    let out = span * scale;
    let mut rgba = vec![0_u8; out * out * 4];
    let shade_available = height_span > 1 && height.len() == height_span * height_span;
    let tint_available = color_span > 0 && color.len() == color_span * color_span;
    let cover_available =
        ground.span > 0 && ground.cover.len() == ground.span * ground.span;

    for row in 0..out {
        for column in 0..out {
            let sx = column / scale;
            let sy = row / scale;
            let class = usize::from(surface[sy * span + sx]).min(SURFACE_PALETTE.len() - 1);
            let base = SURFACE_PALETTE[class];

            // The sampled colour is a tint over the material, and sits near
            // white for most terrain, so apply it as a multiplier rather than
            // as the colour itself.
            let [mut r, mut g, mut b] = if tint_available {
                let tx = column * color_span / out;
                let ty = row * color_span / out;
                let [tr, tg, tb] = rgb565_to_rgb888(color[ty * color_span + tx]);
                [
                    (base[0] * f32::from(tr) / 255.0) as u8,
                    (base[1] * f32::from(tg) / 255.0) as u8,
                    (base[2] * f32::from(tb) / 255.0) as u8,
                ]
            } else {
                [base[0] as u8, base[1] as u8, base[2] as u8]
            };

            // Ground cover sits between the surface material and the water and
            // relief passes: it distinguishes meadow from forest floor from
            // burnt stubble, all of which sample as plain grass.
            if cover_available {
                let cx = column * ground.span / out;
                let cy = row * ground.span / out;
                if let Some((target, strength)) = ground.cover[cy * ground.span + cx].tint() {
                    for (index, channel) in [&mut r, &mut g, &mut b].into_iter().enumerate() {
                        let blended = f32::from(*channel) * (1.0 - strength)
                            + target[index] * strength;
                        *channel = blended.clamp(0.0, 255.0) as u8;
                    }
                }
            }

            if shade_available {
                // Map the sample onto the coarser height raster.
                let hx = (column * height_span / out) as isize;
                let hy = (row * height_span / out) as isize;

                // Anything below the waterline is drawn as water, deepening
                // with distance below it, and is left unshaded because a water
                // surface is flat regardless of the bed beneath it.
                let depth = WATER_LEVEL - sample_height(height, height_span, hx, hy);
                if depth > 0.0 && holds_water(surface[sy * span + sx]) {
                    let t = (depth / WATER_OPAQUE_DEPTH).clamp(0.0, 1.0);
                    let opacity = 0.55 + 0.45 * t;
                    for (index, channel) in [&mut r, &mut g, &mut b].into_iter().enumerate() {
                        let water = WATER_SHALLOW[index]
                            + (WATER_DEEP[index] - WATER_SHALLOW[index]) * t;
                        let blended =
                            f32::from(*channel) * (1.0 - opacity) + water * opacity;
                        *channel = blended.clamp(0.0, 255.0) as u8;
                    }
                    let flipped = out - 1 - row;
                    let offset = (flipped * out + column) * 4;
                    rgba[offset] = r;
                    rgba[offset + 1] = g;
                    rgba[offset + 2] = b;
                    rgba[offset + 3] = 255;
                    continue;
                }

                let dzdx = sample_height(height, height_span, hx + 1, hy)
                    - sample_height(height, height_span, hx - 1, hy);
                let dzdy = sample_height(height, height_span, hx, hy + 1)
                    - sample_height(height, height_span, hx, hy - 1);
                // Light from the north-west, matching how the reference maps read.
                let slope = (-dzdx + dzdy) / (2.0 * HILLSHADE_SCALE);
                let factor = 1.0 + slope.clamp(-1.0, 1.0) * HILLSHADE_STRENGTH;
                r = (f32::from(r) * factor).clamp(0.0, 255.0) as u8;
                g = (f32::from(g) * factor).clamp(0.0, 255.0) as u8;
                b = (f32::from(b) * factor).clamp(0.0, 255.0) as u8;
            }

            let flipped = out - 1 - row;
            let offset = (flipped * out + column) * 4;
            rgba[offset] = r;
            rgba[offset + 1] = g;
            rgba[offset + 2] = b;
            rgba[offset + 3] = 255;
        }
    }

    rgba
}

fn encode_png(rgba: &[u8], span: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, span as u32, span as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("png header: {error}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| format!("png data: {error}"))?;
    }
    Ok(out)
}

fn convert_one(
    bytes: &[u8],
    catalogue: &HashMap<String, AssetInfo>,
    ground_cover: &[GroundCover],
) -> Result<(String, Vec<u8>, u32), String> {
    let tile: BakedTileFile =
        serde_json::from_slice(bytes).map_err(|error| format!("parse: {error}"))?;

    if tile.material_span == 0 || tile.material_span > MAX_SPAN {
        return Err(format!("implausible material span {}", tile.material_span));
    }
    if tile.color_span > MAX_SPAN || tile.height_span > MAX_SPAN {
        return Err("implausible tint or height span".to_owned());
    }

    let surface = decode_surface(&tile.material, tile.material_span * tile.material_span)
        .map_err(|error| format!("material raster: {error}"))?;
    let color = decode_hex_u16(&tile.color, tile.color_span * tile.color_span)
        .map_err(|error| format!("colour raster: {error}"))?;
    let height = if tile.height.is_empty() || tile.height_span == 0 {
        Vec::new()
    } else {
        decode_hex_u16(&tile.height, tile.height_span * tile.height_span)
            .map_err(|error| format!("height raster: {error}"))?
            .into_iter()
            .map(decode_height)
            .collect()
    };

    let clutter = decode_clutter(
        tile.clutter.as_deref().unwrap_or_default(),
        tile.clutter_span,
        ground_cover,
    )?;
    let scale = render_scale_for(tile.material_span);
    let render_span = tile.material_span * scale;
    let mut rgba = render_tile_rgba_scaled(
        &surface,
        tile.material_span,
        &color,
        tile.color_span,
        &height,
        tile.height_span,
        GroundCoverLayer {
            cover: &clutter,
            span: if clutter.is_empty() { 0 } else { tile.clutter_span },
        },
        scale,
    );

    // Trees, rocks, buildings and the crashed ship are assets rather than
    // terrain, so nothing above sees them.
    // Terrain does not depend on the objects, so an unreadable asset stream
    // costs the buildings on a tile, not the tile. Failing the whole conversion
    // left four hundred cells blank on the map.
    let placed = match decode_assets(tile.assets.as_deref().unwrap_or_default(), tile.asset_stride)
    {
        Ok(placed) => placed,
        Err(error) => {
            eprintln!("ScrapMap could not read the objects on {}: {error}", tile.uuid);
            Vec::new()
        }
    };
    if !placed.is_empty() {
        let palette: Vec<Option<AssetInfo>> = tile
            .asset_palette
            .unwrap_or_default()
            .iter()
            .map(|uuid| catalogue.get(&uuid.to_ascii_lowercase()).cloned())
            .collect();
        let tile_metres = (tile.size.max(1) as f32) * 64.0;
        let pixels_per_metre = render_span as f32 / tile_metres;
        draw_assets(
            &mut rgba,
            render_span,
            &placed,
            &palette,
            pixels_per_metre,
            &height,
            tile.height_span,
            tile_metres,
        );
    }

    let png = encode_png(&rgba, render_span)?;
    Ok((tile.uuid, png, tile.size))
}

pub fn atlas_root() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("ScrapMap").join("atlas"))
}

/// Rewrites the manifest so every generated tile is the preferred image.
///
/// Existing entries keep their identity and simply gain a `topDownRelativePath`;
/// tiles the manifest has never seen are appended, which is what makes the new
/// start-area and quest tiles renderable at all.
/// `generated` maps tile UUID to its cell size, or `None` when the PNG was
/// already current and the size was therefore never read.
fn merge_manifest(root: &Path, generated: &BTreeMap<String, Option<u32>>) -> Result<(), String> {
    let manifest_path = root.join("manifest.json");
    let mut manifest: Value = match fs::read(&manifest_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("manifest parse: {error}"))?,
        Err(_) => json!({ "kind": "scrapmap.tile-atlas", "entries": [] }),
    };

    let entries = manifest
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .ok_or("manifest has no entries array")?;

    let mut seen = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        if let Some(uuid) = entry.get("tileUuid").and_then(Value::as_str) {
            seen.insert(uuid.to_ascii_lowercase(), index);
        }
    }

    for (uuid, size) in generated {
        let key = uuid.to_ascii_lowercase();
        let relative = format!("{GENERATED_DIRECTORY}/{key}.png");
        // A photograph outranks the generated tile, and the photo directory is
        // the authority on whether there is one. Deciding this here rather than
        // preserving whatever the manifest happened to say makes the pass
        // self-healing: it re-points tiles whose photographs an earlier version
        // of this function stamped over.
        let photo = format!("{PHOTO_DIRECTORY}/{key}.png");
        let has_photo = root.join("tiles").join(&photo).exists();
        let (top_down, kind) = if has_photo {
            (photo.clone(), "photo")
        } else {
            (relative.clone(), "generated")
        };
        match seen.get(&key) {
            Some(&index) => {
                let entry = &mut entries[index];
                entry["topDownRelativePath"] = json!(top_down);
                entry["topDownSourceKind"] = json!(kind);
                if let Some(size) = size {
                    entry["tileSize"] = json!(size);
                }
                // Generated tiles render the crash site, warehouses and other
                // landmarks as they are in this build. The imported sm_overview
                // POI photos are years out of date and sat on top of them at a
                // hand-tuned offset, so drop them once we have real terrain.
                if let Some(object) = entry.as_object_mut() {
                    for key in [
                        "poiOverlayRelativePath",
                        "poiOverlayOffsetX",
                        "poiOverlayOffsetY",
                        "poiOverlayTileSize",
                    ] {
                        object.remove(key);
                    }
                }
            }
            None => entries.push(json!({
                "tileUuid": key,
                "relativePath": relative,
                "topDownRelativePath": top_down,
                "topDownSourceKind": kind,
                "tileSize": size.unwrap_or(1),
            })),
        }
    }

    manifest["generatedTiles"] = json!(generated.len());
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|error| format!("serialise: {error}"))?,
    )
    .map_err(|error| format!("write manifest: {error}"))
}

/// The ground a tile should be photographed against.
///
/// Heights are absolute world Z in metres, which was checked against telemetry:
/// with the player on foot, its reported height sits inside its own tile's
/// sampled band 79,481 times out of 79,540, and every exception is *above* the
/// band -- a player standing on a base or a building, not a shifted origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileTerrain {
    /// Median sampled height: the plane most of the tile actually sits at, and
    /// so the one to frame against.
    pub ground: f32,
    /// How far the tile's high ground rises above that plane. Terrain leans out
    /// over a top-down frame exactly the way a building does, so the sweep pulls
    /// the camera back for a hill as well as for a tower.
    pub relief: f32,
    /// Height of the tallest thing standing on the tile, from the collision mesh
    /// of every asset the bake recorded. Measured here rather than raycast in
    /// the game, because the in-game probe returned nothing for three sweeps and
    /// the pull-back it was supposed to drive never once engaged.
    pub structure: f32,
}

/// Reads the ground height of every baked tile.
///
/// This exists because a downward raycast is not a reliable way to find the
/// ground from the air, and the atlas already holds the answer -- sampled by the
/// game itself, for every tile, with no physics involved.
pub fn tile_terrain(game_root: &Path, cache_root: &Path) -> BTreeMap<String, TileTerrain> {
    let baked_dir = game_root.join("Survival").join("ScrapMapAtlas");
    let Ok(listing) = fs::read_dir(&baked_dir) else {
        return BTreeMap::new();
    };
    let catalogue = asset_catalogue::load(game_root, cache_root);

    let mut terrain = BTreeMap::new();
    for entry in listing.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|value| value != "json") {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let Ok(tile) = serde_json::from_slice::<BakedTileFile>(&bytes) else {
            continue;
        };
        if tile.height_span == 0 || tile.height_span > MAX_SPAN {
            continue;
        }
        let Ok(raw) = decode_hex_u16(&tile.height, tile.height_span * tile.height_span) else {
            continue;
        };
        let mut heights: Vec<f32> = raw.into_iter().map(decode_height).collect();
        if heights.is_empty() {
            continue;
        }
        heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let at = |fraction: f32| heights[((heights.len() as f32 * fraction) as usize).min(heights.len() - 1)];
        let ground = at(0.5);
        // The palette lists every asset the tile places, which is all this
        // needs: the tallest one decides the pull-back wherever it stands.
        let structure = tile
            .asset_palette
            .unwrap_or_default()
            .iter()
            .filter_map(|uuid| catalogue.get(&uuid.to_ascii_lowercase()))
            .map(|asset| asset.height)
            .fold(0.0_f32, f32::max);
        // The 95th percentile rather than the maximum: one spike should not pull
        // the camera back for the whole tile.
        terrain.insert(
            tile.uuid.to_ascii_lowercase(),
            TileTerrain {
                ground,
                relief: (at(0.95) - ground).max(0.0),
                structure,
            },
        );
    }
    terrain
}

/// Converts every baked tile under `<gameRoot>/Survival/ScrapMapAtlas`.
///
/// Conversion is incremental: a tile whose PNG is already newer than its baked
/// JSON is left alone, so this is cheap to call on every startup.
pub fn convert_baked_atlas(game_root: &Path, root: &Path) -> Result<AtlasBakeReportV1, String> {
    let baked_dir = game_root.join("Survival").join("ScrapMapAtlas");
    let output_dir = root.join("tiles").join(GENERATED_DIRECTORY);
    fs::create_dir_all(&output_dir).map_err(|error| format!("create output: {error}"))?;

    let listing = match fs::read_dir(&baked_dir) {
        Ok(listing) => listing,
        Err(_) => {
            return Ok(AtlasBakeReportV1 {
                converted: 0,
                skipped: 0,
                failed: 0,
                total_baked: 0,
            })
        }
    };

    let catalogue = asset_catalogue::load(game_root, root);
    let ground_cover = asset_catalogue::load_ground_cover(game_root);

    let mut report = AtlasBakeReportV1 {
        converted: 0,
        skipped: 0,
        failed: 0,
        total_baked: 0,
    };
    let mut generated = BTreeMap::new();

    for entry in listing.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_BAKED_TILE_BYTES {
            continue;
        }
        report.total_baked += 1;

        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let target = output_dir.join(format!("{stem}.png"));

        // Skip work whose output is already current.
        let baked_time = metadata.modified().ok();
        let target_time = fs::metadata(&target).ok().and_then(|m| m.modified().ok());
        if let (Some(baked), Some(existing)) = (baked_time, target_time) {
            if existing >= baked {
                report.skipped += 1;
                generated.insert(stem, None);
                continue;
            }
        }

        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                report.failed += 1;
                continue;
            }
        };
        match convert_one(&bytes, &catalogue, &ground_cover) {
            Ok((uuid, png, size)) => {
                let key = uuid.to_ascii_lowercase();
                let target = output_dir.join(format!("{key}.png"));
                if fs::write(&target, png).is_ok() {
                    generated.insert(key, Some(size));
                    report.converted += 1;
                } else {
                    report.failed += 1;
                }
            }
            Err(_) => report.failed += 1,
        }
    }

    if !generated.is_empty() {
        merge_manifest(root, &generated)?;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_u16(values: &[u16]) -> String {
        values.iter().map(|value| format!("{value:04X}")).collect()
    }

    #[test]
    fn decodes_fixed_width_hex_rasters() {
        assert_eq!(
            decode_hex_u16("0000FFFF8000", 3).unwrap(),
            vec![0x0000, 0xFFFF, 0x8000]
        );
        assert!(decode_hex_u16("0000", 2).is_err());
        assert!(decode_hex_u16("00ZZ", 1).is_err());
    }

    #[test]
    fn rgb565_endpoints_map_to_full_scale() {
        assert_eq!(rgb565_to_rgb888(0x0000), [0, 0, 0]);
        assert_eq!(rgb565_to_rgb888(0xFFFF), [255, 255, 255]);
    }

    #[test]
    fn height_encoding_round_trips_through_the_lua_bias() {
        // The Lua side writes round(h * 10) + 32768.
        for metres in [-40.0_f32, -0.5, 0.0, 12.3, 250.0] {
            let encoded = (metres * 10.0).round() as i32 + 32768;
            let decoded = decode_height(encoded as u16);
            assert!((decoded - metres).abs() < 0.05, "{metres} -> {decoded}");
        }
    }

    #[test]
    fn rendering_flips_rows_so_north_is_up() {
        // Two rows: south row grass, north row sand.
        let surface = vec![0_u8, 0, 1, 1];
        let rgba = render_tile_rgba(&surface, 2, &[], 0, &[], 0, GroundCoverLayer::default());
        // Image row 0 must be the north (sand) row.
        let sand = SURFACE_PALETTE[1];
        let grass = SURFACE_PALETTE[0];
        assert_eq!(rgba[0], sand[0] as u8);
        assert_eq!(rgba[8], grass[0] as u8);
    }

    #[test]
    fn surface_classes_pick_distinct_colours() {
        let surface = vec![0_u8, 1, 2, 3];
        let rgba = render_tile_rgba(&surface, 2, &[], 0, &[], 0, GroundCoverLayer::default());
        let pixels: Vec<[u8; 3]> = (0..4)
            .map(|i| [rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]])
            .collect();
        for (a, b) in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
            assert_ne!(pixels[a], pixels[b], "class {a} and {b} must differ");
        }
    }

    #[test]
    fn the_tint_modulates_the_surface_rather_than_replacing_it() {
        let surface = vec![0_u8; 4];
        // A white tint should leave the palette colour essentially untouched.
        let white = render_tile_rgba(&surface, 2, &[0xFFFF; 4], 2, &[], 0, GroundCoverLayer::default());
        let plain = render_tile_rgba(&surface, 2, &[], 0, &[], 0, GroundCoverLayer::default());
        assert_eq!(white[0], plain[0]);
        // A black tint darkens it towards zero.
        let black = render_tile_rgba(&surface, 2, &[0x0000; 4], 2, &[], 0, GroundCoverLayer::default());
        assert_eq!(black[0], 0);
    }

    #[test]
    fn terrain_below_the_waterline_reads_as_water() {
        let bed = vec![3_u8; 4]; // rock, as real lake beds sample
        let dry = render_tile_rgba(&bed, 2, &[], 0, &vec![0.0_f32; 4], 2, GroundCoverLayer::default());
        let shallow = render_tile_rgba(&bed, 2, &[], 0, &vec![-4.0_f32; 4], 2, GroundCoverLayer::default());
        let deep = render_tile_rgba(&bed, 2, &[], 0, &vec![-30.0_f32; 4], 2, GroundCoverLayer::default());

        let blueness = |px: &[u8]| i32::from(px[2]) - i32::from(px[1]);
        assert!(
            blueness(&shallow) > blueness(&dry),
            "a submerged lake bed should turn blue"
        );
        assert!(
            blueness(&deep) > blueness(&shallow),
            "deeper water should read darker and bluer"
        );
        // A few centimetres of sampling noise must not flood ordinary terrain.
        let noise = render_tile_rgba(&bed, 2, &[], 0, &vec![-0.4_f32; 4], 2, GroundCoverLayer::default());
        assert_eq!(noise, dry, "sub-threshold dips must not become water");
    }

    #[test]
    fn dirt_gouges_do_not_become_rivers() {
        // The crash-site trench cuts several metres down but is dirt, not a
        // lake bed. Rendering it as water made it read as a river.
        let trench = vec![2_u8; 4]; // dirt
        let dry = render_tile_rgba(&trench, 2, &[], 0, &vec![0.0_f32; 4], 2, GroundCoverLayer::default());
        let cut = render_tile_rgba(&trench, 2, &[], 0, &vec![-4.0_f32; 4], 2, GroundCoverLayer::default());
        assert_eq!(cut, dry, "a dirt gouge must stay dry regardless of depth");

        // Grass hollows likewise.
        let hollow = vec![0_u8; 4];
        assert_eq!(
            render_tile_rgba(&hollow, 2, &[], 0, &vec![-4.0_f32; 4], 2, GroundCoverLayer::default()),
            render_tile_rgba(&hollow, 2, &[], 0, &vec![0.0_f32; 4], 2, GroundCoverLayer::default()),
        );
    }

    fn asset_info(kind: AssetKind, color: [u8; 3], radius: f32) -> Option<AssetInfo> {
        Some(AssetInfo {
            kind,
            color,
            radius,
            footprint: Vec::new(),
            height: 0.0,
        })
    }

    #[test]
    fn asset_stream_decodes_palette_and_quarter_metre_positions() {
        // index 1, x = 0x008 quarter-metres = 2 m, y = 0x010 = 4 m.
        let placed = decode_assets_legacy("001008010").unwrap();
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].palette_index, 1);
        assert!((placed[0].x - 2.0).abs() < 0.001);
        assert!((placed[0].y - 4.0).abs() < 0.001);

        assert!(decode_assets_legacy("").unwrap().is_empty());
        assert!(decode_assets_legacy("0010080").is_err(), "ragged stream must fail");
    }

    #[test]
    fn assets_paint_over_the_terrain_at_their_position() {
        let span = 32;
        let mut rgba = vec![255_u8; span * span * 4];
        let palette = vec![asset_info(AssetKind::Foliage, [0, 0, 0], 4.0)];
        // Centre of the tile, one pixel per metre.
        let placed = [PlacedAsset {
            palette_index: 0,
            x: 16.0,
            y: 16.0,
            yaw: 0.0,
        }];
        draw_assets(&mut rgba, span, &placed, &palette, 1.0, &[], 0, 0.0);

        let at = |x: usize, y: usize| rgba[(y * span + x) * 4];
        assert!(at(16, 16) < 200, "the blob centre should be painted");
        assert_eq!(at(0, 0), 255, "distant terrain must be untouched");
    }

    #[test]
    fn water_and_road_assets_are_not_painted() {
        let span = 16;
        let mut rgba = vec![255_u8; span * span * 4];
        let palette = vec![asset_info(AssetKind::Skip, [0, 0, 0], 6.0)];
        let placed = [PlacedAsset {
            palette_index: 0,
            x: 8.0,
            y: 8.0,
            yaw: 0.0,
        }];
        draw_assets(&mut rgba, span, &placed, &palette, 1.0, &[], 0, 0.0);
        assert!(
            rgba.iter().all(|&value| value == 255),
            "skipped assets must leave the terrain alone"
        );
    }

    #[test]
    fn submerged_vegetation_is_dropped_but_ruins_survive() {
        let span = 16;
        let deep = vec![-20.0_f32; span * span];
        let placed = [PlacedAsset {
            palette_index: 0,
            x: 8.0,
            y: 8.0,
            yaw: 0.0,
        }];

        // Seaplants and lily pads sit in open water and read as floating trees.
        let mut foliage = vec![255_u8; span * span * 4];
        draw_assets(
            &mut foliage,
            span,
            &placed,
            &[asset_info(AssetKind::Foliage, [0, 0, 0], 4.0)],
            1.0,
            &deep,
            span,
            span as f32,
        );
        assert!(
            foliage.iter().all(|&value| value == 255),
            "submerged foliage must not be drawn"
        );

        // A ruin standing in a lake is a landmark and should still show.
        let mut ruin = vec![255_u8; span * span * 4];
        draw_assets(
            &mut ruin,
            span,
            &placed,
            &[asset_info(AssetKind::Building, [0, 0, 0], 4.0)],
            1.0,
            &deep,
            span,
            span as f32,
        );
        assert!(
            ruin[(8 * span + 8) * 4] < 200,
            "submerged structures should still be drawn"
        );
    }

    #[test]
    fn unknown_palette_entries_are_ignored_rather_than_panicking() {
        let span = 8;
        let mut rgba = vec![255_u8; span * span * 4];
        // Palette shorter than the highest index the stream references.
        let palette: Vec<Option<AssetInfo>> = vec![None];
        let placed = [PlacedAsset {
            palette_index: 7,
            x: 4.0,
            y: 4.0,
            yaw: 0.0,
        }];
        draw_assets(&mut rgba, span, &placed, &palette, 1.0, &[], 0, 0.0);
        assert!(rgba.iter().all(|&value| value == 255));
    }

    #[test]
    fn clutter_decodes_through_the_game_clutter_table() {
        let table = vec![GroundCover::Grass, GroundCover::Burnt, GroundCover::Stone];
        // Indices 0, 2, 1 and then 0xFF, which the baker writes for "nothing".
        let decoded = decode_clutter("000201FF", 2, &table).unwrap();
        assert_eq!(
            decoded,
            vec![
                GroundCover::Grass,
                GroundCover::Stone,
                GroundCover::Burnt,
                GroundCover::Bare
            ]
        );
        assert!(decode_clutter("", 0, &table).unwrap().is_empty());
        assert!(decode_clutter("0002", 2, &table).is_err(), "ragged raster");
    }

    #[test]
    fn ground_cover_separates_meadow_from_burnt_ground() {
        let surface = vec![0_u8; 16]; // all grass material
        let plain = render_tile_rgba(&surface, 4, &[], 0, &[], 0, GroundCoverLayer::default());
        let grass = vec![GroundCover::Grass; 16];
        let burnt = vec![GroundCover::Burnt; 16];

        let with_grass = render_tile_rgba(
            &surface,
            4,
            &[],
            0,
            &[],
            0,
            GroundCoverLayer {
                cover: &grass,
                span: 4,
            },
        );
        let with_burnt = render_tile_rgba(
            &surface,
            4,
            &[],
            0,
            &[],
            0,
            GroundCoverLayer {
                cover: &burnt,
                span: 4,
            },
        );

        assert_ne!(with_grass, plain, "grass cover should tint the ground");
        assert_ne!(with_burnt, with_grass, "burnt must not look like meadow");
        // Burnt ground is darker than the same material under grass.
        assert!(with_burnt[1] < with_grass[1]);
    }

    #[test]
    fn bare_ground_leaves_the_surface_alone() {
        let surface = vec![0_u8; 4];
        let bare = vec![GroundCover::Bare; 4];
        assert_eq!(
            render_tile_rgba(
                &surface,
                2,
                &[],
                0,
                &[],
                0,
                GroundCoverLayer {
                    cover: &bare,
                    span: 2
                },
            ),
            render_tile_rgba(&surface, 2, &[], 0, &[], 0, GroundCoverLayer::default()),
        );
    }

    #[test]
    fn flat_terrain_is_left_unshaded() {
        let surface = vec![0_u8; 4];
        let flat = vec![10.0_f32; 4];
        let shaded = render_tile_rgba(&surface, 2, &[], 0, &flat, 2, GroundCoverLayer::default());
        let plain = render_tile_rgba(&surface, 2, &[], 0, &[], 0, GroundCoverLayer::default());
        assert_eq!(shaded, plain, "flat ground must not pick up relief");
    }

    /// Brightness of the tile centre, away from the clamped border samples.
    fn centre_brightness(height: &[f32], span: usize) -> u8 {
        let surface = vec![0_u8; span * span];
        let rgba = render_tile_rgba(&surface, span, &[], 0, height, span, GroundCoverLayer::default());
        rgba[((span / 2) * span + span / 2) * 4]
    }

    #[test]
    fn opposing_slopes_shade_in_opposite_directions() {
        let span = 4;
        let flat = vec![0.0_f32; span * span];
        let mut rising = vec![0.0_f32; span * span];
        let mut falling = vec![0.0_f32; span * span];
        for y in 0..span {
            for x in 0..span {
                rising[y * span + x] = x as f32 * 4.0;
                falling[y * span + x] = (span - 1 - x) as f32 * 4.0;
            }
        }

        let flat_value = centre_brightness(&flat, span);
        let rising_value = centre_brightness(&rising, span);
        let falling_value = centre_brightness(&falling, span);

        // Light comes from the north-west, so a slope rising eastward faces away
        // from it and a slope rising westward faces into it.
        assert!(
            rising_value < flat_value,
            "east-facing rise should darken: {rising_value} vs {flat_value}"
        );
        assert!(
            falling_value > flat_value,
            "west-facing rise should lighten: {falling_value} vs {flat_value}"
        );
    }

    #[test]
    fn convert_one_round_trips_a_synthetic_tile() {
        let color = hex_u16(&[0xF800, 0x07E0, 0x001F, 0xFFFF]);
        let height = hex_u16(&[32768, 32768, 32768, 32768]);
        let document = json!({
            "schemaVersion": 2,
            "uuid": "AB-CD",
            "size": 1,
            "materialSpan": 2,
            "colorSpan": 2,
            "heightSpan": 2,
            "material": "0123",
            "color": color,
            "height": height,
        });
        let (uuid, png, size) = convert_one(&serde_json::to_vec(&document).unwrap(), &HashMap::new(), &[]).unwrap();
        assert_eq!(uuid, "AB-CD");
        assert_eq!(size, 1);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG");
    }

    fn scratch_dir(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        let path = env::temp_dir().join(format!("scrapmap-atlas-{label}-{stamp}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// Builds a tile shaped exactly like the Lua baker's output: a colour raster
    /// at `size * colour_res` per edge and a coarser height raster.
    fn baked_tile(uuid: &str, size: usize, material_res: usize, coarse_res: usize) -> Value {
        let material_span = size * material_res;
        let coarse_span = size * coarse_res;
        let material: String = (0..material_span * material_span)
            .map(|index| char::from_digit((index % 4) as u32, 16).unwrap())
            .collect();
        let color: Vec<u16> = vec![0xFFFF; coarse_span * coarse_span];
        let height: Vec<u16> = vec![32768_u16; coarse_span * coarse_span];
        // Lua writes every number as a double; mirror that here.
        json!({
            "schemaVersion": 2,
            "uuid": uuid,
            "size": size as f64,
            "cellSize": 64.0,
            "materialSpan": material_span as f64,
            "colorSpan": coarse_span as f64,
            "heightSpan": coarse_span as f64,
            "materialEncoding": "surface-class-hex",
            "encoding": "rgb565-hex",
            "material": material,
            "color": hex_u16(&color),
            "height": hex_u16(&height),
        })
    }

    #[test]
    fn an_unreadable_asset_stream_costs_the_objects_not_the_tile() {
        // A length that fits neither stride. Losing the whole tile over this
        // blanked four hundred cells on the map.
        assert!(decode_assets("ABCDEFG", 11).is_err());

        let game_root = scratch_dir("game-bad-assets");
        let atlas_root = scratch_dir("atlas-bad-assets");
        let baked_dir = game_root.join("Survival").join("ScrapMapAtlas");
        fs::create_dir_all(&baked_dir).unwrap();
        let mut tile = baked_tile("eeee-5555", 1, 64, 32);
        tile["assets"] = json!("ABCDEFG");
        tile["assetStride"] = json!(11.0);
        fs::write(
            baked_dir.join("eeee-5555.json"),
            serde_json::to_vec(&tile).unwrap(),
        )
        .unwrap();

        let report = convert_baked_atlas(&game_root, &atlas_root).unwrap();
        assert_eq!(report.converted, 1, "the terrain should still be drawn");
        assert_eq!(report.failed, 0);
        assert!(atlas_root
            .join("tiles")
            .join(GENERATED_DIRECTORY)
            .join("eeee-5555.png")
            .exists());

        fs::remove_dir_all(&game_root).ok();
        fs::remove_dir_all(&atlas_root).ok();
    }

    #[test]
    fn a_building_is_drawn_as_its_outline_and_turned_with_its_placement() {
        // An L, so a rotation is visible in the result rather than symmetric.
        let footprint = vec![[-4.0, -2.0], [4.0, -2.0], [4.0, 2.0], [-4.0, 2.0]];
        let info = AssetInfo {
            kind: AssetKind::Building,
            color: [255, 0, 0],
            radius: 4.0,
            footprint,
            height: 8.0,
        };
        let render = |yaw: f32| {
            let mut rgba = vec![0_u8; 64 * 64 * 4];
            draw_assets(
                &mut rgba,
                64,
                &[PlacedAsset { palette_index: 0, x: 32.0, y: 32.0, yaw }],
                &[Some(info.clone())],
                1.0,
                &[],
                0,
                64.0,
            );
            rgba
        };

        let flat = render(0.0);
        let turned = render(std::f32::consts::FRAC_PI_2);
        let painted = |rgba: &[u8]| rgba.chunks_exact(4).filter(|p| p[0] > 60).count();

        // Eight by four metres, so about 32 pixels either way round.
        assert!(painted(&flat) > 20, "nothing drawn: {}", painted(&flat));
        assert_eq!(
            painted(&flat),
            painted(&turned),
            "a quarter turn should paint the same area"
        );

        // Wide when flat, tall when turned -- which is the whole point of
        // carrying the placement's rotation across from the bake.
        let extent = |rgba: &[u8], horizontal: bool| {
            let (mut lo, mut hi) = (usize::MAX, 0);
            for y in 0..64 {
                for x in 0..64 {
                    if rgba[(y * 64 + x) * 4] > 60 {
                        let v = if horizontal { x } else { y };
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
            }
            hi.saturating_sub(lo)
        };
        assert!(extent(&flat, true) > extent(&flat, false), "flat should be wide");
        assert!(extent(&turned, false) > extent(&turned, true), "turned should be tall");
    }

    #[test]
    fn tile_terrain_reports_the_typical_ground_and_how_far_it_rises() {
        let game_root = scratch_dir("terrain-game");
        let baked_dir = game_root.join("Survival").join("ScrapMapAtlas");
        fs::create_dir_all(&baked_dir).unwrap();

        // A plateau at 20 m with a corner of the tile rising to 60 m. The plane
        // to frame against is the plateau, not the average and not the peak.
        let mut samples = vec![32768u16 + 200; 16]; // 20.0 m
        for sample in samples.iter_mut().take(2) {
            *sample = 32768 + 600; // 60.0 m
        }
        fs::write(
            baked_dir.join("hill.json"),
            serde_json::to_vec(&json!({
                "uuid": "HILL",
                "size": 1.0,
                "heightSpan": 4.0,
                "height": hex_u16(&samples),
            }))
            .unwrap(),
        )
        .unwrap();

        let cache_root = scratch_dir("terrain-cache");
        let terrain = tile_terrain(&game_root, &cache_root);
        // Looked up in lower case, because the layout and the bake disagree.
        let tile = terrain.get("hill").expect("the tile should be read");
        assert!((tile.ground - 20.0).abs() < 0.05, "ground was {}", tile.ground);
        assert!(tile.relief > 0.0, "a 40 m rise should register as relief");

        // A tile with no height raster is skipped rather than reported as flat
        // sea level, so the caller can tell the difference.
        fs::write(
            baked_dir.join("bare.json"),
            serde_json::to_vec(&json!({ "uuid": "BARE", "size": 1.0 })).unwrap(),
        )
        .unwrap();
        assert!(!tile_terrain(&game_root, &cache_root).contains_key("bare"));

        fs::remove_dir_all(&game_root).ok();
        fs::remove_dir_all(&cache_root).ok();
    }

    #[test]
    fn baked_tiles_become_pngs_and_are_published_through_the_manifest() {
        let game_root = scratch_dir("game");
        let atlas_root = scratch_dir("atlas");
        let baked_dir = game_root.join("Survival").join("ScrapMapAtlas");
        fs::create_dir_all(&baked_dir).unwrap();

        // A 1x1 tile and a 2x2 tile, at the resolutions the Lua baker uses.
        for (uuid, size) in [("aaaa-1111", 1_usize), ("bbbb-2222", 2)] {
            fs::write(
                baked_dir.join(format!("{uuid}.json")),
                serde_json::to_vec(&baked_tile(uuid, size, 64, 32)).unwrap(),
            )
            .unwrap();
        }

        let report = convert_baked_atlas(&game_root, &atlas_root).unwrap();
        assert_eq!(report.converted, 2, "both tiles should convert");
        assert_eq!(report.failed, 0);
        assert_eq!(report.total_baked, 2);

        let generated = atlas_root.join("tiles").join(GENERATED_DIRECTORY);
        for (uuid, size) in [("aaaa-1111", 1_u32), ("bbbb-2222", 2)] {
            let png = generated.join(format!("{uuid}.png"));
            let bytes = fs::read(&png).unwrap_or_else(|_| panic!("missing {uuid}.png"));
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            // PNG stores width in bytes 16..20, big endian.
            let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
            // 64 samples per cell, drawn at RENDER_SCALE pixels each.
            let expected = size * 64 * render_scale_for(size as usize * 64) as u32;
            assert_eq!(width, expected, "{uuid} is the wrong width");
        }

        let manifest: Value =
            serde_json::from_slice(&fs::read(atlas_root.join("manifest.json")).unwrap()).unwrap();
        let entries = manifest["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "unknown tiles must be appended");
        for entry in entries {
            // The renderer gates purely on this field, so it is what makes the
            // generated tiles visible.
            assert!(entry["topDownRelativePath"]
                .as_str()
                .unwrap()
                .starts_with("generated/"));
            assert_eq!(entry["topDownSourceKind"], "generated");
        }

        // A second pass must be a no-op rather than redoing the work.
        let again = convert_baked_atlas(&game_root, &atlas_root).unwrap();
        assert_eq!(again.converted, 0);
        assert_eq!(again.skipped, 2);

        fs::remove_dir_all(&game_root).ok();
        fs::remove_dir_all(&atlas_root).ok();
    }

    #[test]
    fn a_photograph_survives_the_next_atlas_refresh() {
        // The bake runs on every startup. It used to stamp `generated` over
        // every entry, so restarting the overlay silently threw away the
        // photographs of an entire sweep -- 115 taken, 37 still pointed at.
        let game_root = scratch_dir("game-photo-merge");
        let atlas_root = scratch_dir("atlas-photo-merge");
        let baked_dir = game_root.join("Survival").join("ScrapMapAtlas");
        fs::create_dir_all(&baked_dir).unwrap();
        fs::write(
            baked_dir.join("dddd-4444.json"),
            serde_json::to_vec(&baked_tile("dddd-4444", 1, 64, 32)).unwrap(),
        )
        .unwrap();

        convert_baked_atlas(&game_root, &atlas_root).unwrap();
        let read = || -> Value {
            serde_json::from_slice(&fs::read(atlas_root.join("manifest.json")).unwrap()).unwrap()
        };
        assert_eq!(read()["entries"][0]["topDownSourceKind"], "generated");

        // A photograph appears, exactly as the sweep would leave one.
        let photo_dir = atlas_root.join("tiles").join(PHOTO_DIRECTORY);
        fs::create_dir_all(&photo_dir).unwrap();
        fs::write(photo_dir.join("dddd-4444.png"), b"not really a png").unwrap();

        convert_baked_atlas(&game_root, &atlas_root).unwrap();
        let entry = read()["entries"][0].clone();
        assert_eq!(entry["topDownSourceKind"], "photo");
        assert_eq!(entry["topDownRelativePath"], "photo/dddd-4444.png");

        // And deleting it falls back rather than leaving a broken pointer.
        fs::remove_file(photo_dir.join("dddd-4444.png")).unwrap();
        convert_baked_atlas(&game_root, &atlas_root).unwrap();
        assert_eq!(read()["entries"][0]["topDownSourceKind"], "generated");

        fs::remove_dir_all(&game_root).ok();
        fs::remove_dir_all(&atlas_root).ok();
    }

    #[test]
    fn existing_manifest_entries_keep_their_identity() {
        let game_root = scratch_dir("game-merge");
        let atlas_root = scratch_dir("atlas-merge");
        let baked_dir = game_root.join("Survival").join("ScrapMapAtlas");
        fs::create_dir_all(&baked_dir).unwrap();
        fs::write(
            baked_dir.join("cccc-3333.json"),
            serde_json::to_vec(&baked_tile("cccc-3333", 1, 8, 4)).unwrap(),
        )
        .unwrap();
        fs::write(
            atlas_root.join("manifest.json"),
            serde_json::to_vec(&json!({
                "kind": "scrapmap.tile-atlas",
                "entries": [{
                    "tileUuid": "cccc-3333",
                    "relativePath": "meadow/cccc-3333.png",
                    "sha256": "keep-me",
                    "poiOverlayRelativePath": "topdown/poi/crashed_ship.jpg",
                    "poiOverlayOffsetX": -2,
                    "poiOverlayOffsetY": -2,
                    "poiOverlayTileSize": 4
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        convert_baked_atlas(&game_root, &atlas_root).unwrap();

        let manifest: Value =
            serde_json::from_slice(&fs::read(atlas_root.join("manifest.json")).unwrap()).unwrap();
        let entries = manifest["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "the tile must be updated, not duplicated");
        assert_eq!(entries[0]["sha256"], "keep-me");
        assert_eq!(entries[0]["relativePath"], "meadow/cccc-3333.png");
        assert_eq!(
            entries[0]["topDownRelativePath"], "generated/cccc-3333.png",
            "the generated image should take over rendering"
        );
        // The stale sm_overview POI photo must not stay layered on top.
        for key in [
            "poiOverlayRelativePath",
            "poiOverlayOffsetX",
            "poiOverlayOffsetY",
            "poiOverlayTileSize",
        ] {
            assert!(
                entries[0].get(key).is_none(),
                "{key} should be dropped for generated tiles"
            );
        }

        fs::remove_dir_all(&game_root).ok();
        fs::remove_dir_all(&atlas_root).ok();
    }

    #[test]
    fn lua_float_counts_are_accepted() {
        // This is exactly what sm.json.save writes: every number is a double.
        let document = json!({
            "uuid": "float-tile",
            "size": 2.0,
            "materialSpan": 2.0,
            "colorSpan": 2.0,
            "heightSpan": 0.0,
            "material": "0123",
            "color": hex_u16(&[0xF800, 0x07E0, 0x001F, 0xFFFF]),
            "height": "",
        });
        let (uuid, png, size) = convert_one(&serde_json::to_vec(&document).unwrap(), &HashMap::new(), &[])
            .expect("float-valued counts must parse");
        assert_eq!(uuid, "float-tile");
        assert_eq!(size, 2);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn a_tile_with_no_assets_still_converts() {
        // sm.json.save writes an empty Lua table as null, not [], and serde's
        // `default` only covers a missing field. Fourteen tiles failed on this.
        let document = json!({
            "uuid": "empty-assets",
            "size": 1.0,
            "materialSpan": 2.0,
            "colorSpan": 0.0,
            "heightSpan": 0.0,
            "material": "0123",
            "color": "",
            "height": "",
            "assetPalette": Value::Null,
            "assets": "",
        });
        let (uuid, png, _) = convert_one(&serde_json::to_vec(&document).unwrap(), &HashMap::new(), &[])
            .expect("a null asset palette must not fail the tile");
        assert_eq!(uuid, "empty-assets");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn convert_one_rejects_a_truncated_raster() {
        let document = json!({
            "uuid": "x",
            "materialSpan": 4,
            "colorSpan": 0,
            "heightSpan": 0,
            "material": "01",
            "color": "",
            "height": "",
        });
        assert!(convert_one(&serde_json::to_vec(&document).unwrap(), &HashMap::new(), &[]).is_err());
    }
}
