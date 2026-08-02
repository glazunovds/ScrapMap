use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Maximum accepted Scrap Mechanic log size.
///
/// The identity probe is deliberately not a general-purpose log reader. A
/// larger file is rejected instead of allocating an unbounded buffer or
/// returning an identity derived from a truncated connection history.
/// Ceiling on a game log the probe will look at.
///
/// This was 4 MiB, which a normal session stays well under -- but an hour of POI
/// photography writes telemetry four times a second and pushed one log to 62 MB.
/// The probe then failed closed on every poll, the world never got an identity,
/// and the profile stayed quarantined: no position, no fog. The log is streamed
/// rather than read into memory, so the only reason for a limit at all is to
/// refuse something absurd.
pub const MAX_GAME_LOG_BYTES: u64 = 512 * 1024 * 1024;

const MAX_PID_SCAN_LINES: usize = 64;
const LOG_FILE_PREFIX: &str = "game-";
const LOG_FILE_SUFFIX: &str = ".log";
const NETWORK_CONNECT_MARKER: &str = "[Network] Connecting to ";
/// The game tags each line with its main *thread* id, not its process id.
const MAIN_THREAD_MARKER: &str = "[Main:";
const STABLE_ID_PREFIX: &str = "steam-sha256:";
const STABLE_ID_DOMAIN_SALT: &[u8] = b"scrapmap-peer-host-v1\0";
const OBSERVATION_ID_PREFIX: &str = "connection-sha256:";
const OBSERVATION_ID_DOMAIN_SALT: &[u8] = b"scrapmap-connection-observation-v1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerIdentityKind {
    Local,
    PeerHosted,
    Unknown,
}

/// An opaque identity suitable for the existing `ServerIdentityV1` contract.
///
/// A raw SteamID64 is never stored in this value. `stable_id` is populated only
/// for a peer-hosted connection and contains a domain-separated SHA-256 digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerIdentity {
    kind: ServerIdentityKind,
    stable_id: Option<String>,
}

impl ServerIdentity {
    #[cfg(test)]
    pub const fn kind(&self) -> ServerIdentityKind {
        self.kind
    }

    #[cfg(test)]
    pub fn stable_id(&self) -> Option<&str> {
        self.stable_id.as_deref()
    }

    fn local() -> Self {
        Self {
            kind: ServerIdentityKind::Local,
            stable_id: None,
        }
    }

    fn peer_hosted(stable_id: String) -> Self {
        Self {
            kind: ServerIdentityKind::PeerHosted,
            stable_id: Some(stable_id),
        }
    }

