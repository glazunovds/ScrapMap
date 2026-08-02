//! Describes the placed objects the terrain baker records.
//!
//! `sm.terrainTile.getAssetsForCell` yields a uuid and a position, which is not
//! enough to draw anything meaningful. The game ships an asset database under
//! `Terrain/Database/AssetSets` giving each uuid a name and its default colours,
//! and most assets carry a collision mesh we can measure for a footprint. This
//! module turns that into what the renderer needs: a colour and a radius.
//!
//! The catalogue depends only on the installed game, so it is cached next to the
//! tile atlas and rebuilt when the game changes.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const CATALOGUE_FILE: &str = "asset-catalogue.json";
const CATALOGUE_VERSION: u32 = 2;
/// Collision meshes are metres; nothing legitimate is wider than a tile.
const MAX_RADIUS: f32 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssetKind {
    Foliage,
    Rock,
    Building,
    Wreck,
    Debris,
    /// Water planes and road decals: already represented by the terrain itself.
    Skip,
    Other,
}

impl AssetKind {
    /// Fallback footprint when no collision mesh is available, in metres.
    fn default_radius(self) -> f32 {
        match self {
            AssetKind::Foliage => 3.0,
            AssetKind::Rock => 2.5,
            AssetKind::Building => 6.0,
            AssetKind::Wreck => 8.0,
            AssetKind::Debris => 1.5,
            AssetKind::Skip | AssetKind::Other => 2.0,
        }
    }

    fn default_color(self) -> [u8; 3] {
        match self {
            AssetKind::Foliage => [46, 92, 42],
            AssetKind::Rock => [118, 116, 112],
            AssetKind::Building => [126, 112, 96],
            AssetKind::Wreck => [150, 68, 54],
            AssetKind::Debris => [104, 96, 84],
            AssetKind::Skip | AssetKind::Other => [110, 106, 96],
        }
    }

    /// How strongly the object is drawn over the terrain.
    pub fn opacity(self) -> f32 {
        match self {
            AssetKind::Foliage => 0.72,
            AssetKind::Rock => 0.80,
            AssetKind::Building | AssetKind::Wreck => 0.92,
            AssetKind::Debris => 0.55,
            AssetKind::Skip | AssetKind::Other => 0.6,
        }
    }
}

/// Classifies by the asset set the entry came from, refined by its name. The
/// game's own `Type/...` categories cover barely half the sets -- Forest,
/// Building and Spaceship all lack one -- so the file name is the better signal.
fn classify(set_name: &str, asset_name: &str) -> AssetKind {
    let set = set_name.to_ascii_lowercase();
    let name = asset_name.to_ascii_lowercase();

    if set.contains("water") || set.contains("waterfall") || set.contains("distanceplane") {
        return AssetKind::Skip;
    }
    if set.contains("lightcone") || set.contains("collider") || set.contains("blockout") {
        return AssetKind::Skip;
    }
    // Road assets are kerbs and markings; the road surface is already in the
    // terrain material, so drawing them again only muddies it.
    if set.contains("road") || set.contains("racetrack") {
        return AssetKind::Skip;
    }
    if set.contains("spaceship") || set.contains("bosstrain") {
        return AssetKind::Wreck;
    }
    if set.contains("foliage") || set.contains("forest") || name.contains("tree")
        || name.contains("bush") || name.contains("plant") || name.contains("fern")
    {
        return AssetKind::Foliage;
    }
    if set.contains("rock") || set.contains("stone") || set.contains("iceformation")
        || name.contains("rock") || name.contains("cliff") || name.contains("boulder")
    {
        return AssetKind::Rock;
    }
    if set.contains("garbage") || set.contains("rubble") || set.contains("trash")
        || name.contains("debris") || name.contains("scrap")
    {
        return AssetKind::Debris;
    }
    if set.contains("building") || set.contains("warehouse") || set.contains("ruin")
        || set.contains("hideout") || set.contains("station") || set.contains("factory")
        || set.contains("structure") || set.contains("garage") || set.contains("minehub")
        || set.contains("dungeon") || set.contains("department") || set.contains("camping")
        || set.contains("excavation") || set.contains("mechanical") || set.contains("manmade")
    {
        return AssetKind::Building;
    }
    AssetKind::Other
}

