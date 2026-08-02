//! Captures the game window's client area.
//!
//! `PrintWindow` with `PW_RENDERFULLCONTENT` is what works here: plain `BitBlt`
//! returns a uniformly black buffer because the window is DirectX-presented.
//! That is measured, not assumed -- see `examples/capture_probe.rs`, which is
//! the quickest way to re-check after a game or driver update.
//!
//! No hook and no injection: this reads the window the compositor already has.

#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA, top-down.
    pub pixels: Vec<u8>,
}

impl Frame {
    /// Cuts a centred square of `side` pixels.
    ///
    /// The camera frames a tile to the full height of the client area, so the
    /// centred square of that height covers exactly the tile: pixels are square,
    /// so the same pixel count spans the same distance on both axes.
    pub fn centre_square(&self, side: u32) -> Option<Frame> {
        let side = side.min(self.width).min(self.height);
        if side == 0 {
            return None;
        }
        let left = (self.width - side) / 2;
        let top = (self.height - side) / 2;
        let mut pixels = Vec::with_capacity((side * side * 4) as usize);
        for row in 0..side {
            let start = (((top + row) * self.width + left) * 4) as usize;
            let end = start + (side * 4) as usize;
            pixels.extend_from_slice(self.pixels.get(start..end)?);
        }
        Some(Frame {
            width: side,
            height: side,
            pixels,
        })
    }

    /// Rescales with a box filter. Tiles are captured far larger than they are
    /// drawn, so averaging avoids the aliasing a nearest-neighbour pick gives.
    pub fn resize(&self, target: u32) -> Option<Frame> {
        if target == 0 || self.width == 0 || self.height == 0 {
            return None;
        }
        let mut pixels = vec![0_u8; (target * target * 4) as usize];
        for y in 0..target {
            let y0 = y * self.height / target;
            let y1 = (((y + 1) * self.height).div_ceil(target)).max(y0 + 1).min(self.height);
            for x in 0..target {
                let x0 = x * self.width / target;
                let x1 = (((x + 1) * self.width).div_ceil(target)).max(x0 + 1).min(self.width);
                let mut sum = [0_u32; 3];
                let mut count = 0_u32;
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let offset = ((sy * self.width + sx) * 4) as usize;
                        sum[0] += u32::from(self.pixels[offset]);
                        sum[1] += u32::from(self.pixels[offset + 1]);
                        sum[2] += u32::from(self.pixels[offset + 2]);
                        count += 1;
                    }
                }
                let count = count.max(1);
                let out = ((y * target + x) * 4) as usize;
                pixels[out] = (sum[0] / count) as u8;
                pixels[out + 1] = (sum[1] / count) as u8;
                pixels[out + 2] = (sum[2] / count) as u8;
                pixels[out + 3] = 255;
            }
        }
        Some(Frame {
            width: target,
            height: target,
            pixels,
        })
    }

    /// Mean absolute deviation of luminance, as a rough measure of detail.
    ///
    /// Brightness alone cannot tell terrain from a wall of fog: a camera stuck
    /// inside a hill or above the clouds returns an evenly lit frame that is
    /// perfectly bright and completely useless. Real terrain has contrast.
    pub fn detail(&self) -> f32 {
        if self.pixels.len() < 4 {
            return 0.0;
        }
        let luma = |chunk: &[u8]| {
            (0.299 * f32::from(chunk[0]) + 0.587 * f32::from(chunk[1]) + 0.114 * f32::from(chunk[2]))
        };
        let mut sum = 0.0_f64;
        let mut count = 0_u32;
        for chunk in self.pixels.chunks_exact(4) {
            sum += f64::from(luma(chunk));
            count += 1;
        }
        let mean = (sum / f64::from(count.max(1))) as f32;
        let mut deviation = 0.0_f64;
        for chunk in self.pixels.chunks_exact(4) {
            deviation += f64::from((luma(chunk) - mean).abs());
        }
        (deviation / f64::from(count.max(1))) as f32
    }

    /// Share of pixels carrying any signal. A window that refuses to be
    /// captured comes back uniformly black, so this distinguishes a real frame
    /// from a failed one better than the API's own success return does.
    pub fn lit_fraction(&self) -> f32 {
        if self.pixels.is_empty() {
            return 0.0;
        }
        let mut lit = 0_u64;
        let mut total = 0_u64;
        for chunk in self.pixels.chunks_exact(4) {
            if u16::from(chunk[0]) + u16::from(chunk[1]) + u16::from(chunk[2]) > 24 {
                lit += 1;
            }
            total += 1;
        }
        lit as f32 / total.max(1) as f32
    }
}

#[cfg(target_os = "windows")]
pub use platform::capture_window;