    fn unknown() -> Self {
        Self {
            kind: ServerIdentityKind::Unknown,
            stable_id: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerIdentityIssue {
    InvalidExpectedPid,
    ProcessUnavailable,
    LogsUnavailable,
    NoGameLog,
    LogTooLarge,
    LogUnreadable,
    InvalidUtf8,
    MissingProcessId,
    ProcessIdMismatch,
    NoConnectionIdentity,
    InvalidConnectionIdentity,
    IncompleteConnectionLine,
}

/// Result of a read-only identity probe.
///
/// `issue` intentionally contains only a bounded enum: neither filesystem
/// paths nor raw log contents can cross the module boundary through errors.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerIdentityProbe {
    pub identity: ServerIdentity,
    pub observation_id: Option<String>,
    pub issue: Option<ServerIdentityIssue>,
}

impl ServerIdentityProbe {
    fn known(
        identity: ServerIdentity,
        log_name: &str,
        line_index: usize,
        byte_offset: usize,
    ) -> Self {
        let observation_id =
            hash_connection_observation(log_name, line_index, byte_offset, &identity);
        Self {
            identity,
            observation_id: Some(observation_id),
            issue: None,
        }
    }

    fn unknown(issue: ServerIdentityIssue) -> Self {
        Self {
            identity: ServerIdentity::unknown(),
            observation_id: None,
            issue: Some(issue),
        }
    }
}

/// Probe `<game_install_dir>/Logs` for the active Scrap Mechanic server.
///
/// `expected_pid` should come from the already discovered game window. The
/// newest log is rejected if its early `[Main:<pid>]` marker does not match,
/// preventing an older game session from being mistaken for the active one.
pub fn probe_game_install(
    game_install_dir: impl AsRef<Path>,
    expected_pid: u32,
) -> ServerIdentityProbe {
    probe_logs_directory(game_install_dir.as_ref().join("Logs"), expected_pid)
}

/// Resolve the running game's installation directory from its process image,
/// then inspect its Logs directory without exposing either path to the UI.
pub fn probe_game_process(expected_pid: u32) -> ServerIdentityProbe {
    if expected_pid == 0 {
        return ServerIdentityProbe::unknown(ServerIdentityIssue::InvalidExpectedPid);
    }
    let Some(install_directory) = process_install_directory(expected_pid) else {
        return ServerIdentityProbe::unknown(ServerIdentityIssue::ProcessUnavailable);
    };
    probe_game_install(install_directory, expected_pid)
}

#[cfg(target_os = "windows")]
pub(crate) fn process_executable_path(process_id: u32) -> Option<PathBuf> {
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    };

    let process =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
    let mut buffer = vec![0_u16; 32_768];
    let mut length = u32::try_from(buffer.len()).ok()?;
    let result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    result.ok()?;
    buffer.truncate(length as usize);
    Some(PathBuf::from(String::from_utf16(&buffer).ok()?))
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn process_executable_path(_process_id: u32) -> Option<PathBuf> {
    None
}

fn process_install_directory(process_id: u32) -> Option<PathBuf> {
    let executable = process_executable_path(process_id)?;
    installation_directory_from_executable(&executable)
}

fn installation_directory_from_executable(executable: &Path) -> Option<PathBuf> {
    let executable_directory = executable.parent()?;
    if executable_directory
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("Release"))
    {
        executable_directory.parent().map(Path::to_path_buf)
    } else {
        Some(executable_directory.to_path_buf())
    }
}

/// Probe a supplied Logs directory.
///
/// This entry point keeps tests independent from an installed game and also
/// allows a future Tauri state object to use a pre-resolved, trusted Logs path.
pub fn probe_logs_directory(
    logs_directory: impl AsRef<Path>,
    expected_pid: u32,
) -> ServerIdentityProbe {
    if expected_pid == 0 {
        return ServerIdentityProbe::unknown(ServerIdentityIssue::InvalidExpectedPid);
    }

    let path = match newest_game_log(logs_directory.as_ref()) {
        Ok(Some(path)) => path,
        Ok(None) => return ServerIdentityProbe::unknown(ServerIdentityIssue::NoGameLog),
        Err(_) => return ServerIdentityProbe::unknown(ServerIdentityIssue::LogsUnavailable),
    };

    probe_log_file(&path, expected_pid)
}

fn newest_game_log(logs_directory: &Path) -> io::Result<Option<PathBuf>> {
    let entries = fs::read_dir(logs_directory)?;
    let mut newest: Option<(String, PathBuf)> = None;

    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            continue;
        }

        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_game_log_name(&name) {
            continue;
        }

        // The exact `YYYYMMDD-HHMMSS` suffix sorts chronologically.
        if newest
            .as_ref()
            .is_none_or(|(current_name, _)| name > *current_name)
        {
            newest = Some((name, entry.path()));
        }
    }

    Ok(newest.map(|(_, path)| path))
}

fn is_game_log_name(name: &str) -> bool {
    let Some(timestamp) = name
        .strip_prefix(LOG_FILE_PREFIX)
        .and_then(|value| value.strip_suffix(LOG_FILE_SUFFIX))
    else {
        return false;
    };

    timestamp.len() == 15
        && timestamp.as_bytes()[8] == b'-'
        && timestamp
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 8 || byte.is_ascii_digit())
}

