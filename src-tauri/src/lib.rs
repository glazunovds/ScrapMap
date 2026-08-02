mod asset_catalogue;
mod atlas_bake;
mod diagnostic_source;
mod game_build;
mod game_log_source;
mod game_window;
mod native_process;
mod poi_capture;
mod server_identity;
mod storage;
mod tile_atlas;
mod window_capture;

use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use diagnostic_source::{
    DiagnosticIssue, DiagnosticIssueKind, DiagnosticPoll, DiagnosticSource, TelemetrySnapshot,
    DEFAULT_POLL_INTERVAL,
};
use game_build::{probe_game_build_process, GameBuildProbe};
use game_log_source::GameLogSource;
use game_window::{
    full_overlay_geometry, mini_overlay_geometry, overlay_should_be_present, MiniCorner,
    DEFAULT_MINI_LOGICAL_MARGIN, DEFAULT_MINI_LOGICAL_SIZE,
    GameWindowPoll, GameWindowSnapshot, GameWindowTracker, NativeWindowHandle, OverlayGeometry,
    WindowIdentity,
};
use native_process::{game_log_directory, probe_native_process, NativeTransportProbe};
use serde::Serialize;
use server_identity::{probe_game_process, ServerIdentityProbe};
use storage::{
    AcceptTelemetrySequenceRequestV1, ActivateProfileRequestV1, FogMergeResultV1,
    ListManualProfilesRequestV1, ListManualProfilesResultV1, MarkerReplaceResultV1,
    MergeFogRequestV1, ProfileSettingsV1, ProfileSnapshotV1, ReplaceLocalMarkersRequestV1, RouteV1,
    SaveProfileSettingsRequestV1, SetActiveRouteRequestV1, StorageState, TrailWriteResultV1,
    WriteTrailBatchRequestV1,
};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};
use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};
use tile_atlas::{atlas_manifest, atlas_preview};

const GAME_WINDOW_POLL_INTERVAL: Duration = Duration::from_millis(200);
const MISSING_GAME_POLL_INTERVAL: Duration = Duration::from_secs(1);
const MAX_GAME_LAYOUT_BYTES: u64 = 16 * 1024 * 1024;

struct OverlayState {
    expanded: AtomicBool,
    user_visible: AtomicBool,
    effective_visible: AtomicBool,
    transition: Mutex<()>,
    last_geometry: Mutex<Option<OverlayGeometry>>,
    last_game_status: Mutex<Option<GameWindowStatusEvent>>,
    game_process_id: AtomicU32,
    /// Compact-map size and corner, so the map can be moved clear of the
    /// game's own HUD without rebuilding.
    mini_size: AtomicU32,
    mini_corner: AtomicU32,
    /// The game's top-level window, so POI photography can capture it without
    /// re-discovering it on another thread.
    game_window_handle: AtomicI64,
}

#[derive(Default)]
struct DiagnosticRuntime {
    latest: Mutex<Option<TelemetrySnapshot>>,
    last_received: Mutex<Option<Instant>>,
    status: Mutex<Option<DiagnosticStatusEvent>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GameLayoutSignature {
    path: PathBuf,
    modified: Option<SystemTime>,
    length: u64,
}

#[derive(Default)]
struct GameLayoutRuntime {
    last_sent: Mutex<Option<GameLayoutSignature>>,
}

struct ProfileStorageRuntime {
    storage: Option<StorageState>,
    startup_error: Option<String>,
}

impl ProfileStorageRuntime {
    fn open_default() -> Self {
        match StorageState::open_default() {
            Ok(storage) => Self {
                storage: Some(storage),
                startup_error: None,
            },
            Err(error) => Self {
                storage: None,
                startup_error: Some(error.to_string()),
            },
        }
    }

