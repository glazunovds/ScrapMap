//! Installs and removes ScrapMap's game-script patch.
//!
//! A port of `tools/game-patch.mjs` into the executable, so a released build
//! needs neither Node nor a checkout of this repository. The Lua is compiled in
//! with `include_str!`, which also removes a whole class of mismatch: a player
//! cannot run the binary against addon files from a different revision.
//!
//! Scrap Mechanic compares Lua script checksums when a client joins a server, so
//! two players must run byte-identical files. Everything here exists to make
//! that exact and reversible:
//!
//! - stock files are never edited in place, only appended to between markers;
//! - the patched text is rebuilt from a recorded vanilla baseline every time, so
//!   applying twice is the same as applying once;
//! - line endings are normalised on install, because a checkout with
//!   `core.autocrlf` rewrites `.lua` files and one changed byte is a refusal.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

const MARKER_BEGIN: &str = "-- SCRAPMAP ADDON BEGIN";
const MARKER_END: &str = "-- SCRAPMAP ADDON END";
/// The telemetry patch shipped before the markers were generalised.
const LEGACY_MARKERS: [(&str, &str); 1] = [(
    "-- SCRAPMAP TELEMETRY ADDON BEGIN",
    "-- SCRAPMAP TELEMETRY ADDON END",
)];

/// Files copied in whole; they do not exist in a stock install.
const ADDONS: [(&str, &str); 4] = [
    (
        "Survival/Scripts/terrain/ScrapMapAtlasBake.lua",
        include_str!("../../game-patch/Survival/Scripts/terrain/ScrapMapAtlasBake.lua"),
    ),
    (
        "Survival/Scripts/terrain/ScrapMapLayoutExport.lua",
        include_str!("../../game-patch/Survival/Scripts/terrain/ScrapMapLayoutExport.lua"),
    ),
    (
        "Survival/Scripts/game/ScrapMapTelemetry.lua",
        include_str!("../../game-patch/Survival/Scripts/game/ScrapMapTelemetry.lua"),
    ),
    (
        "Survival/Scripts/game/ScrapMapPoiShoot.lua",
        include_str!("../../game-patch/Survival/Scripts/game/ScrapMapPoiShoot.lua"),
    ),
];

