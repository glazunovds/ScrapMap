//! Discovery and geometry helpers for attaching the overlay to Scrap Mechanic.
//!
//! The Win32-facing API intentionally returns small, Tauri-independent value
//! types. The caller can poll [`GameWindowTracker::poll`] on a worker thread and
//! apply the resulting physical coordinates on Tauri's event loop.

pub const SCRAP_MECHANIC_PROCESS_NAME: &str = "ScrapMechanic.exe";
pub const SCRAP_MECHANIC_WINDOW_TITLE: &str = "Scrap Mechanic";
pub const DEFAULT_DPI: u32 = 96;
pub const DEFAULT_MINI_LOGICAL_SIZE: u32 = 420;
pub const DEFAULT_MINI_LOGICAL_MARGIN: u32 = 24;

/// An opaque native top-level window handle.
///
/// Keeping the raw handle in a plain integer makes snapshots safe to move from
/// a polling thread to the Tauri event loop. It must always be revalidated
/// before being passed back to Win32.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NativeWindowHandle(pub isize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowIdentity {
    pub handle: NativeWindowHandle,
    pub process_id: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ForegroundWindow {
    pub identity: Option<WindowIdentity>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ScreenRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlayGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl From<ScreenRect> for OverlayGeometry {
    fn from(rect: ScreenRect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowMatchKind {
    ProcessImage,
    WindowTitleFallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GameWindowSnapshot {
    pub identity: WindowIdentity,
    pub client_rect: Option<ScreenRect>,
    pub dpi: u32,
    pub visible: bool,
    pub minimized: bool,
    /// True when the foreground window belongs to the game process.
    pub foreground: bool,
    pub match_kind: WindowMatchKind,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GameWindowPoll {
    /// `None` is the normal, safe result while the game is closed or restarting.
    pub game: Option<GameWindowSnapshot>,
    pub foreground: ForegroundWindow,
}

/// Pure foreground policy used by the overlay visibility controller.
///
/// The overlay is eligible to be shown only while the game has a usable,
/// visible client area and either the game or the overlay itself owns the
/// foreground window. `user_visible` remains a separate preference so an
/// automatic hide does not undo Ctrl+Shift+H.
pub fn overlay_should_be_present(
    user_visible: bool,
    game: Option<&GameWindowSnapshot>,
    foreground: ForegroundWindow,
    overlay: Option<WindowIdentity>,
) -> bool {
    if !user_visible {
        return false;
    }

    let Some(game) = game else {
        return false;
    };

    let has_client_area = game
        .client_rect
        .is_some_and(|client_rect| !client_rect.is_empty());
    if !game.visible || game.minimized || !has_client_area {
        return false;
    }

    foreground_matches(foreground, game.identity)
        || overlay.is_some_and(|identity| foreground_matches(foreground, identity))
}

fn foreground_matches(foreground: ForegroundWindow, target: WindowIdentity) -> bool {
    foreground.identity.is_some_and(|identity| {
        identity.handle == target.handle
            || (identity.process_id != 0 && identity.process_id == target.process_id)
    })
}

/// Computes a square mini-map anchored to the top-right of the game client.
///
/// All returned values are physical screen pixels. Logical size and margin are
/// scaled using the game window's DPI and clamped for very small window sizes.
pub fn mini_overlay_geometry(
    client: ScreenRect,
    dpi: u32,
    logical_size: u32,
    logical_margin: u32,
    corner: MiniCorner,
) -> Option<OverlayGeometry> {
    if client.is_empty() {
        return None;
    }

    let dpi = dpi.max(1);
    let desired_size = scale_logical_pixels(logical_size, dpi).max(1);
    let desired_margin = scale_logical_pixels(logical_margin, dpi);
    let margin = desired_margin
        .min(client.width.saturating_sub(1) / 2)
        .min(client.height.saturating_sub(1) / 2);
    let available_width = client.width.saturating_sub(margin.saturating_mul(2));
    let available_height = client.height.saturating_sub(margin.saturating_mul(2));
    let size = desired_size.min(available_width).min(available_height);

    if size == 0 {
        return None;
    }

    let far_x = client.width.saturating_sub(margin).saturating_sub(size);
    let far_y = client.height.saturating_sub(margin).saturating_sub(size);
    let (x_offset, y_offset) = match corner {
        MiniCorner::TopLeft => (margin, margin),
        MiniCorner::TopRight => (far_x, margin),
        MiniCorner::BottomLeft => (margin, far_y),
        MiniCorner::BottomRight => (far_x, far_y),
    };
    Some(OverlayGeometry {
        x: saturating_add_unsigned(client.x, x_offset),
        y: saturating_add_unsigned(client.y, y_offset),
        width: size,
        height: size,
    })
}

/// Which corner of the game window the compact map sits in.
///
/// Scrap Mechanic puts its quest tracker down the right-hand side, so the
/// default overlaps it on smaller windows; moving the map is usually better
/// than shrinking it to fit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MiniCorner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

impl MiniCorner {
    pub fn from_code(code: u32) -> Self {
        match code {
            0 => Self::TopLeft,
            2 => Self::BottomLeft,
            3 => Self::BottomRight,
            _ => Self::TopRight,
        }
    }
}

pub fn default_mini_overlay_geometry(client: ScreenRect, dpi: u32) -> Option<OverlayGeometry> {
    mini_overlay_geometry(
        client,
        dpi,
        DEFAULT_MINI_LOGICAL_SIZE,
        DEFAULT_MINI_LOGICAL_MARGIN,
        MiniCorner::default(),
    )
}

/// Full-map mode occupies the exact physical client area of the game.
pub fn full_overlay_geometry(client: ScreenRect) -> Option<OverlayGeometry> {
    (!client.is_empty()).then(|| client.into())
}

fn scale_logical_pixels(value: u32, dpi: u32) -> u32 {
    let scaled = u64::from(value)
        .saturating_mul(u64::from(dpi))
        .saturating_add(u64::from(DEFAULT_DPI / 2))
        / u64::from(DEFAULT_DPI);
    scaled.min(u64::from(u32::MAX)) as u32
}

fn saturating_add_unsigned(origin: i32, offset: u32) -> i32 {
    (i64::from(origin) + i64::from(offset)).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[derive(Debug, Default)]
pub struct GameWindowTracker {
    cached_handle: Option<NativeWindowHandle>,
}

impl GameWindowTracker {
    pub const fn new() -> Self {
        Self {
            cached_handle: None,
        }
    }

    /// Polls current game and foreground state.
    ///
    /// This call is non-blocking apart from short Win32 process queries during
    /// discovery. A cached HWND avoids enumeration on normal 150-250 ms polls.
    /// Absence, shutdown and relaunch races are represented as `game: None`.
    pub fn poll(&mut self) -> GameWindowPoll {
        platform::poll(self)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::cmp::Ordering;
    use std::ffi::c_void;

    use windows::{
        core::BOOL,
        Win32::{
            Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT},
            Graphics::Gdi::ClientToScreen,
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
            UI::{
                HiDpi::GetDpiForWindow,
                WindowsAndMessaging::{
                    EnumWindows, GetClientRect, GetForegroundWindow, GetWindowTextLengthW,
                    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
                },
            },
        },
    };

    use super::{
        ForegroundWindow, GameWindowPoll, GameWindowSnapshot, GameWindowTracker,
        NativeWindowHandle, ScreenRect, WindowIdentity, WindowMatchKind, DEFAULT_DPI,
        SCRAP_MECHANIC_PROCESS_NAME, SCRAP_MECHANIC_WINDOW_TITLE,
    };

    #[derive(Clone, Copy, Debug)]
    struct Candidate {
        handle: NativeWindowHandle,
        process_id: u32,
        match_kind: WindowMatchKind,
        visible: bool,
        minimized: bool,
        client_rect: Option<ScreenRect>,
        title_matches: bool,
    }

    impl Candidate {
        fn rank(self) -> (u8, u8, u8, u8, u64) {
            (
                u8::from(self.match_kind == WindowMatchKind::ProcessImage),
                u8::from(self.visible),
                u8::from(!self.minimized),
                u8::from(self.title_matches),
                self.client_rect
                    .map(|rect| u64::from(rect.width) * u64::from(rect.height))
                    .unwrap_or(0),
            )
        }
    }

    pub(super) fn poll(tracker: &mut GameWindowTracker) -> GameWindowPoll {
        let foreground = foreground_window();

        let cached_candidate = tracker.cached_handle.and_then(candidate_from_handle);
        let candidate = match cached_candidate {
            Some(candidate) if candidate_is_usable(candidate) => Some(candidate),
            candidate => find_game_window().or(candidate),
        };

        let Some(candidate) = candidate else {
            tracker.cached_handle = None;
            return GameWindowPoll {
                game: None,
                foreground,
            };
        };

        tracker.cached_handle = Some(candidate.handle);
        let identity = WindowIdentity {
            handle: candidate.handle,
            process_id: candidate.process_id,
        };

        GameWindowPoll {
            game: Some(GameWindowSnapshot {
                identity,
                client_rect: candidate.client_rect,
                dpi: window_dpi(candidate.handle),
                visible: candidate.visible,
                minimized: candidate.minimized,
                foreground: super::foreground_matches(foreground, identity),
                match_kind: candidate.match_kind,
            }),
            foreground,
        }
    }

    fn candidate_is_usable(candidate: Candidate) -> bool {
        candidate.visible
            && !candidate.minimized
            && candidate
                .client_rect
                .is_some_and(|client_rect| !client_rect.is_empty())
    }

    fn find_game_window() -> Option<Candidate> {
        let mut handles = Vec::<NativeWindowHandle>::new();
        let handles_pointer = (&mut handles as *mut Vec<NativeWindowHandle>) as isize;

        // SAFETY: `handles_pointer` remains valid and exclusively borrowed for
        // the synchronous duration of EnumWindows. The callback only appends
        // integer copies of HWND values.
        if unsafe { EnumWindows(Some(collect_window), LPARAM(handles_pointer)) }.is_err() {
            return None;
        }

        handles
            .into_iter()
            .filter_map(candidate_from_handle)
            .max_by(|left, right| compare_candidates(*left, *right))
    }

    fn compare_candidates(left: Candidate, right: Candidate) -> Ordering {
        left.rank().cmp(&right.rank())
    }

    unsafe extern "system" fn collect_window(hwnd: HWND, state: LPARAM) -> BOOL {
        // SAFETY: `state` is created from a live `&mut Vec` in
        // `find_game_window`, and EnumWindows invokes callbacks synchronously.
        let handles = unsafe { &mut *(state.0 as *mut Vec<NativeWindowHandle>) };
        handles.push(from_hwnd(hwnd));
        BOOL(1)
    }

    fn candidate_from_handle(handle: NativeWindowHandle) -> Option<Candidate> {
        let hwnd = to_hwnd(handle);
        // SAFETY: IsWindow only probes the opaque value and does not retain it.
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return None;
        }

        let process_id = window_process_id(hwnd)?;
        let title = window_title(hwnd);
        let title_matches = title.as_deref().is_some_and(|title| {
            title
                .trim()
                .eq_ignore_ascii_case(SCRAP_MECHANIC_WINDOW_TITLE)
        });
        let process_matches = process_image_name(process_id)
            .is_some_and(|name| name.eq_ignore_ascii_case(SCRAP_MECHANIC_PROCESS_NAME));

        let match_kind = if process_matches {
            WindowMatchKind::ProcessImage
        } else if title_matches {
            WindowMatchKind::WindowTitleFallback
        } else {
            return None;
        };

        // SAFETY: The HWND was just validated. Races with window destruction
        // are harmless: these APIs return false/default values.
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        let minimized = unsafe { IsIconic(hwnd) }.as_bool();

        Some(Candidate {
            handle,
            process_id,
            match_kind,
            visible,
            minimized,
            client_rect: client_screen_rect(hwnd),
            title_matches,
        })
    }

    fn foreground_window() -> ForegroundWindow {
        // SAFETY: GetForegroundWindow has no preconditions and returns either a
        // borrowed HWND value or NULL.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return ForegroundWindow::default();
        }

        ForegroundWindow {
            identity: window_process_id(hwnd).map(|process_id| WindowIdentity {
                handle: from_hwnd(hwnd),
                process_id,
            }),
        }
    }

    fn window_process_id(hwnd: HWND) -> Option<u32> {
        let mut process_id = 0;
        // SAFETY: `process_id` is a valid output pointer for this call.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
        (process_id != 0).then_some(process_id)
    }

    fn process_image_name(process_id: u32) -> Option<String> {
        // PROCESS_QUERY_LIMITED_INFORMATION works for an ordinary peer process
        // without requesting write, VM-read or debugger access.
        let process =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
        let mut buffer = vec![0_u16; 32_768];
        let mut length = buffer.len() as u32;
        // SAFETY: The handle is valid and `buffer`/`length` satisfy the API's
        // writable buffer contract.
        let query_result = unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        };
        // SAFETY: `process` is owned by this function and closed exactly once.
        let _ = unsafe { CloseHandle(process) };
        query_result.ok()?;

        let full_path = String::from_utf16_lossy(&buffer[..length as usize]);
        full_path
            .rsplit(['\\', '/'])
            .next()
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
    }

    fn window_title(hwnd: HWND) -> Option<String> {
        // SAFETY: The HWND value is borrowed for the duration of each call.
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length <= 0 {
            return None;
        }

        let mut buffer = vec![0_u16; length as usize + 1];
        // SAFETY: `buffer` includes room for the terminating NUL.
        let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        (copied > 0).then(|| String::from_utf16_lossy(&buffer[..copied as usize]))
    }

    fn client_screen_rect(hwnd: HWND) -> Option<ScreenRect> {
        let mut client = RECT::default();
        // SAFETY: `client` is a valid output pointer.
        unsafe { GetClientRect(hwnd, &mut client) }.ok()?;

        let width = client.right.checked_sub(client.left)?;
        let height = client.bottom.checked_sub(client.top)?;
        if width <= 0 || height <= 0 {
            return None;
        }

        let mut origin = POINT {
            x: client.left,
            y: client.top,
        };
        // SAFETY: `origin` is a valid in/out pointer.
        if !unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
            return None;
        }

        Some(ScreenRect::new(
            origin.x,
            origin.y,
            width as u32,
            height as u32,
        ))
    }

    fn window_dpi(handle: NativeWindowHandle) -> u32 {
        // SAFETY: The handle is only queried and a stale handle yields zero.
        let dpi = unsafe { GetDpiForWindow(to_hwnd(handle)) };
        if dpi == 0 {
            DEFAULT_DPI
        } else {
            dpi
        }
    }

    fn from_hwnd(hwnd: HWND) -> NativeWindowHandle {
        NativeWindowHandle(hwnd.0 as isize)
    }

    fn to_hwnd(handle: NativeWindowHandle) -> HWND {
        HWND(handle.0 as *mut c_void)
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::{GameWindowPoll, GameWindowTracker};

    pub(super) fn poll(tracker: &mut GameWindowTracker) -> GameWindowPoll {
        tracker.cached_handle = None;
        GameWindowPoll::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: WindowIdentity = WindowIdentity {
        handle: NativeWindowHandle(11),
        process_id: 101,
    };
    const OVERLAY: WindowIdentity = WindowIdentity {
        handle: NativeWindowHandle(22),
        process_id: 202,
    };

    fn snapshot() -> GameWindowSnapshot {
        GameWindowSnapshot {
            identity: GAME,
            client_rect: Some(ScreenRect::new(-1_900, -900, 1_600, 900)),
            dpi: DEFAULT_DPI,
            visible: true,
            minimized: false,
            foreground: true,
            match_kind: WindowMatchKind::ProcessImage,
        }
    }

    fn foreground(identity: WindowIdentity) -> ForegroundWindow {
        ForegroundWindow {
            identity: Some(identity),
        }
    }

    #[test]
    fn default_mini_is_scaled_and_anchored_inside_negative_origin_client() {
        let geometry =
            default_mini_overlay_geometry(ScreenRect::new(-1_900, -900, 1_600, 900), 144)
                .expect("valid geometry");

        assert_eq!(
            geometry,
            OverlayGeometry {
                x: -966,
                y: -864,
                width: 630,
                height: 630,
            }
        );
    }

    #[test]
    fn mini_geometry_clamps_to_tiny_game_window() {
        let geometry = mini_overlay_geometry(ScreenRect::new(10, 20, 90, 60), 96, 420, 24, MiniCorner::default())
            .expect("clamped geometry");

        assert_eq!(
            geometry,
            OverlayGeometry {
                x: 64,
                y: 44,
                width: 12,
                height: 12,
            }
        );
    }

    #[test]
    fn each_corner_anchors_inside_the_client_area() {
        // A 1000x800 client at the origin, 100 logical px of map, 20 of margin.
        let client = ScreenRect::new(0, 0, 1_000, 800);
        let place = |corner| {
            mini_overlay_geometry(client, DEFAULT_DPI, 100, 20, corner).expect("geometry")
        };

        assert_eq!(place(MiniCorner::TopLeft).x, 20);
        assert_eq!(place(MiniCorner::TopLeft).y, 20);
        assert_eq!(place(MiniCorner::TopRight).x, 1_000 - 20 - 100);
        assert_eq!(place(MiniCorner::TopRight).y, 20);
        assert_eq!(place(MiniCorner::BottomLeft).x, 20);
        assert_eq!(place(MiniCorner::BottomLeft).y, 800 - 20 - 100);
        assert_eq!(place(MiniCorner::BottomRight).x, 1_000 - 20 - 100);
        assert_eq!(place(MiniCorner::BottomRight).y, 800 - 20 - 100);

        // Every corner must stay fully inside the client area.
        for corner in [
            MiniCorner::TopLeft,
            MiniCorner::TopRight,
            MiniCorner::BottomLeft,
            MiniCorner::BottomRight,
        ] {
            let geometry = place(corner);
            assert!(geometry.x >= 0 && geometry.y >= 0, "{corner:?} escaped");
            assert!(
                geometry.x + geometry.width as i32 <= 1_000
                    && geometry.y + geometry.height as i32 <= 800,
                "{corner:?} overflowed the client area"
            );
        }
    }

    #[test]
    fn corner_codes_round_trip_from_the_frontend() {
        assert_eq!(MiniCorner::from_code(0), MiniCorner::TopLeft);
        assert_eq!(MiniCorner::from_code(1), MiniCorner::TopRight);
        assert_eq!(MiniCorner::from_code(2), MiniCorner::BottomLeft);
        assert_eq!(MiniCorner::from_code(3), MiniCorner::BottomRight);
        // Anything unexpected falls back to the shipped default.
        assert_eq!(MiniCorner::from_code(99), MiniCorner::TopRight);
    }

    #[test]
    fn empty_client_has_no_geometry() {
        assert_eq!(
            default_mini_overlay_geometry(ScreenRect::new(0, 0, 0, 720), 96),
            None
        );
        assert_eq!(full_overlay_geometry(ScreenRect::new(0, 0, 1280, 0)), None);
    }

    #[test]
    fn full_geometry_matches_game_client_exactly() {
        let client = ScreenRect::new(-1_920, 0, 1_920, 1_080);
        assert_eq!(full_overlay_geometry(client), Some(client.into()));
    }

    #[test]
    fn overlay_is_present_when_game_or_overlay_owns_foreground() {
        let game = snapshot();

        assert!(overlay_should_be_present(
            true,
            Some(&game),
            foreground(GAME),
            Some(OVERLAY),
        ));
        assert!(overlay_should_be_present(
            true,
            Some(&game),
            foreground(OVERLAY),
            Some(OVERLAY),
        ));
    }

    #[test]
    fn another_window_from_same_process_counts_as_foreground() {
        let game = snapshot();
        let game_dialog = WindowIdentity {
            handle: NativeWindowHandle(12),
            process_id: GAME.process_id,
        };
        let overlay_child = WindowIdentity {
            handle: NativeWindowHandle(23),
            process_id: OVERLAY.process_id,
        };

        assert!(overlay_should_be_present(
            true,
            Some(&game),
            foreground(game_dialog),
            Some(OVERLAY),
        ));
        assert!(overlay_should_be_present(
            true,
            Some(&game),
            foreground(overlay_child),
            Some(OVERLAY),
        ));
    }

    #[test]
    fn overlay_hides_safely_when_game_is_unusable_or_inactive() {
        let mut game = snapshot();
        let unrelated = WindowIdentity {
            handle: NativeWindowHandle(99),
            process_id: 999,
        };

        assert!(!overlay_should_be_present(
            false,
            Some(&game),
            foreground(GAME),
            Some(OVERLAY),
        ));
        assert!(!overlay_should_be_present(
            true,
            None,
            foreground(GAME),
            Some(OVERLAY),
        ));
        assert!(!overlay_should_be_present(
            true,
            Some(&game),
            foreground(unrelated),
            Some(OVERLAY),
        ));

        game.minimized = true;
        assert!(!overlay_should_be_present(
            true,
            Some(&game),
            foreground(GAME),
            Some(OVERLAY),
        ));
        game.minimized = false;
        game.visible = false;
        assert!(!overlay_should_be_present(
            true,
            Some(&game),
            foreground(GAME),
            Some(OVERLAY),
        ));
        game.visible = true;
        game.client_rect = None;
        assert!(!overlay_should_be_present(
            true,
            Some(&game),
            foreground(GAME),
            Some(OVERLAY),
        ));
    }
}