#[cfg(target_os = "windows")]
mod platform {
    use super::Frame;
    use windows::Win32::{
        Foundation::{HWND, RECT},
        Graphics::Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
            ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
        },
        Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS},
        UI::WindowsAndMessaging::GetClientRect,
    };

    /// Undocumented but the reason this works on hardware-accelerated windows.
    const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(0x0000_0002);
    /// Guards against a bogus client rect turning into a huge allocation.
    const MAX_DIMENSION: u32 = 16_384;

    pub fn capture_window(handle: isize) -> Result<Frame, String> {
        unsafe {
            let window = HWND(handle as *mut std::ffi::c_void);
            let mut client = RECT::default();
            GetClientRect(window, &mut client).map_err(|error| error.to_string())?;
            let width = (client.right - client.left).max(0) as u32;
            let height = (client.bottom - client.top).max(0) as u32;
            if width == 0 || height == 0 {
                return Err("game window has no client area".to_owned());
            }
            if width > MAX_DIMENSION || height > MAX_DIMENSION {
                return Err(format!("implausible client area {width}x{height}"));
            }

            let window_dc = GetDC(Some(window));
            if window_dc.is_invalid() {
                return Err("could not obtain a device context".to_owned());
            }
            let memory_dc = CreateCompatibleDC(Some(window_dc));
            let bitmap = CreateCompatibleBitmap(window_dc, width as i32, height as i32);
            let previous = SelectObject(memory_dc, bitmap.into());

            let printed = PrintWindow(window, memory_dc, PW_RENDERFULLCONTENT).as_bool();

            let mut pixels = vec![0_u8; (width * height * 4) as usize];
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    // Negative height asks for a top-down image.
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

            if !printed || scanned == 0 {
                return Err("the window declined to render into the capture".to_owned());
            }

            // GDI hands back BGRA; the rest of the pipeline speaks RGBA.
            for chunk in pixels.chunks_exact_mut(4) {
                chunk.swap(0, 2);
                chunk[3] = 255;
            }
            Ok(Frame {
                width,
                height,
                pixels,
            })
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn capture_window(_handle: isize) -> Result<Frame, String> {
    Err("window capture is Windows-only".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, fill: impl Fn(u32, u32) -> [u8; 4]) -> Frame {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&fill(x, y));
            }
        }
        Frame {
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn centre_square_takes_the_middle_of_a_wide_frame() {
        // Columns numbered by x so the crop position is checkable.
        let wide = frame(9, 3, |x, _| [x as u8, 0, 0, 255]);
        let square = wide.centre_square(3).unwrap();
        assert_eq!((square.width, square.height), (3, 3));
        // A 9-wide frame cropped to 3 starts at column 3.
        assert_eq!(square.pixels[0], 3);
        assert_eq!(square.pixels[4], 4);
        assert_eq!(square.pixels[8], 5);
    }

    #[test]
    fn centre_square_is_bounded_by_the_frame() {
        let small = frame(4, 2, |_, _| [1, 2, 3, 255]);
        // Asking for more than the frame holds clamps to the short side.
        let square = small.centre_square(64).unwrap();
        assert_eq!((square.width, square.height), (2, 2));
    }

    #[test]
    fn resize_averages_rather_than_dropping_pixels() {
        // Left half black, right half white: a box filter gives mid grey when
        // collapsed to a single pixel, whereas point sampling gives one or the
        // other.
        let split = frame(4, 4, |x, _| {
            if x < 2 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let one = split.resize(1).unwrap();
        assert!(
            (120..=136).contains(&one.pixels[0]),
            "expected mid grey, got {}",
            one.pixels[0]
        );
    }

    #[test]
    fn resize_preserves_a_flat_colour() {
        let flat = frame(8, 8, |_, _| [10, 200, 30, 255]);
        let small = flat.resize(4).unwrap();
        assert_eq!((small.width, small.height), (4, 4));
        assert_eq!(&small.pixels[0..4], &[10, 200, 30, 255]);
        assert_eq!(&small.pixels[small.pixels.len() - 4..], &[10, 200, 30, 255]);
    }

    #[test]
    fn detail_separates_terrain_from_a_wall_of_fog() {
        // A camera inside a hill or above the clouds returns an evenly lit
        // sheet. It is bright, so brightness alone accepts it; it carries no
        // detail, which is what actually distinguishes it from terrain.
        let fog = frame(16, 16, |_, _| [188, 190, 196, 255]);
        assert!(fog.lit_fraction() > 0.9, "fog is bright, hence the problem");
        assert!(fog.detail() < 1.0, "fog should read as featureless");

        let terrain = frame(16, 16, |x, y| {
            let value = ((x * 37 + y * 61) % 200) as u8;
            [value, value / 2 + 40, 60, 255]
        });
        assert!(
            terrain.detail() > 10.0,
            "terrain should carry contrast, got {}",
            terrain.detail()
        );
    }

    #[test]
    fn lit_fraction_separates_a_black_capture_from_a_real_one() {
        let black = frame(4, 4, |_, _| [0, 0, 0, 255]);
        assert_eq!(black.lit_fraction(), 0.0);
        let real = frame(4, 4, |_, _| [90, 120, 60, 255]);
        assert_eq!(real.lit_fraction(), 1.0);
        // Half-lit sits in between, which is what a partly-loaded scene looks like.
        let half = frame(4, 4, |_, y| if y < 2 { [0, 0, 0, 255] } else { [80, 80, 80, 255] });
        assert!((half.lit_fraction() - 0.5).abs() < 0.001);
    }
}