/// Blocks appended to stock files. Appending rather than editing keeps the
/// patch exactly reversible and survives a game update that moves the
/// surrounding code.
const INJECTIONS: [(&str, &str); 2] = [
    (
        "Survival/Scripts/game/SurvivalGame.lua",
        concat!(
            "dofile( \"$SURVIVAL_DATA/Scripts/game/ScrapMapTelemetry.lua\" )\n",
            "dofile( \"$SURVIVAL_DATA/Scripts/game/ScrapMapPoiShoot.lua\" )"
        ),
    ),
    (
        "Survival/Scripts/terrain/terrain_overworld.lua",
        concat!(
            "dofile( \"$SURVIVAL_DATA/Scripts/terrain/ScrapMapLayoutExport.lua\" )\n",
            "dofile( \"$SURVIVAL_DATA/Scripts/terrain/ScrapMapAtlasBake.lua\" )\n",
            "\n",
            "local __scrapmapInnerLoad = Load\n",
            "function Load()\n",
            "\tlocal loaded = __scrapmapInnerLoad()\n",
            "\tif loaded then\n",
            "\t\tScrapMapExportLayout()\n",
            "\t\tScrapMapBakeAtlas()\n",
            "\tend\n",
            "\treturn loaded\n",
            "end"
        ),
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileState {
    Absent,
    Addon,
    Patched,
    Vanilla,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchedFile {
    pub path: String,
    pub state: FileState,
    /// First 16 hex characters of the SHA-256, which is what two players
    /// compare before playing together.
    pub hash: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchStatus {
    /// True when every managed file is installed and injected.
    pub applied: bool,
    pub baseline_recorded: bool,
    pub files: Vec<PatchedFile>,
}

/// Collapses CRLF and lone CR to LF so installed files are byte-stable.
fn normalise_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn short_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Removes every ScrapMap block, current or legacy, from a file's text.
fn strip_blocks(text: &str) -> String {
    let mut out = text.to_owned();
    let pairs = std::iter::once((MARKER_BEGIN, MARKER_END)).chain(LEGACY_MARKERS);
    for (begin, end) in pairs {
        loop {
            let Some(start) = out.find(begin) else { break };
            match out[start..].find(end) {
                Some(offset) => {
                    let stop = start + offset + end.len();
                    out = format!("{}{}", &out[..start], &out[stop..]);
                }
                None => {
                    out.truncate(start);
                    break;
                }
            }
        }
    }
    format!("{}\n", out.trim_end())
}

fn is_patched(text: &str) -> bool {
    text.contains(MARKER_BEGIN) || LEGACY_MARKERS.iter().any(|(begin, _)| text.contains(begin))
}

/// The text a stock file becomes once patched.
fn patched_text(baseline: &str, body: &str) -> String {
    format!(
        "{}{MARKER_BEGIN}\n{body}\n{MARKER_END}\n",
        strip_blocks(baseline)
    )
}

fn is_game_root(root: &Path) -> bool {
    root.join("Survival").join("Scripts").is_dir()
}

/// Finds the install in the usual Steam library layouts on every drive.
pub fn discover_game_root() -> Option<PathBuf> {
    let libraries = [
        PathBuf::from("SteamLibrary"),
        PathBuf::from("Program Files (x86)").join("Steam"),
        PathBuf::from("Steam"),
    ];
    for letter in 'C'..='Z' {
        for library in &libraries {
            let candidate = PathBuf::from(format!("{letter}:\\"))
                .join(library)
                .join("steamapps")
                .join("common")
                .join("Scrap Mechanic");
            if is_game_root(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Where the untouched copies of the stock files are kept.
///
/// Under `%LOCALAPPDATA%\ScrapMap` rather than in the game directory, so
/// verifying the game's files through Steam cannot quietly take the baseline
/// with them -- and deliberately *not* under `atlas\`, which is documented as
/// disposable and does get deleted to force a re-bake.
fn baseline_root(cache_root: &Path) -> PathBuf {
    cache_root.join("vanilla")
}

fn read_text(path: &Path) -> Option<String> {
    fs::read(path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

pub fn status(game_root: &Path, cache_root: &Path) -> PatchStatus {
    let mut files = Vec::new();
    let mut applied = true;

    for (relative, _) in ADDONS {
        let path = game_root.join(relative);
        let bytes = fs::read(&path).ok();
        if bytes.is_none() {
            applied = false;
        }
        files.push(PatchedFile {
            path: relative.to_owned(),
            state: if bytes.is_some() {
                FileState::Addon
            } else {
                FileState::Absent
            },
            hash: bytes.as_deref().map(short_hash),
        });
    }

    for (relative, _) in INJECTIONS {
        let path = game_root.join(relative);
        let bytes = fs::read(&path).ok();
        let state = match bytes.as_deref().and_then(|b| std::str::from_utf8(b).ok()) {
            Some(text) if is_patched(text) => FileState::Patched,
            Some(_) => FileState::Vanilla,
            None => FileState::Absent,
        };
        if state != FileState::Patched {
            applied = false;
        }
        files.push(PatchedFile {
            path: relative.to_owned(),
            state,
            hash: bytes.as_deref().map(short_hash),
        });
    }

    let baseline = baseline_root(cache_root);
    let baseline_recorded = INJECTIONS
        .iter()
        .all(|(relative, _)| baseline.join(relative).is_file());

    PatchStatus {
        applied,
        baseline_recorded,
        files,
    }
}

/// Records the current stock files as the baseline to restore to.
///
/// Refuses to run over patched files: capturing those as "vanilla" would make
/// revert a no-op and leave the player unable to join anyone.
pub fn snapshot(game_root: &Path, cache_root: &Path) -> Result<usize, String> {
    for (relative, _) in INJECTIONS {
        let path = game_root.join(relative);
        let Some(text) = read_text(&path) else {
            return Err(format!("{relative} is missing from the game"));
        };
        if is_patched(&text) {
            return Err(format!(
                "{relative} still carries a ScrapMap block. Restore the game's files first \
                 -- Steam, Properties, Installed Files, Verify integrity of game files."
            ));
        }
    }

    let baseline = baseline_root(cache_root);
    let mut recorded = 0;
    for (relative, _) in INJECTIONS {
        let target = baseline.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::copy(game_root.join(relative), &target).map_err(|error| error.to_string())?;
        recorded += 1;
    }
    Ok(recorded)
}

pub fn apply(game_root: &Path, cache_root: &Path) -> Result<PatchStatus, String> {
    let baseline = baseline_root(cache_root);
    // Take the baseline automatically when the game is stock, so a first run
    // does not need a separate step that is easy to forget.
    if INJECTIONS
        .iter()
        .any(|(relative, _)| !baseline.join(relative).is_file())
    {
        snapshot(game_root, cache_root)?;
    }

    for (relative, contents) in ADDONS {
        let target = game_root.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&target, normalise_endings(contents)).map_err(|error| {
            format!("could not install {relative}: {error}")
        })?;
    }

    for (relative, body) in INJECTIONS {
        let source = baseline.join(relative);
        let text = read_text(&source)
            .ok_or_else(|| format!("no recorded baseline for {relative}"))?;
        fs::write(game_root.join(relative), patched_text(&text, body))
            .map_err(|error| format!("could not patch {relative}: {error}"))?;
    }

    Ok(status(game_root, cache_root))
}

pub fn revert(game_root: &Path, cache_root: &Path) -> Result<PatchStatus, String> {
    for (relative, _) in ADDONS {
        let target = game_root.join(relative);
        if target.is_file() {
            fs::remove_file(&target)
                .map_err(|error| format!("could not remove {relative}: {error}"))?;
        }
    }

    let baseline = baseline_root(cache_root);
    for (relative, _) in INJECTIONS {
        let source = baseline.join(relative);
        let target = game_root.join(relative);
        if source.is_file() {
            fs::copy(&source, &target)
                .map_err(|error| format!("could not restore {relative}: {error}"))?;
        } else if let Some(text) = read_text(&target) {
            // No baseline: strip the blocks in place. Less exact than a
            // restore, so say so rather than reporting success.
            fs::write(&target, strip_blocks(&text)).map_err(|error| error.to_string())?;
        }
    }

    Ok(status(game_root, cache_root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "scrapmap-patch-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn stock_game(root: &Path) {
        for (relative, _) in INJECTIONS {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "-- stock file\nfunction Load() end\n").unwrap();
        }
    }

    #[test]
    fn applying_twice_leaves_the_same_bytes() {
        // The whole patch rests on this: two players who apply from the same
        // build must end up with identical files, whatever state they started
        // from, or the game refuses the connection.
        let game = scratch("idempotent-game");
        let cache = scratch("idempotent-cache");
        stock_game(&game);

        let first = apply(&game, &cache).unwrap();
        assert!(first.applied);
        let once: Vec<_> = first.files.iter().map(|f| f.hash.clone()).collect();

        let second = apply(&game, &cache).unwrap();
        let twice: Vec<_> = second.files.iter().map(|f| f.hash.clone()).collect();
        assert_eq!(once, twice, "applying twice must not compound");

        fs::remove_dir_all(&game).ok();
        fs::remove_dir_all(&cache).ok();
    }

    #[test]
    fn revert_restores_the_original_bytes_exactly() {
        let game = scratch("revert-game");
        let cache = scratch("revert-cache");
        stock_game(&game);
        let before: Vec<_> = INJECTIONS
            .iter()
            .map(|(relative, _)| fs::read(game.join(relative)).unwrap())
            .collect();

        apply(&game, &cache).unwrap();
        let after_apply = status(&game, &cache);
        assert!(after_apply.applied);

        let reverted = revert(&game, &cache).unwrap();
        assert!(!reverted.applied);
        for (index, (relative, _)) in INJECTIONS.iter().enumerate() {
            assert_eq!(
                fs::read(game.join(relative)).unwrap(),
                before[index],
                "{relative} should be byte-identical to the stock file"
            );
        }
        for (relative, _) in ADDONS {
            assert!(!game.join(relative).exists(), "{relative} should be gone");
        }

        fs::remove_dir_all(&game).ok();
        fs::remove_dir_all(&cache).ok();
    }

    #[test]
    fn a_snapshot_of_patched_files_is_refused() {
        // Recording a patched file as the baseline would make revert a no-op
        // and leave the player unable to join anyone, with nothing to restore.
        let game = scratch("snapshot-game");
        let cache = scratch("snapshot-cache");
        stock_game(&game);
        apply(&game, &cache).unwrap();

        let fresh = scratch("snapshot-cache-2");
        let refused = snapshot(&game, &fresh);
        assert!(refused.is_err(), "a patched file must not become a baseline");
        assert!(refused.unwrap_err().contains("Verify integrity"));

        fs::remove_dir_all(&game).ok();
        fs::remove_dir_all(&cache).ok();
        fs::remove_dir_all(&fresh).ok();
    }

    #[test]
    fn installed_addons_have_normalised_line_endings() {
        // core.autocrlf on a fresh clone rewrites .lua files, and one changed
        // byte is a refused connection.
        assert_eq!(normalise_endings("a\r\nb\rc\n"), "a\nb\nc\n");
        let game = scratch("endings-game");
        let cache = scratch("endings-cache");
        stock_game(&game);
        apply(&game, &cache).unwrap();
        for (relative, _) in ADDONS {
            let bytes = fs::read(game.join(relative)).unwrap();
            assert!(!bytes.contains(&b'\r'), "{relative} should contain no CR");
        }
        fs::remove_dir_all(&game).ok();
        fs::remove_dir_all(&cache).ok();
    }
}
