//! Probes whether the Scrap Mechanic window can be captured without a hook.
//!
//! POI photography needs a frame of the game's client area. GDI `BitBlt`
//! typically returns black on a DirectX-rendered window, and `PrintWindow` may
//! or may not depending on how the swap chain presents, so this measures rather
//! than assumes: it tries each method and reports how much of the result is
//! actually non-black, writing the frames out for inspection.
//!
//!   cargo run --example capture_probe -- <output-directory>

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("capture_probe is Windows-only");
}

#[cfg(target_os = "windows")]
fn main() {
    use std::{env, ffi::c_void, path::PathBuf};

    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, RECT, TRUE},
            Graphics::Gdi::{
                BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
                GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
                BI_RGB, DIB_RGB_COLORS, HBITMAP, SRCCOPY,
            },
            Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS},
            UI::WindowsAndMessaging::{
                EnumWindows, GetClientRect, GetWindowTextW, GetWindowThreadProcessId,
            },
        },
    };

    /// Undocumented but widely used: render the full window content, which is
    /// what makes PrintWindow work for many hardware-accelerated windows.
    const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(0x00000002);

    struct Found(HWND);
    static mut TARGET: isize = 0;

    unsafe extern "system" fn visit(handle: HWND, _: LPARAM) -> windows::core::BOOL {
        let mut text = [0_u16; 256];
        let length = unsafe { GetWindowTextW(handle, &mut text) };
        if length > 0 {
            let title = String::from_utf16_lossy(&text[..length as usize]);
            if title.trim() == "Scrap Mechanic" {
                let mut pid = 0_u32;
                unsafe { GetWindowThreadProcessId(handle, Some(&mut pid)) };
                if pid != 0 {
                    unsafe { TARGET = handle.0 as isize };
                    return windows::core::BOOL(0); // stop enumerating
                }
            }
        }
        TRUE
    }

    fn find_game() -> Option<Found> {
        unsafe {
            TARGET = 0;
            let _ = EnumWindows(Some(visit), LPARAM(0));
            (TARGET != 0).then(|| Found(HWND(TARGET as *mut c_void)))
        }
    }

    /// Captures the client area, returning BGRA rows top-down.
    fn capture(window: HWND, use_print_window: bool) -> Option<(u32, u32, Vec<u8>)> {
        unsafe {
            let mut client = RECT::default();
            GetClientRect(window, &mut client).ok()?;
            let width = (client.right - client.left).max(0) as u32;
            let height = (client.bottom - client.top).max(0) as u32;
            if width == 0 || height == 0 {
                return None;
            }

            let window_dc = GetDC(Some(window));
            if window_dc.is_invalid() {
                return None;
            }
            let memory_dc = CreateCompatibleDC(Some(window_dc));
            let bitmap = CreateCompatibleBitmap(window_dc, width as i32, height as i32);
            let previous = SelectObject(memory_dc, bitmap.into());

            let ok = if use_print_window {
                PrintWindow(window, memory_dc, PW_RENDERFULLCONTENT).as_bool()
            } else {
                BitBlt(
                    memory_dc,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    Some(window_dc),
                    0,
                    0,
                    SRCCOPY,
                )
                .is_ok()
            };

            let mut pixels = vec![0_u8; (width * height * 4) as usize];
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    // Negative height gives a top-down image.
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let scanned = GetDIBits(
                memory_dc,
                bitmap,
                0,
                height,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            );

            SelectObject(memory_dc, previous);
            let _ = DeleteObject(HBITMAP(bitmap.0).into());
            let _ = DeleteDC(memory_dc);
            ReleaseDC(Some(window), window_dc);

            (ok && scanned != 0).then_some((width, height, pixels))
        }
    }

    /// A DirectX window that refuses to be captured comes back uniformly black,
    /// so the useful measure is how many pixels carry any signal at all.
    fn describe(pixels: &[u8]) -> (f32, f32) {
        let mut lit = 0_u64;
        let mut total = 0_u64;
        let mut sum = 0_u64;
        for chunk in pixels.chunks_exact(4) {
            let brightness = u64::from(chunk[0]) + u64::from(chunk[1]) + u64::from(chunk[2]);
            if brightness > 24 {
                lit += 1;
            }
            sum += brightness;
            total += 1;
        }
        let total = total.max(1) as f32;
        (lit as f32 / total * 100.0, sum as f32 / total / 3.0)
    }

    fn write_png(path: &PathBuf, width: u32, height: u32, bgra: &[u8]) {
        let mut rgba = Vec::with_capacity(bgra.len());
        for chunk in bgra.chunks_exact(4) {
            rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], 255]);
        }
        let file = std::fs::File::create(path).expect("create png");
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .expect("png header")
            .write_image_data(&rgba)
            .expect("png data");
    }

    let output = PathBuf::from(
        env::args()
            .nth(1)
            .unwrap_or_else(|| env::temp_dir().to_string_lossy().into_owned()),
    );

    let Some(Found(window)) = find_game() else {
        eprintln!("Scrap Mechanic window not found -- is the game running?");
        return;
    };
    println!("found game window {:?}", window.0);

    for (label, use_print_window) in [("printwindow", true), ("bitblt", false)] {
        match capture(window, use_print_window) {
            Some((width, height, pixels)) => {
                let (lit, mean) = describe(&pixels);
                let path = output.join(format!("capture_{label}.png"));
                write_png(&path, width, height, &pixels);
                println!(
                    "{label:<12} {width}x{height}  non-black {lit:.1}%  mean brightness {mean:.1}  -> {}",
                    path.display()
                );
            }
            None => println!("{label:<12} failed"),
        }
    }
}