fn probe_log_file(path: &Path, expected_pid: u32) -> ServerIdentityProbe {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return ServerIdentityProbe::unknown(ServerIdentityIssue::LogUnreadable),
    };
    match file.metadata() {
        Ok(metadata) if metadata.len() > MAX_GAME_LOG_BYTES => {
            return ServerIdentityProbe::unknown(ServerIdentityIssue::LogTooLarge)
        }
        Ok(_) => {}
        Err(_) => return ServerIdentityProbe::unknown(ServerIdentityIssue::LogUnreadable),
    }

    let Some(log_name) = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| is_game_log_name(name))
        .map(str::to_owned)
    else {
        return ServerIdentityProbe::unknown(ServerIdentityIssue::NoGameLog);
    };

    // Streamed a line at a time. These logs grow without bound while the game
    // runs, so holding one in memory is not an option, and the identity only
    // depends on the header and the last connection line anyway.
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let mut header_id = None;
    let mut header_lines = 0;
    let mut latest: Option<(ConnectionTarget, usize, usize)> = None;
    let mut line_index = 0_usize;
    let mut byte_offset = 0_usize;

    loop {
        buffer.clear();
        let read = match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => return ServerIdentityProbe::unknown(ServerIdentityIssue::LogUnreadable),
        };
        let segment = match std::str::from_utf8(&buffer) {
            Ok(segment) => segment,
            Err(_) => return ServerIdentityProbe::unknown(ServerIdentityIssue::InvalidUtf8),
        };

        // A line without its terminator is still being written. Half a
        // connection line must not be read as a connection to somewhere else.
        if !segment.ends_with('\n') {
            if segment.contains(NETWORK_CONNECT_MARKER) {
                return ServerIdentityProbe::unknown(ServerIdentityIssue::IncompleteConnectionLine);
            }
            break;
        }

        let line = segment.strip_suffix('\n').unwrap_or(segment);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if header_id.is_none() && header_lines < MAX_PID_SCAN_LINES {
            header_lines += 1;
            header_id = main_thread_id_from_line(line);
        }
        if let Some(target) = connection_target(line) {
            latest = Some((target, line_index, byte_offset));
        }
        byte_offset += read;
        line_index += 1;
    }

    match header_id {
        Some(id) if id == expected_pid || thread_belongs_to_process(id, expected_pid) => {}
        Some(_) => return ServerIdentityProbe::unknown(ServerIdentityIssue::ProcessIdMismatch),
        None => return ServerIdentityProbe::unknown(ServerIdentityIssue::MissingProcessId),
    }

    match latest {
        Some((ConnectionTarget::Local, line_index, line_offset)) => {
            ServerIdentityProbe::known(ServerIdentity::local(), &log_name, line_index, line_offset)
        }
        Some((ConnectionTarget::Peer(stable_id), line_index, line_offset)) => {
            ServerIdentityProbe::known(
                ServerIdentity::peer_hosted(stable_id),
                &log_name,
                line_index,
                line_offset,
            )
        }
        Some((ConnectionTarget::Invalid, _, _)) => {
            ServerIdentityProbe::unknown(ServerIdentityIssue::InvalidConnectionIdentity)
        }
        None => ServerIdentityProbe::unknown(ServerIdentityIssue::NoConnectionIdentity),
    }
}


