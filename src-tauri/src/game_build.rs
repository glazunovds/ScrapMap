use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::server_identity::process_executable_path;

const MAX_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 1024 * 1024;
const COMPATIBILITY_ID_PREFIX: &str = "smgb1_";

struct KnownBuild {
    sha256: &'static str,
    layout_id: &'static str,
}

// Add a hash only after its native layout has been verified against that exact executable.
const KNOWN_BUILDS: &[KnownBuild] = &[KnownBuild {
    sha256: "4d2a8c50da8f1c20826a522e3b64ef49c167e374957d1b132bb0e9611fd882b4",
    layout_id: "sm-1.0.3.871-win64-candidate-v1",
}];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GameBuildState {
    Missing,
    Recognized,
    Unsupported,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GameBuildIssue {
    InvalidProcessId,
    ProcessUnavailable,
    ExecutableUnreadable,
    ExecutableTooLarge,
    ExecutableChanged,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameBuildProbe {
    pub state: GameBuildState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<GameBuildIssue>,
}

impl GameBuildProbe {
    fn unavailable(issue: GameBuildIssue) -> Self {
        Self {
            state: GameBuildState::Unavailable,
            executable_sha256: None,
            compatibility_id: None,
            layout_id: None,
            size_bytes: None,
            issue: Some(issue),
        }
    }

    fn missing(issue: GameBuildIssue) -> Self {
        Self {
            state: GameBuildState::Missing,
            executable_sha256: None,
            compatibility_id: None,
            layout_id: None,
            size_bytes: None,
            issue: Some(issue),
        }
    }
}

pub fn probe_game_build_process(process_id: u32) -> GameBuildProbe {
    if process_id == 0 {
        return GameBuildProbe::missing(GameBuildIssue::InvalidProcessId);
    }
    let Some(executable) = process_executable_path(process_id) else {
        return GameBuildProbe::missing(GameBuildIssue::ProcessUnavailable);
    };
    probe_game_build_file(&executable)
}

fn probe_game_build_file(path: &Path) -> GameBuildProbe {
    match hash_stable_file(path) {
        Ok((sha256, size_bytes)) => classify_build(sha256, size_bytes),
        Err(HashIssue::Io) => GameBuildProbe::unavailable(GameBuildIssue::ExecutableUnreadable),
        Err(HashIssue::TooLarge) => GameBuildProbe::unavailable(GameBuildIssue::ExecutableTooLarge),
        Err(HashIssue::Changed) => GameBuildProbe::unavailable(GameBuildIssue::ExecutableChanged),
    }
}

fn classify_build(sha256: String, size_bytes: u64) -> GameBuildProbe {
    let known = KNOWN_BUILDS
        .iter()
        .find(|build| build.sha256.eq_ignore_ascii_case(&sha256));
    GameBuildProbe {
        state: if known.is_some() {
            GameBuildState::Recognized
        } else {
            GameBuildState::Unsupported
        },
        compatibility_id: Some(format!("{COMPATIBILITY_ID_PREFIX}{sha256}")),
        executable_sha256: Some(sha256),
        layout_id: known.map(|build| build.layout_id.to_owned()),
        size_bytes: Some(size_bytes),
        issue: None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HashIssue {
    Io,
    TooLarge,
    Changed,
}

fn hash_stable_file(path: &Path) -> Result<(String, u64), HashIssue> {
    let mut file = File::open(path).map_err(|_| HashIssue::Io)?;
    let before = file.metadata().map_err(|_| HashIssue::Io)?;
    if !before.is_file() {
        return Err(HashIssue::Io);
    }
    if before.len() > MAX_EXECUTABLE_BYTES {
        return Err(HashIssue::TooLarge);
    }

    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer).map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => HashIssue::Changed,
            _ => HashIssue::Io,
        })?;
        if read == 0 {
            break;
        }
        total = total.checked_add(read as u64).ok_or(HashIssue::TooLarge)?;
        if total > MAX_EXECUTABLE_BYTES {
            return Err(HashIssue::TooLarge);
        }
        hasher.update(&buffer[..read]);
    }

    let after = file.metadata().map_err(|_| HashIssue::Io)?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || total != after.len()
    {
        return Err(HashIssue::Changed);
    }

    let sha256 = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((sha256, total))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock is before Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "scrapmap-game-build-{}-{nonce}.exe",
                std::process::id()
            ));
            fs::write(&path, bytes).expect("failed to write build fixture");
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn build_probe_is_deterministic_and_path_free() {
        const BYTES: &[u8] = b"synthetic Scrap Mechanic executable";
        let first = Fixture::new(BYTES);
        let second = Fixture::new(BYTES);
        let first_probe = probe_game_build_file(&first.0);
        let second_probe = probe_game_build_file(&second.0);

        assert_eq!(first_probe.state, GameBuildState::Unsupported);
        assert_eq!(
            first_probe.executable_sha256,
            second_probe.executable_sha256
        );
        assert_eq!(first_probe.compatibility_id, second_probe.compatibility_id);
        assert_eq!(first_probe.size_bytes, Some(BYTES.len() as u64));
        assert!(first_probe.layout_id.is_none());
        assert!(first_probe
            .compatibility_id
            .as_deref()
            .is_some_and(|value| value.starts_with(COMPATIBILITY_ID_PREFIX)));
    }

    #[test]
    fn allowlisted_hash_selects_only_its_candidate_layout() {
        let probe = classify_build(KNOWN_BUILDS[0].sha256.to_owned(), 123);
        assert_eq!(probe.state, GameBuildState::Recognized);
        assert_eq!(probe.layout_id.as_deref(), Some(KNOWN_BUILDS[0].layout_id));
    }

    #[test]
    fn missing_build_file_fails_closed() {
        let probe = probe_game_build_file(Path::new("missing-synthetic-build.exe"));
        assert_eq!(probe.state, GameBuildState::Unavailable);
        assert_eq!(probe.issue, Some(GameBuildIssue::ExecutableUnreadable));
        assert!(probe.executable_sha256.is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn current_process_is_observed_through_the_win32_probe() {
        let probe = probe_game_build_process(std::process::id());
        assert!(matches!(
            probe.state,
            GameBuildState::Recognized | GameBuildState::Unsupported
        ));
        assert_eq!(probe.executable_sha256.as_deref().map(str::len), Some(64));
        assert!(probe.issue.is_none());
    }
}