    fn storage(&self) -> Result<&StorageState, String> {
        self.storage.as_ref().ok_or_else(|| {
            self.startup_error
                .clone()
                .unwrap_or_else(|| "profile storage is unavailable".to_owned())
        })
    }
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            expanded: AtomicBool::new(false),
            user_visible: AtomicBool::new(true),
            effective_visible: AtomicBool::new(false),
            transition: Mutex::new(()),
            last_geometry: Mutex::new(None),
            last_game_status: Mutex::new(None),
            game_process_id: AtomicU32::new(0),
            mini_size: AtomicU32::new(DEFAULT_MINI_LOGICAL_SIZE),
            mini_corner: AtomicU32::new(1),
            game_window_handle: AtomicI64::new(0),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum GameWindowState {
    Missing,
    Background,
    Minimized,
    Unavailable,
    Active,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameWindowStatusEvent {
    state: GameWindowState,
    attached: bool,
    foreground: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayModeEvent {
    expanded: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum DiagnosticState {
    Waiting,
    Active,
    Stale,
    Invalid,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticStatusEvent {
    state: DiagnosticState,
    #[serde(skip_serializing_if = "Option::is_none")]
    issue: Option<DiagnosticIssue>,
}

fn main_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "main overlay window is unavailable".to_owned())
}

#[cfg(target_os = "windows")]
fn overlay_identity(window: &WebviewWindow) -> Option<WindowIdentity> {
    window.hwnd().ok().map(|handle| WindowIdentity {
        handle: NativeWindowHandle(handle.0 as isize),
        process_id: std::process::id(),
    })
}

#[cfg(not(target_os = "windows"))]
fn overlay_identity(_window: &WebviewWindow) -> Option<WindowIdentity> {
    None
}

fn game_status(poll: &GameWindowPoll, overlay: Option<WindowIdentity>) -> GameWindowStatusEvent {
    let Some(game) = poll.game.as_ref() else {
        return GameWindowStatusEvent {
            state: GameWindowState::Missing,
            attached: false,
            foreground: false,
        };
    };

    let foreground = overlay_should_be_present(true, Some(game), poll.foreground, overlay);
    let state = if !game.visible || game.minimized {
        GameWindowState::Minimized
    } else if game.client_rect.is_none_or(|rect| rect.is_empty()) {
        GameWindowState::Unavailable
    } else if foreground {
        GameWindowState::Active
    } else {
        GameWindowState::Background
    };

    GameWindowStatusEvent {
        state,
        attached: true,
        foreground,
    }
}

fn emit_game_status_if_changed(
    window: &WebviewWindow,
    state: &OverlayState,
    status: GameWindowStatusEvent,
) -> Result<(), String> {
    let mut previous = state
        .last_game_status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if previous.as_ref() == Some(&status) {
        return Ok(());
    }

    window
        .emit("scrapmap:game-window-status", status)
        .map_err(|error| error.to_string())?;
    *previous = Some(status);
    Ok(())
}

fn notify_frontend(window: &WebviewWindow, expanded: bool) -> Result<(), String> {
    window
        .emit("scrapmap:overlay-mode", OverlayModeEvent { expanded })
        .map_err(|error| error.to_string())
}

fn configure_overlay_mode(window: &WebviewWindow, expanded: bool) -> Result<(), String> {
    window
        .set_focusable(expanded)
        .map_err(|error| error.to_string())?;
    window
        .set_ignore_cursor_events(!expanded)
        .map_err(|error| error.to_string())?;
    window
        .set_resizable(false)
        .map_err(|error| error.to_string())
}

fn geometry_for_game(
    game: Option<&GameWindowSnapshot>,
    expanded: bool,
    state: &OverlayState,
) -> Option<OverlayGeometry> {
    let game = game?;
    let client = game.client_rect?;
    if expanded {
        full_overlay_geometry(client)
    } else {
        mini_overlay_geometry(
            client,
            game.dpi,
            state.mini_size.load(Ordering::Acquire),
            DEFAULT_MINI_LOGICAL_MARGIN,
            MiniCorner::from_code(state.mini_corner.load(Ordering::Acquire)),
        )
    }
}

fn apply_geometry_if_changed(
    window: &WebviewWindow,
    state: &OverlayState,
    geometry: OverlayGeometry,
) -> Result<(), String> {
    let mut previous = state
        .last_geometry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if previous.as_ref() == Some(&geometry) {
        return Ok(());
    }

    window
        .set_size(PhysicalSize::new(geometry.width, geometry.height))
        .map_err(|error| error.to_string())?;
    window
        .set_position(PhysicalPosition::new(geometry.x, geometry.y))
        .map_err(|error| error.to_string())?;
    *previous = Some(geometry);
    Ok(())
}

fn set_effective_visibility(
    window: &WebviewWindow,
    state: &OverlayState,
    visible: bool,
) -> Result<(), String> {
    let was_visible = state.effective_visible.load(Ordering::Acquire);
    if was_visible == visible {
        return Ok(());
    }

    if visible {
        window.show().map_err(|error| error.to_string())?;
    } else {
        window.hide().map_err(|error| error.to_string())?;
    }
    state.effective_visible.store(visible, Ordering::Release);
    Ok(())
}

fn synchronize_with_game_locked(
    window: &WebviewWindow,
    state: &OverlayState,
    poll: &GameWindowPoll,
    focus_expanded: bool,
) -> Result<(), String> {
    state.game_process_id.store(
        poll.game
            .as_ref()
            .map_or(0, |game| game.identity.process_id),
        Ordering::Release,
    );
    state.game_window_handle.store(
        poll.game
            .as_ref()
            .map_or(0, |game| game.identity.handle.0 as i64),
        Ordering::Release,
    );
    let expanded = state.expanded.load(Ordering::Acquire);
    let overlay = overlay_identity(window);

    if let Some(geometry) = geometry_for_game(poll.game.as_ref(), expanded, state) {
        apply_geometry_if_changed(window, state, geometry)?;
    }

    let should_show = overlay_should_be_present(
        state.user_visible.load(Ordering::Acquire),
        poll.game.as_ref(),
        poll.foreground,
        overlay,
    );
    set_effective_visibility(window, state, should_show)?;

    if should_show && expanded && focus_expanded {
        window.set_focus().map_err(|error| error.to_string())?;
    }

    emit_game_status_if_changed(window, state, game_status(poll, overlay))
}

fn synchronize_with_game(
    window: &WebviewWindow,
    state: &OverlayState,
    poll: &GameWindowPoll,
    focus_expanded: bool,
) -> Result<(), String> {
    let _transition = state
        .transition
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    synchronize_with_game_locked(window, state, poll, focus_expanded)
}

fn current_game_poll() -> GameWindowPoll {
    GameWindowTracker::new().poll()
}

fn apply_overlay_mode(app: &AppHandle, state: &OverlayState, expanded: bool) -> Result<(), String> {
    let window = main_window(app)?;
    let _transition = state
        .transition
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    configure_overlay_mode(&window, expanded)?;
    state.expanded.store(expanded, Ordering::Release);
    state.user_visible.store(true, Ordering::Release);
    synchronize_with_game_locked(&window, state, &current_game_poll(), true)?;
    notify_frontend(&window, expanded)
}

#[tauri::command]
fn set_overlay_mode(
    app: AppHandle,
    state: State<'_, OverlayState>,
    expanded: bool,
) -> Result<(), String> {
    apply_overlay_mode(&app, &state, expanded)
}

/// Sets the compact map's size and corner.
///
/// The window tracker re-applies geometry on its next poll, so this only has to
/// record the preference. Size is clamped to something that stays usable and
/// still fits a small game window.
#[tauri::command]
fn set_mini_overlay_layout(
    state: State<'_, OverlayState>,
    size: u32,
    corner: u32,
) -> Result<(), String> {
    if corner > 3 {
        return Err("corner must be 0..=3".to_owned());
    }
    state
        .mini_size
        .store(size.clamp(200, 900), Ordering::Release);
    state.mini_corner.store(corner, Ordering::Release);
    Ok(())
}


/// Prepares a POI photography sweep.
///
/// Writes the target list where Lua will find it. It cannot be picked up
/// mid-session -- sm.json.fileExists does not see files written after the game
/// started -- so the sweep begins on the next world load.
#[tauri::command]
fn poi_capture_prepare(state: State<'_, OverlayState>) -> Result<serde_json::Value, String> {
    let process_id = state.game_process_id.load(Ordering::Acquire);
    let Some(log_directory) = (process_id != 0)
        .then(|| game_log_directory(process_id))
        .flatten()
    else {
        return Err("Scrap Mechanic is not running".to_owned());
    };
    let game_root = log_directory
        .parent()
        .ok_or("could not locate the game directory")?;

    let layout_path = game_root.join("Survival").join("ScrapMapLayout.json");
    let bytes = fs::read(&layout_path)
        .map_err(|_| "no world layout yet -- load a survival world first".to_owned())?;
    let layout: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("layout is invalid: {error}"))?;

    // The sweep frames each shot against the tile's own ground, taken from the
    // baked atlas. Without it every photograph is framed from sea level, so say
    // how many targets are missing one rather than letting it pass unnoticed.
    let cache_root = atlas_bake::atlas_root().ok_or("LOCALAPPDATA is not set")?;
    let terrain = atlas_bake::tile_terrain(game_root, &cache_root);
    let all = poi_capture::build_targets(&layout, &terrain);
    if all.is_empty() {
        return Err("this world has no points of interest to photograph".to_owned());
    }
    // An interrupted sweep resumes rather than starting over.
    let total = all.len();
    let targets = poi_capture::remaining_targets(game_root, &cache_root, all);
    if targets.is_empty() {
        return Err("every point of interest in this world is already photographed".to_owned());
    }
    let without_ground = targets
        .iter()
        .filter(|target| !terrain.contains_key(&target.uuid.to_ascii_lowercase()))
        .count();
    poi_capture::write_request(game_root, &targets)?;
    // A tile only ever placed turned has to be photographed turned, and the
    // renderer will turn it again. Report it rather than let it look like noise.
    let rotated = targets.iter().filter(|target| target.rotation != 0).count();
    Ok(serde_json::json!({
        "targets": targets.len(),
        "alreadyPhotographed": total - targets.len(),
        "withoutGroundHeight": without_ground,
        "rotatedPlacements": rotated,
        "note": "reload the survival world to run the sweep",
    }))
}

#[tauri::command]
fn overlay_status(state: State<'_, OverlayState>) -> serde_json::Value {
    let game = *state
        .last_game_status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    serde_json::json!({
        "expanded": state.expanded.load(Ordering::Acquire),
        "userVisible": state.user_visible.load(Ordering::Acquire),
        "visible": state.effective_visible.load(Ordering::Acquire),
        "game": game,
        "toggleModeShortcut": "Ctrl+Shift+M",
        "toggleVisibilityShortcut": "Ctrl+Shift+H",
        "exitShortcut": "Ctrl+Shift+Q"
    })
}

#[tauri::command]
fn diagnostic_snapshot(state: State<'_, DiagnosticRuntime>) -> Option<TelemetrySnapshot> {
    state
        .latest
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

#[tauri::command]
fn game_layout_snapshot(
    state: State<'_, OverlayState>,
    runtime: State<'_, GameLayoutRuntime>,
) -> Result<Option<serde_json::Value>, String> {
    let process_id = state.game_process_id.load(Ordering::Acquire);
    let Some(log_directory) = (process_id != 0)
        .then(|| game_log_directory(process_id))
        .flatten()
    else {
        return Ok(None);
    };
    let Some(game_root) = log_directory.parent() else {
        return Ok(None);
    };
    let path = game_root.join("Survival").join("ScrapMapLayout.json");
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("layout metadata is unavailable ({})", error.kind())),
    };
    if metadata.len() > MAX_GAME_LAYOUT_BYTES {
        return Err("layout export exceeds the 16 MiB safety limit".to_owned());
    }
    let signature = GameLayoutSignature {
        path: path.clone(),
        modified: metadata.modified().ok(),
        length: metadata.len(),
    };
    if runtime
        .last_sent
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        == Some(&signature)
    {
        return Ok(None);
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("layout export is unreadable ({})", error.kind()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("layout export is invalid JSON ({error})"))?;
    *runtime
        .last_sent
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(signature);
    Ok(Some(value))
}

/// Turns whatever the in-game baker has produced so far into map tiles.
///
/// Safe to call repeatedly: tiles whose PNG is already current are skipped, so
/// this converges as the baker fills in more tiles across successive world loads.
#[tauri::command]
fn atlas_bake_refresh(
    state: State<'_, OverlayState>,
) -> Result<Option<atlas_bake::AtlasBakeReportV1>, String> {
    let process_id = state.game_process_id.load(Ordering::Acquire);
    let Some(log_directory) = (process_id != 0)
        .then(|| game_log_directory(process_id))
        .flatten()
    else {
        return Ok(None);
    };
    let Some(game_root) = log_directory.parent() else {
        return Ok(None);
    };
    let Some(atlas_root) = atlas_bake::atlas_root() else {
        return Err("LOCALAPPDATA is not set".to_owned());
    };
    atlas_bake::convert_baked_atlas(game_root, &atlas_root).map(Some)
}

#[tauri::command]
fn diagnostic_status(state: State<'_, DiagnosticRuntime>) -> Option<DiagnosticStatusEvent> {
    state
        .status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

#[tauri::command]
fn server_identity_probe(state: State<'_, OverlayState>) -> ServerIdentityProbe {
    probe_game_process(state.game_process_id.load(Ordering::Acquire))
}

#[tauri::command]
fn game_build_probe(state: State<'_, OverlayState>) -> GameBuildProbe {
    probe_game_build_process(state.game_process_id.load(Ordering::Acquire))
}

#[tauri::command]
fn native_transport_probe(state: State<'_, OverlayState>) -> NativeTransportProbe {
    probe_native_process(state.game_process_id.load(Ordering::Acquire))
}

#[tauri::command]
fn profile_activate(
    state: State<'_, ProfileStorageRuntime>,
    request: ActivateProfileRequestV1,
) -> Result<ProfileSnapshotV1, String> {
    state
        .storage()?
        .activate_profile(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn profile_get_active(
    state: State<'_, ProfileStorageRuntime>,
) -> Result<Option<ProfileSnapshotV1>, String> {
    state
        .storage()?
        .get_active_snapshot()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn profile_list_manual_profiles(
    state: State<'_, ProfileStorageRuntime>,
    request: ListManualProfilesRequestV1,
) -> Result<ListManualProfilesResultV1, String> {
    state
        .storage()?
        .list_manual_profiles(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn profile_merge_fog(
    state: State<'_, ProfileStorageRuntime>,
    request: MergeFogRequestV1,
) -> Result<FogMergeResultV1, String> {
    state
        .storage()?
        .merge_fog(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn profile_replace_local_markers(
    state: State<'_, ProfileStorageRuntime>,
    request: ReplaceLocalMarkersRequestV1,
) -> Result<MarkerReplaceResultV1, String> {
    state
        .storage()?
        .replace_local_markers(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn profile_save_settings(
    state: State<'_, ProfileStorageRuntime>,
    request: SaveProfileSettingsRequestV1,
) -> Result<ProfileSettingsV1, String> {
    state
        .storage()?
        .save_settings(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn profile_accept_telemetry_sequence(
    state: State<'_, ProfileStorageRuntime>,
    request: AcceptTelemetrySequenceRequestV1,
) -> Result<bool, String> {
    state
        .storage()?
        .accept_telemetry_sequence(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn profile_set_active_route(
    state: State<'_, ProfileStorageRuntime>,
    request: SetActiveRouteRequestV1,
) -> Result<Option<RouteV1>, String> {
    state
        .storage()?
        .set_active_route(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn profile_write_trail_batch(
    state: State<'_, ProfileStorageRuntime>,
    request: WriteTrailBatchRequestV1,
) -> Result<TrailWriteResultV1, String> {
    state
        .storage()?
        .write_trail_batch(request)
        .map_err(|error| error.to_string())
}

fn toggle_visibility(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<OverlayState>();
    let window = main_window(app)?;
    let _transition = state
        .transition
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let user_visible = !state.user_visible.load(Ordering::Acquire);
    state.user_visible.store(user_visible, Ordering::Release);

    if user_visible {
        synchronize_with_game_locked(&window, &state, &current_game_poll(), true)
    } else {
        set_effective_visibility(&window, &state, false)
    }
}

fn start_game_window_tracker(app: AppHandle) -> Result<(), String> {
    thread::Builder::new()
        .name("scrapmap-game-window".to_owned())
        .spawn(move || {
            let mut tracker = GameWindowTracker::new();
            let mut last_error = None;

            loop {
                let Some(window) = app.get_webview_window("main") else {
                    break;
                };
                let state = app.state::<OverlayState>();
                let poll = tracker.poll();

                match synchronize_with_game(&window, &state, &poll, false) {
                    Ok(()) => last_error = None,
                    Err(error) if last_error.as_ref() != Some(&error) => {
                        eprintln!("ScrapMap overlay tracker paused: {error}");
                        last_error = Some(error);
                    }
                    Err(_) => {}
                }

                let poll_interval = if poll.game.is_some() {
                    GAME_WINDOW_POLL_INTERVAL
                } else {
                    MISSING_GAME_POLL_INTERVAL
                };
                thread::sleep(poll_interval);
            }
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn diagnostic_status_for(issue: DiagnosticIssue, has_snapshot: bool) -> DiagnosticStatusEvent {
    let state = match issue.kind {
        DiagnosticIssueKind::Unsupported => DiagnosticState::Unsupported,
        DiagnosticIssueKind::InvalidPayload | DiagnosticIssueKind::TooLarge => {
            DiagnosticState::Invalid
        }
        DiagnosticIssueKind::Busy
        | DiagnosticIssueKind::InvalidJson
        | DiagnosticIssueKind::Io
        | DiagnosticIssueKind::Missing
            if has_snapshot =>
        {
            DiagnosticState::Stale
        }
        DiagnosticIssueKind::InvalidJson => DiagnosticState::Invalid,
        DiagnosticIssueKind::Busy | DiagnosticIssueKind::Io | DiagnosticIssueKind::Missing => {
            DiagnosticState::Waiting
        }
    };
    DiagnosticStatusEvent {
        state,
        issue: Some(issue),
    }
}

fn publish_diagnostic_status(
    window: &WebviewWindow,
    runtime: &DiagnosticRuntime,
    status: DiagnosticStatusEvent,
) -> Result<(), String> {
    let mut previous = runtime
        .status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if previous.as_ref() == Some(&status) {
        return Ok(());
    }
    window
        .emit("scrapmap:diagnostic-status", status.clone())
        .map_err(|error| error.to_string())?;
    *previous = Some(status);
    Ok(())
}

fn diagnostic_snapshot_is_stale(runtime: &DiagnosticRuntime) -> bool {
    let stale_after = runtime
        .latest
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .map(|snapshot| Duration::from_millis(snapshot.stale_after_ms));
    let received = *runtime
        .last_received
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    snapshot_is_stale(received, stale_after, Instant::now())
}

fn snapshot_is_stale(
    received: Option<Instant>,
    stale_after: Option<Duration>,
    now: Instant,
) -> bool {
    received
        .zip(stale_after)
        .is_some_and(|(received, stale_after)| {
            now.saturating_duration_since(received) > stale_after
        })
}

fn start_diagnostic_source(app: AppHandle) -> Result<(), String> {
    thread::Builder::new()
        .name("scrapmap-diagnostic-source".to_owned())
        .spawn(move || {
            let mut source = match DiagnosticSource::from_process() {
                Ok(source) => source,
                Err(error) => {
                    if let Some(window) = app.get_webview_window("main") {
                        let runtime = app.state::<DiagnosticRuntime>();
                        let _ = publish_diagnostic_status(
                            &window,
                            &runtime,
                            DiagnosticStatusEvent {
                                state: DiagnosticState::Invalid,
                                issue: Some(DiagnosticIssue {
                                    kind: DiagnosticIssueKind::Io,
                                    detail: error.to_string(),
                                }),
                            },
                        );
                    }
                    return;
                }
            };
            let mut game_log_source = GameLogSource::default();
            let mut shot_watcher = poi_capture::ShotWatcher::default();
            let mut photographed: Vec<String> = Vec::new();
            let mut last_error = None;

            loop {
                let Some(window) = app.get_webview_window("main") else {
                    break;
                };
                let runtime = app.state::<DiagnosticRuntime>();

                let process_id = app
                    .state::<OverlayState>()
                    .game_process_id
                    .load(Ordering::Acquire);
                game_log_source.set_root(
                    (process_id != 0)
                        .then(|| game_log_directory(process_id))
                        .flatten(),
                );
                // A POI sweep announces each pose in the same log, and holds it
                // long enough for the capture to land inside the dwell.
                if let Some(directory) = (process_id != 0)
                    .then(|| game_log_directory(process_id))
                    .flatten()
                {
                    let window = app
                        .state::<OverlayState>()
                        .game_window_handle
                        .load(Ordering::Acquire) as isize;
                    for event in shot_watcher.poll(&directory) {
                        match event {
                            poi_capture::ShotEvent::Ready { uuid, crop, .. } if window != 0 => {
                                // Let the scene finish streaming before looking.
                                thread::sleep(Duration::from_millis(1_200));
                                match atlas_bake::atlas_root()
                                    .ok_or_else(|| "LOCALAPPDATA is not set".to_owned())
                                    .and_then(|root| {
                                        poi_capture::capture_tile(window, &uuid, crop, &root)
                                    }) {
                                    Ok(_) => {
                                        // Published now rather than at the end:
                                        // a sweep that is interrupted should
                                        // still leave behind what it captured.
                                        if let Some(root) = atlas_bake::atlas_root() {
                                            if let Err(error) = poi_capture::publish_photos(
                                                &root,
                                                std::slice::from_ref(&uuid),
                                            ) {
                                                eprintln!(
                                                    "ScrapMap could not publish {uuid}: {error}"
                                                );
                                            }
                                        }
                                        photographed.push(uuid);
                                    }
                                    Err(error) => {
                                        eprintln!("ScrapMap could not photograph {uuid}: {error}")
                                    }
                                }
                            }
                            poi_capture::ShotEvent::Ready { .. } => {}
                            poi_capture::ShotEvent::Finished => {
                                if let Some(root) = atlas_bake::atlas_root() {
                                    if let Err(error) =
                                        poi_capture::publish_photos(&root, &photographed)
                                    {
                                        eprintln!("ScrapMap could not publish photos: {error}");
                                    }
                                }
                                if let Some(game_root) = directory.parent() {
                                    poi_capture::clear_request(game_root);
                                }
                                println!(
                                    "ScrapMap photographed {} points of interest",
                                    photographed.len()
                                );
                                photographed.clear();
                            }
                        }
                    }
                }

                let game_log_snapshot = game_log_source.poll().map(Box::new);
                let poll = game_log_snapshot
                    .map(DiagnosticPoll::Updated)
                    .unwrap_or_else(|| source.poll());

                let result = match poll {
                    DiagnosticPoll::Updated(snapshot) => {
                        *runtime
                            .latest
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                            Some((*snapshot).clone());
                        *runtime
                            .last_received
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                            Some(Instant::now());
                        window
                            .emit("scrapmap:telemetry", snapshot)
                            .map_err(|error| error.to_string())
                            .and_then(|_| {
                                publish_diagnostic_status(
                                    &window,
                                    &runtime,
                                    DiagnosticStatusEvent {
                                        state: DiagnosticState::Active,
                                        issue: None,
                                    },
                                )
                            })
                    }
                    DiagnosticPoll::Unchanged if diagnostic_snapshot_is_stale(&runtime) => {
                        publish_diagnostic_status(
                            &window,
                            &runtime,
                            DiagnosticStatusEvent {
                                state: DiagnosticState::Stale,
                                issue: None,
                            },
                        )
                    }
                    DiagnosticPoll::Unchanged => Ok(()),
                    DiagnosticPoll::Waiting(issue)
                        if issue.kind == DiagnosticIssueKind::Unsupported =>
                    {
                        *runtime
                            .latest
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                        *runtime
                            .last_received
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                        publish_diagnostic_status(
                            &window,
                            &runtime,
                            diagnostic_status_for(issue, false),
                        )
                    }
                    DiagnosticPoll::Waiting(issue) => {
                        let has_snapshot = runtime
                            .latest
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .is_some();
                        publish_diagnostic_status(
                            &window,
                            &runtime,
                            diagnostic_status_for(issue, has_snapshot),
                        )
                    }
                };

                match result {
                    Ok(()) => last_error = None,
                    Err(error) if last_error.as_ref() != Some(&error) => {
                        eprintln!("ScrapMap diagnostic channel paused: {error}");
                        last_error = Some(error);
                    }
                    Err(_) => {}
                }

                thread::sleep(DEFAULT_POLL_INTERVAL);
            }
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shortcut_plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_shortcuts(["ctrl+shift+m", "ctrl+shift+h", "ctrl+shift+q"])
        .expect("failed to register ScrapMap shortcut definitions")
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }

            if shortcut.matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyM) {
                let state = app.state::<OverlayState>();
                let expanded = !state.expanded.load(Ordering::Acquire);
                let _ = apply_overlay_mode(app, &state, expanded);
            } else if shortcut.matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyH) {
                let _ = toggle_visibility(app);
            } else if shortcut.matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyQ) {
                app.exit(0);
            }
        })
        .build();

    tauri::Builder::default()
        .manage(OverlayState::default())
        .manage(DiagnosticRuntime::default())
        .manage(GameLayoutRuntime::default())
        .manage(ProfileStorageRuntime::open_default())
        .plugin(shortcut_plugin)
        .setup(|app| {
            let window = app
                .get_webview_window("main")
                .ok_or("main overlay window is unavailable")?;
            configure_overlay_mode(&window, false)?;
            start_game_window_tracker(app.handle().clone())?;
            start_diagnostic_source(app.handle().clone())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            set_overlay_mode,
            set_mini_overlay_layout,
            overlay_status,
            diagnostic_snapshot,
            diagnostic_status,
            game_layout_snapshot,
            atlas_bake_refresh,
            poi_capture_prepare,
            server_identity_probe,
            game_build_probe,
            native_transport_probe,
            profile_activate,
            profile_get_active,
            profile_list_manual_profiles,
            profile_merge_fog,
            profile_replace_local_markers,
            profile_save_settings,
            profile_accept_telemetry_sequence,
            profile_set_active_route,
            profile_write_trail_batch,
            atlas_manifest,
            atlas_preview
        ])
        .run(tauri::generate_context!())
        .expect("ScrapMap failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_freshness_uses_monotonic_receive_time() {
        let received = Instant::now();
        let timeout = Duration::from_millis(500);

        assert!(!snapshot_is_stale(
            Some(received),
            Some(timeout),
            received + timeout,
        ));
        assert!(snapshot_is_stale(
            Some(received),
            Some(timeout),
            received + timeout + Duration::from_millis(1),
        ));
        assert!(!snapshot_is_stale(None, Some(timeout), received + timeout));
        assert!(!snapshot_is_stale(Some(received), None, received + timeout));
    }

    #[test]
    fn unsupported_diagnostic_issue_is_fail_closed() {
        let status = diagnostic_status_for(
            DiagnosticIssue {
                kind: DiagnosticIssueKind::Unsupported,
                detail: "synthetic unsupported build".to_owned(),
            },
            true,
        );
        assert_eq!(status.state, DiagnosticState::Unsupported);
    }
}