/// Parses the game's JSON, which is really JSONC.
///
/// `farming.harvestableset` annotates its entries with `//` comments, and the
/// game's own parser accepts them. Strict parsing silently dropped the whole
/// file, so comments are stripped first -- outside string literals, so a `//`
/// inside a path or value survives.
fn parse_game_json(bytes: &[u8]) -> Option<Value> {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        return Some(value);
    }

    let text = std::str::from_utf8(bytes).ok()?;
    let mut cleaned = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(character) = characters.next() {
        if in_string {
            cleaned.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
            cleaned.push(character);
            continue;
        }
        if character == '/' && characters.peek() == Some(&'/') {
            for skipped in characters.by_ref() {
                if skipped == '\n' {
                    cleaned.push('\n');
                    break;
                }
            }
            continue;
        }
        cleaned.push(character);
    }

    serde_json::from_str(&cleaned).ok()
}

fn parse_hex_color(text: &str) -> Option<[u8; 3]> {
    let clean = text.trim().trim_start_matches('#');
    if clean.len() < 6 {
        return None;
    }
    let channel = |index: usize| u8::from_str_radix(&clean[index..index + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// Picks the colour that best represents the object seen from above.
fn representative_color(colors: &Value) -> Option<[u8; 3]> {
    let object = colors.as_object()?;
    for key in ["leaves", "foliage", "grass", "wood", "trunk", "metal", "dirt"] {
        if let Some(value) = object.get(key).and_then(Value::as_str) {
            if let Some(color) = parse_hex_color(value) {
                return Some(color);
            }
        }
    }
    object
        .values()
        .filter_map(Value::as_str)
        .find_map(parse_hex_color)
}

/// Collision meshes are not in metres. Calibrated against objects of known
/// scale -- a warehouse lands near 28 m across and a giant tree near 16 m --
/// the vertices read as decimetres. This is a plausibility fit rather than a
/// documented unit, so it is the first thing to adjust if objects render
/// consistently too large or too small.
const MESH_UNITS_PER_METRE: f32 = 10.0;

/// Measures a collision mesh's horizontal half-extent.
///
/// Uses the bounding box rather than distance from the origin: asset meshes are
/// not centred on their pivot, so an origin radius overstates anything modelled
/// off to one side. Not every `.obj` here is text -- some are compiled -- so a
/// line that will not parse is skipped rather than failing the whole mesh.
fn radius_from_obj(path: &Path) -> Option<f32> {
    let text = fs::read_to_string(path).ok()?;
    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    let mut seen = 0_u32;

    for line in text.lines() {
        let mut parts = line.split_ascii_whitespace();
        if parts.next() != Some("v") {
            continue;
        }
        let (Some(x), Some(y)) = (
            parts.next().and_then(|value| value.parse::<f32>().ok()),
            parts.next().and_then(|value| value.parse::<f32>().ok()),
        ) else {
            continue;
        };
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
        seen += 1;
    }

    if seen < 3 {
        return None;
    }
    let radius = ((max_x - min_x) / 2.0).max((max_y - min_y) / 2.0) / MESH_UNITS_PER_METRE;
    (radius > 0.0).then(|| radius.min(MAX_RADIUS))
}

/// Resolves the game's `$SURVIVAL_DATA` / `$GAME_DATA` path variables.
fn resolve_game_path(game_root: &Path, raw: &str) -> Option<PathBuf> {
    let (variable, rest) = raw.split_once('/')?;
    let base = match variable {
        "$SURVIVAL_DATA" => game_root.join("Survival"),
        "$GAME_DATA" => game_root.join("Data"),
        "$CHALLENGE_DATA" => game_root.join("ChallengeData"),
        _ => return None,
    };
    Some(base.join(rest.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct AssetInfo {
    pub kind: AssetKind,
    pub color: [u8; 3],
    pub radius: f32,
}

#[derive(Serialize, Deserialize)]
struct CataloguePayload {
    version: u32,
    assets: HashMap<String, AssetInfo>,
}

fn asset_set_directories(game_root: &Path) -> Vec<PathBuf> {
    ["Survival", "Data"]
        .into_iter()
        .map(|part| {
            game_root
                .join(part)
                .join("Terrain")
                .join("Database")
                .join("AssetSets")
        })
        .filter(|path| path.is_dir())
        .collect()
}

fn harvestable_set_directories(game_root: &Path) -> Vec<PathBuf> {
    ["Survival", "Data"]
        .into_iter()
        .map(|part| {
            game_root
                .join(part)
                .join("Harvestables")
                .join("Database")
                .join("HarvestableSets")
        })
        .filter(|path| path.is_dir())
        .collect()
}

/// Harvestable sets carry no collision mesh, so size comes from the name. A
/// forest is mostly full-grown trees, and the map only needs them to read as
/// canopy rather than to be individually measurable.
fn harvestable_radius(kind: AssetKind, name: &str) -> f32 {
    let item = name.to_ascii_lowercase();
    if item.contains("sapling") || item.contains("small") || item.contains("young") {
        return 1.4;
    }
    if item.contains("large") || item.contains("big") {
        return 4.0;
    }
    match kind {
        AssetKind::Foliage => 2.6,
        AssetKind::Rock => 1.8,
        _ => kind.default_radius(),
    }
}

/// Harvestables are the ordinary forest and boulders: trees, stones, ore.
/// Their sets are named plainly, so the file name classifies them.
fn classify_harvestable(set_name: &str, name: &str) -> AssetKind {
    let set = set_name.to_ascii_lowercase();
    let item = name.to_ascii_lowercase();
    if set.contains("tree") || set.contains("forest") || set.contains("farm")
        || set.contains("plantable") || set.contains("potato") || item.contains("tree")
    {
        return AssetKind::Foliage;
    }
    if set.contains("stone") || set.contains("ore") || set.contains("crystal")
        || set.contains("mineral") || set.contains("amber") || set.contains("sparkstone")
    {
        return AssetKind::Rock;
    }
    if set.contains("dungeon") || set.contains("station") {
        return AssetKind::Building;
    }
    // Loot crates, fences, traps and remains: small scatter, drawn faintly.
    AssetKind::Debris
}

/// The first entry of a harvestable's colour list stands in for the whole
/// object; the rest are per-instance variants of the same foliage.
fn harvestable_color(value: &Value) -> Option<[u8; 3]> {
    match value {
        Value::Array(items) => items.iter().filter_map(Value::as_str).find_map(parse_hex_color),
        Value::String(text) => parse_hex_color(text),
        _ => None,
    }
}

fn build(game_root: &Path) -> HashMap<String, AssetInfo> {
    let mut catalogue = HashMap::new();

    for directory in harvestable_set_directories(game_root) {
        let Ok(listing) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in listing.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("harvestableset") {
                continue;
            }
            let set_name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_owned();
            let Ok(bytes) = fs::read(&path) else { continue };
            let Some(document) = parse_game_json(&bytes) else {
                continue;
            };
            let Some(items) = document.get("harvestableList").and_then(Value::as_array) else {
                continue;
            };

            for item in items {
                let Some(uuid) = item.get("uuid").and_then(Value::as_str) else {
                    continue;
                };
                let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                let kind = classify_harvestable(&set_name, name);
                let color = item
                    .get("color")
                    .and_then(harvestable_color)
                    .unwrap_or_else(|| kind.default_color());
                catalogue.insert(
                    uuid.to_ascii_lowercase(),
                    AssetInfo {
                        kind,
                        color,
                        radius: harvestable_radius(kind, name),
                    },
                );
            }
        }
    }

    for directory in asset_set_directories(game_root) {
        let Ok(listing) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in listing.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("assetset") {
                continue;
            }
            let set_name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_owned();
            let Ok(bytes) = fs::read(&path) else { continue };
            let Some(document) = parse_game_json(&bytes) else {
                continue;
            };
            let Some(assets) = document.get("assetListRenderable").and_then(Value::as_array) else {
                continue;
            };

            for asset in assets {
                let Some(uuid) = asset.get("uuid").and_then(Value::as_str) else {
                    continue;
                };
                let name = asset.get("name").and_then(Value::as_str).unwrap_or_default();
                let kind = classify(&set_name, name);
                let color = asset
                    .get("defaultColors")
                    .and_then(representative_color)
                    .unwrap_or_else(|| kind.default_color());
                let radius = asset
                    .get("col")
                    .and_then(Value::as_str)
                    .and_then(|raw| resolve_game_path(game_root, raw))
                    .and_then(|path| radius_from_obj(&path))
                    .unwrap_or_else(|| kind.default_radius());

                catalogue.insert(
                    uuid.to_ascii_lowercase(),
                    AssetInfo {
                        kind,
                        color,
                        radius,
                    },
                );
            }
        }
    }

    catalogue
}

/// Ground cover, as a tint applied over the surface material.
///
/// Clutter is what separates a mown meadow from a forest floor from burnt
/// stubble: the material sampler reports only grass/sand/dirt/rock, so without
/// this every vegetated tile is the same flat green.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroundCover {
    /// No clutter at all: bare ground, roads, building interiors.
    Bare,
    Grass,
    Weed,
    Burnt,
    Scrap,
    Stone,
    /// Underwater growth; the water layer already covers it.
    Submerged,
}

impl GroundCover {
    /// Tint and strength. Deliberately gentle: this modulates the terrain
    /// rather than replacing it, so roads and shorelines still read through.
    pub fn tint(self) -> Option<([f32; 3], f32)> {
        match self {
            GroundCover::Bare | GroundCover::Submerged => None,
            GroundCover::Grass => Some(([74, 122, 52].map(|v| v as f32), 0.34)),
            GroundCover::Weed => Some(([96, 108, 58].map(|v| v as f32), 0.26)),
            GroundCover::Burnt => Some(([64, 56, 48].map(|v| v as f32), 0.40)),
            GroundCover::Scrap => Some(([112, 96, 78].map(|v| v as f32), 0.30)),
            GroundCover::Stone => Some(([124, 122, 116].map(|v| v as f32), 0.24)),
        }
    }
}

fn classify_clutter(name: &str) -> GroundCover {
    let item = name.to_ascii_lowercase();
    if item.contains("burnt") {
        return GroundCover::Burnt;
    }
    if item.contains("seaweed") || item.contains("coral") || item.contains("marimo") {
        return GroundCover::Submerged;
    }
    if item.contains("scrap") || item.contains("junk") || item.contains("road") {
        return GroundCover::Scrap;
    }
    // fluff_short and fluff_tall are the grass tufts; there is no
    // "env_clutter_grass" despite what the name would suggest.
    if item.contains("fluff") || item.contains("spore") || item.contains("moss") {
        return GroundCover::Grass;
    }
    if item.contains("weed") || item.contains("root") {
        return GroundCover::Weed;
    }
    if item.contains("rock") || item.contains("pebble") || item.contains("stone")
        || item.contains("mineral") || item.contains("crystal") || item.contains("ore")
        || item.contains("stalactite") || item.contains("pyramond")
    {
        return GroundCover::Stone;
    }
    GroundCover::Bare
}

/// Reads `clutter.json`, whose array position is the index the terrain API
/// returns from `getClutterIdxAt`.
pub fn load_ground_cover(game_root: &Path) -> Vec<GroundCover> {
    for part in ["Survival", "Data"] {
        let path = game_root
            .join(part)
            .join("Terrain")
            .join("Database")
            .join("clutter.json");
        let Ok(bytes) = fs::read(&path) else { continue };
        let Some(document) = parse_game_json(&bytes) else {
            continue;
        };
        let Some(list) = document.get("clutterList").and_then(Value::as_array) else {
            continue;
        };
        return list
            .iter()
            .map(|entry| {
                classify_clutter(entry.get("name").and_then(Value::as_str).unwrap_or_default())
            })
            .collect();
    }
    Vec::new()
}

/// Loads the catalogue, building and caching it if the cache is absent or stale.
pub fn load(game_root: &Path, cache_root: &Path) -> HashMap<String, AssetInfo> {
    let cache_path = cache_root.join(CATALOGUE_FILE);
    if let Ok(bytes) = fs::read(&cache_path) {
        if let Ok(payload) = serde_json::from_slice::<CataloguePayload>(&bytes) {
            if payload.version == CATALOGUE_VERSION && !payload.assets.is_empty() {
                return payload.assets;
            }
        }
    }

    let assets = build(game_root);
    if !assets.is_empty() {
        let payload = CataloguePayload {
            version: CATALOGUE_VERSION,
            assets: assets.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&payload) {
            let _ = fs::create_dir_all(cache_root);
            let _ = fs::write(&cache_path, bytes);
        }
    }
    assets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_from_the_shipped_hex_form() {
        assert_eq!(parse_hex_color("73791d"), Some([0x73, 0x79, 0x1D]));
        assert_eq!(parse_hex_color("#017a13"), Some([0x01, 0x7A, 0x13]));
        assert_eq!(parse_hex_color("abc"), None);
    }

    #[test]
    fn foliage_colour_prefers_leaves_over_soil() {
        let colors = serde_json::json!({ "dirt": "746237", "leaves": "017a13" });
        assert_eq!(representative_color(&colors), Some([0x01, 0x7A, 0x13]));
    }

    #[test]
    fn sets_without_a_type_category_still_classify() {
        // Forest, Building and Spaceship carry no Type/ category in the game's
        // own database, which is why classification keys off the set name.
        assert_eq!(classify("Forest", "env_nature_gianttree_trunk"), AssetKind::Foliage);
        assert_eq!(classify("Spaceship", "env_ship_hull_01"), AssetKind::Wreck);
        assert_eq!(classify("Building", "env_warehouse_wall"), AssetKind::Building);
        assert_eq!(classify("Rocks", "env_rock_large_02"), AssetKind::Rock);
        assert_eq!(classify("Garbage", "env_scrap_pile"), AssetKind::Debris);
    }

    #[test]
    fn terrain_duplicating_assets_are_skipped() {
        // Water and road surfaces already come from the terrain sampler.
        assert_eq!(classify("Water", "env_water_plane"), AssetKind::Skip);
        assert_eq!(classify("Road", "env_road_kerb_01"), AssetKind::Skip);
        assert_eq!(classify("Distanceplane", "backdrop"), AssetKind::Skip);
    }

    #[test]
    fn radius_comes_from_the_widest_vertex() {
        let directory = std::env::temp_dir().join(format!(
            "scrapmap-obj-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("mesh.obj");
        // X spans -20..20 and Y spans 0..20, so the half-extent is 20 units,
        // which is 2 m. Height must not count, and the unparsable line from a
        // compiled mesh must be skipped rather than aborting the file.
        fs::write(
            &path,
            "v -20.0 0.0 0.0\nv 20.0 20.0 900.0\nv 0.0 10.0 5.0\nv ytsaq garbage\nf 1 2 3\n",
        )
        .unwrap();
        let radius = radius_from_obj(&path).unwrap();
        assert!((radius - 2.0).abs() < 0.001, "got {radius}");
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn meshes_offset_from_their_pivot_are_not_overstated() {
        let directory = std::env::temp_dir().join("scrapmap-obj-offset");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("offset.obj");
        // A 20-unit-wide object modelled 500 units away from its origin: the
        // bounding box is what matters, not the distance from the pivot.
        fs::write(
            &path,
            "v 500.0 0.0 0.0\nv 510.0 10.0 0.0\nv 505.0 5.0 1.0\n",
        )
        .unwrap();
        let radius = radius_from_obj(&path).unwrap();
        assert!((radius - 0.5).abs() < 0.001, "got {radius}");
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn harvestable_sets_classify_from_their_file_name() {
        // Ordinary forest is harvestables, not assets: Forest.assetset holds
        // only giant-tree parts.
        assert_eq!(
            classify_harvestable("trees", "harvestable_tree_leafy01"),
            AssetKind::Foliage
        );
        assert_eq!(
            classify_harvestable("burntforest", "harvestable_tree_burnt02"),
            AssetKind::Foliage
        );
        assert_eq!(
            classify_harvestable("stones", "harvestable_stone_large01"),
            AssetKind::Rock
        );
        assert_eq!(classify_harvestable("ore", "harvestable_ore_iron"), AssetKind::Rock);
        assert_eq!(
            classify_harvestable("loot", "harvestable_crate01"),
            AssetKind::Debris
        );
    }

    #[test]
    fn harvestable_colour_takes_the_first_of_the_variant_list() {
        // The shipped form is a list of RGBA variants for the same plant.
        let colors = serde_json::json!(["54c51eff", "68cf37ff", "9ebe2bff"]);
        assert_eq!(harvestable_color(&colors), Some([0x54, 0xC5, 0x1E]));
        assert_eq!(
            harvestable_color(&serde_json::json!("db9819ff")),
            Some([0xDB, 0x98, 0x19])
        );
        assert_eq!(harvestable_color(&serde_json::json!(7)), None);
    }

    #[test]
    fn saplings_are_drawn_smaller_than_grown_trees() {
        let sapling = harvestable_radius(AssetKind::Foliage, "harvestable_tree_sapling01");
        let grown = harvestable_radius(AssetKind::Foliage, "harvestable_tree_leafy01");
        let large = harvestable_radius(AssetKind::Foliage, "harvestable_tree_large01");
        assert!(sapling < grown, "{sapling} !< {grown}");
        assert!(grown < large, "{grown} !< {large}");
    }

    #[test]
    fn game_json_with_comments_parses() {
        // farming.harvestableset ships exactly this shape; strict parsing
        // dropped the whole file and its entries silently vanished.
        let jsonc = br#"{
            "harvestableList": [
                {
                    "uuid": "abc", // blueberry
                    "name": "hvs_farming_blueberry" //the bush
                }
            ]
        }"#;
        let value = parse_game_json(jsonc).expect("comments must be tolerated");
        assert_eq!(value["harvestableList"][0]["uuid"], "abc");
    }

    #[test]
    fn a_double_slash_inside_a_string_is_not_a_comment() {
        let jsonc = br#"{ "path": "$SURVIVAL_DATA//Terrain/x.obj", "n": 1 }"#;
        let value = parse_game_json(jsonc).expect("valid json");
        assert_eq!(value["path"], "$SURVIVAL_DATA//Terrain/x.obj");
        assert_eq!(value["n"], 1);
    }

    #[test]
    fn irrecoverable_json_still_returns_none() {
        assert!(parse_game_json(b"{ not json at all ").is_none());
    }

    #[test]
    fn game_path_variables_resolve() {
        let root = Path::new("C:/game");
        assert_eq!(
            resolve_game_path(root, "$SURVIVAL_DATA/Terrain/Collision/a.obj"),
            Some(root.join("Survival").join("Terrain").join("Collision").join("a.obj"))
        );
        assert!(resolve_game_path(root, "$UNKNOWN/x.obj").is_none());
    }
}