/// Whether `thread_id` is one of `process_id`'s threads.
///
/// The game logs `[Main:<id>]`, and that id is its **main thread**, not its
/// process -- confirmed against a live game: PID 13880 logging `[Main:71964]`,
/// which is exactly what Windows reports as its first thread. Comparing the two
/// directly can therefore never match, which is why a world never acquired an
/// identity on its own and the profile sat quarantined.
///
/// Checking thread ownership keeps what the comparison was for -- proving the
/// log belongs to the process being tracked -- instead of weakening it to
/// "newest log wins".
#[cfg(target_os = "windows")]
fn thread_belongs_to_process(thread_id: u32, process_id: u32) -> bool {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        },
    };

    if thread_id == 0 || process_id == 0 {
        return false;
    }
    // SAFETY: a thread snapshot takes no input buffer, and the handle is closed
    // on every path out of this function.
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }) else {
        return false;
    };

    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut owned = false;
    // SAFETY: `entry` is a correctly sized THREADENTRY32 for both calls.
    if unsafe { Thread32First(snapshot, &mut entry) }.is_ok() {
        loop {
            if entry.th32ThreadID == thread_id && entry.th32OwnerProcessID == process_id {
                owned = true;
                break;
            }
            if unsafe { Thread32Next(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }
    // SAFETY: `snapshot` came from CreateToolhelp32Snapshot and is still open.
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    owned
}

#[cfg(not(target_os = "windows"))]
fn thread_belongs_to_process(_thread_id: u32, _process_id: u32) -> bool {
    false
}

fn main_thread_id_from_line(line: &str) -> Option<u32> {
    let marker_start = line.find(MAIN_THREAD_MARKER)? + MAIN_THREAD_MARKER.len();
    let suffix = &line[marker_start..];
    let marker_end = suffix.find(']')?;
    let digits = &suffix[..marker_end];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}


enum ConnectionTarget {
    Local,
    Peer(String),
    Invalid,
}

fn connection_target(line: &str) -> Option<ConnectionTarget> {
    let (_, target) = line.split_once(NETWORK_CONNECT_MARKER)?;
    let target = target.trim_end_matches('\r');
    if target == "self" {
        return Some(ConnectionTarget::Local);
    }
    if is_peer_steam_id64(target) {
        return Some(ConnectionTarget::Peer(hash_peer_steam_id(target)));
    }
    Some(ConnectionTarget::Invalid)
}

fn is_peer_steam_id64(value: &str) -> bool {
    if value.len() != 17 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let Ok(id) = value.parse::<u64>() else {
        return false;
    };

    // SteamID64 bit layout: public universe (1), individual account type (1)
    // and desktop instance (1). Account id zero is accepted for synthetic test
    // fixtures; no real user identifier is needed to exercise the parser.
    let universe = id >> 56;
    let account_type = (id >> 52) & 0x0f;
    let instance = (id >> 32) & 0x000f_ffff;
    universe == 1 && account_type == 1 && instance == 1
}

fn hash_peer_steam_id(steam_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(STABLE_ID_DOMAIN_SALT);
    hasher.update(steam_id.as_bytes());
    let digest = hasher.finalize();

    let mut encoded = String::with_capacity(STABLE_ID_PREFIX.len() + digest.len() * 2);
    encoded.push_str(STABLE_ID_PREFIX);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing into a String cannot fail");
    }
    encoded
}

fn hash_connection_observation(
    log_name: &str,
    line_index: usize,
    byte_offset: usize,
    identity: &ServerIdentity,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(OBSERVATION_ID_DOMAIN_SALT);
    hasher.update(log_name.as_bytes());
    hasher.update([0]);
    hasher.update((line_index as u64).to_le_bytes());
    hasher.update((byte_offset as u64).to_le_bytes());
    hasher.update([0]);
    match identity.kind {
        ServerIdentityKind::Local => hasher.update(b"local"),
        ServerIdentityKind::PeerHosted => {
            hasher.update(b"peer-hosted\0");
            hasher.update(
                identity
                    .stable_id
                    .as_deref()
                    .expect("peer-hosted identity always has an opaque stable id")
                    .as_bytes(),
            );
        }
        ServerIdentityKind::Unknown => {
            unreachable!("unknown identities never receive an observation id")
        }
    }
    let digest = hasher.finalize();

    let mut encoded = String::with_capacity(OBSERVATION_ID_PREFIX.len() + digest.len() * 2);
    encoded.push_str(OBSERVATION_ID_PREFIX);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing into a String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File, OpenOptions},
        io::Write,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    const PID: u32 = 42_424;
    // Public/individual/desktop SteamID64 with a synthetic zero account id.
    const SYNTHETIC_HOST_ID: &str = "76561197960265728";

    static DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock must be after the Unix epoch")
                .as_nanos();
            let counter = DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "scrapmap-server-identity-{}-{timestamp}-{counter}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("failed to create test directory");
            Self(path)
        }

        fn write_log(&self, name: &str, body: &str) {
            fs::write(self.0.join(name), body).expect("failed to write test log");
        }

        fn append_log(&self, name: &str, body: &str) {
            OpenOptions::new()
                .append(true)
                .open(self.0.join(name))
                .expect("failed to open test log for append")
                .write_all(body.as_bytes())
                .expect("failed to append test log");
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn header(pid: u32) -> String {
        format!(
            "12:00:00 (1/0) [Main:{pid}] [Default] Initialized Logger\n\
             12:00:00 (1/0) [Main:{pid}] [Default] Game version synthetic\n"
        )
    }

    fn remote_log(pid: u32) -> String {
        format!(
            "{}12:00:01 (1/1) [Main:{pid}] [Network] Connecting to {SYNTHETIC_HOST_ID}\n",
            header(pid)
        )
    }

    #[test]
    fn newest_well_formed_log_is_selected() {
        let directory = TestDirectory::new();
        directory.write_log(
            "game-20260101-010101.log",
            &format!(
                "{}12:00:01 (1/1) [Main:{PID}] [Network] Connecting to {SYNTHETIC_HOST_ID}\n",
                header(PID)
            ),
        );
        directory.write_log(
            "game-20260102-010101.log",
            &format!(
                "{}12:00:01 (1/1) [Main:{PID}] [Network] Connecting to self\n",
                header(PID)
            ),
        );
        directory.write_log("game-newest.log", &remote_log(PID));
        directory.write_log("notes.log", &remote_log(PID));

        let probe = probe_logs_directory(&directory.0, PID);
        assert_eq!(probe.identity.kind(), ServerIdentityKind::Local);
        assert_eq!(probe.identity.stable_id(), None);
        assert!(probe
            .observation_id
            .as_deref()
            .is_some_and(|id| id.starts_with(OBSERVATION_ID_PREFIX)));
        assert_eq!(probe.issue, None);
    }

    #[test]
    fn remote_target_becomes_a_deterministic_opaque_identity() {
        let directory = TestDirectory::new();
        directory.write_log("game-20260101-010101.log", &remote_log(PID));

        let first = probe_logs_directory(&directory.0, PID);
        let second = probe_logs_directory(&directory.0, PID);
        assert_eq!(first, second);
        assert_eq!(first.identity.kind(), ServerIdentityKind::PeerHosted);
        let stable_id = first
            .identity
            .stable_id()
            .expect("peer-hosted identity must have a digest");
        assert!(stable_id.starts_with(STABLE_ID_PREFIX));
        assert_eq!(stable_id.len(), STABLE_ID_PREFIX.len() + 64);
        assert!(!stable_id.contains(SYNTHETIC_HOST_ID));
        let observation_id = first
            .observation_id
            .as_deref()
            .expect("known connection must have an observation id");
        assert!(observation_id.starts_with(OBSERVATION_ID_PREFIX));
        assert_eq!(observation_id.len(), OBSERVATION_ID_PREFIX.len() + 64);
        assert!(!observation_id.contains(SYNTHETIC_HOST_ID));
        assert_eq!(first.issue, None);
    }

    #[test]
    fn a_new_connection_to_the_same_peer_changes_only_the_observation_id() {
        let directory = TestDirectory::new();
        let name = "game-20260101-010101.log";
        directory.write_log(name, &remote_log(PID));

        let first = probe_logs_directory(&directory.0, PID);
        directory.append_log(
            name,
            &format!("12:00:02 (1/2) [Main:{PID}] [Network] Connecting to {SYNTHETIC_HOST_ID}\n"),
        );
        let second = probe_logs_directory(&directory.0, PID);

        assert_eq!(first.identity, second.identity);
        assert_ne!(first.observation_id, second.observation_id);
        assert!(first.observation_id.is_some());
        assert!(second.observation_id.is_some());
    }

    #[test]
    fn last_connection_event_wins_without_using_handles_or_world_indices() {
        let directory = TestDirectory::new();
        let body = format!(
            "{}\
             12:00:01 (1/1) [Main:{PID}] [Network] Connecting to self\n\
             12:00:02 (1/2) [Main:{PID}] [Network] Connection handle: 123, user: 0\n\
             12:00:03 (1/3) [Main:{PID}] [World] Creating world 2\n\
             12:00:04 (1/4) [Main:{PID}] [Network] Connecting to {SYNTHETIC_HOST_ID}\n",
            header(PID)
        );
        directory.write_log("game-20260101-010101.log", &body);

        let probe = probe_logs_directory(&directory.0, PID);
        assert_eq!(probe.identity.kind(), ServerIdentityKind::PeerHosted);
        assert_eq!(probe.issue, None);
    }

    #[test]
    fn pid_mismatch_fails_closed() {
        let directory = TestDirectory::new();
        directory.write_log("game-20260101-010101.log", &remote_log(PID + 1));

        let probe = probe_logs_directory(&directory.0, PID);
        assert_eq!(probe.identity.kind(), ServerIdentityKind::Unknown);
        assert_eq!(probe.identity.stable_id(), None);
        assert_eq!(probe.observation_id, None);
        assert_eq!(probe.issue, Some(ServerIdentityIssue::ProcessIdMismatch));
    }

    #[test]
    fn oversized_log_fails_closed() {
        let directory = TestDirectory::new();
        let path = directory.0.join("game-20260101-010101.log");
        let file = File::create(path).expect("failed to create oversized test log");
        file.set_len(MAX_GAME_LOG_BYTES + 1)
            .expect("failed to resize test log");

        let probe = probe_logs_directory(&directory.0, PID);
        assert_eq!(probe.identity.kind(), ServerIdentityKind::Unknown);
        assert_eq!(probe.issue, Some(ServerIdentityIssue::LogTooLarge));
    }

    #[test]
    fn newest_invalid_log_does_not_fall_back_to_stale_identity() {
        let directory = TestDirectory::new();
        directory.write_log("game-20260101-010101.log", &remote_log(PID));
        directory.write_log("game-20260102-010101.log", "not a Scrap Mechanic log\n");

        let probe = probe_logs_directory(&directory.0, PID);
        assert_eq!(probe.identity.kind(), ServerIdentityKind::Unknown);
        assert_eq!(probe.issue, Some(ServerIdentityIssue::MissingProcessId));
    }

    #[test]
    fn invalid_latest_connection_clears_an_older_valid_identity() {
        let directory = TestDirectory::new();
        let body = format!(
            "{}\
             12:00:01 (1/1) [Main:{PID}] [Network] Connecting to {SYNTHETIC_HOST_ID}\n\
             12:00:02 (1/2) [Main:{PID}] [Network] Connecting to not-a-steam-id\n",
            header(PID)
        );
        directory.write_log("game-20260101-010101.log", &body);

        let probe = probe_logs_directory(&directory.0, PID);
        assert_eq!(probe.identity.kind(), ServerIdentityKind::Unknown);
        assert_eq!(probe.observation_id, None);
        assert_eq!(
            probe.issue,
            Some(ServerIdentityIssue::InvalidConnectionIdentity)
        );
    }

    #[test]
    fn incomplete_connection_line_fails_closed() {
        let directory = TestDirectory::new();
        let body = format!(
            "{}\
             12:00:01 (1/1) [Main:{PID}] [Network] Connecting to self\n\
             12:00:02 (1/2) [Main:{PID}] [Network] Connecting to {SYNTHETIC_HOST_ID}",
            header(PID)
        );
        directory.write_log("game-20260101-010101.log", &body);

        let probe = probe_logs_directory(&directory.0, PID);
        assert_eq!(probe.identity.kind(), ServerIdentityKind::Unknown);
        assert_eq!(
            probe.issue,
            Some(ServerIdentityIssue::IncompleteConnectionLine)
        );
    }

    #[test]
    fn invalid_utf8_and_missing_directory_fail_closed() {
        let directory = TestDirectory::new();
        let mut file = File::create(directory.0.join("game-20260101-010101.log"))
            .expect("failed to create invalid UTF-8 test log");
        file.write_all(&[0xff, 0xfe, 0xfd])
            .expect("failed to write invalid UTF-8 test log");

        let invalid_utf8 = probe_logs_directory(&directory.0, PID);
        assert_eq!(invalid_utf8.issue, Some(ServerIdentityIssue::InvalidUtf8));

        let missing = probe_logs_directory(directory.0.join("missing"), PID);
        assert_eq!(missing.issue, Some(ServerIdentityIssue::LogsUnavailable));
    }

    #[test]
    fn release_executable_resolves_logs_from_the_game_root() {
        let executable = Path::new("synthetic-game")
            .join("Release")
            .join("ScrapMechanic.exe");
        let install = installation_directory_from_executable(&executable)
            .expect("the synthetic executable path has a parent");
        assert_eq!(install, Path::new("synthetic-game"));

        let flat_executable = Path::new("flat-game").join("ScrapMechanic.exe");
        assert_eq!(
            installation_directory_from_executable(&flat_executable).as_deref(),
            Some(Path::new("flat-game"))
        );
    }
}
