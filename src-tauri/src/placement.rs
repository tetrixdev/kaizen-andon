//! Where the window sits.
//!
//! Against the WORK AREA, never the screen. On Windows that is
//! `SPI_GETWORKAREA`, which already excludes the taskbar and any docked
//! appbars, so nothing here subtracts a hardcoded height. The taskbar's size
//! changes with DPI and the small-buttons setting, it can sit on any edge, and
//! it can auto-hide.
//!
//! 24px in from both edges. There is no Microsoft guideline for third-party
//! widgets; the number comes from precedent (Windows' own toasts sit 12 to 16
//! inside the work area, macOS notifications about 20).

/// Margin from the work area's edges to the CARD, in logical pixels.
pub const MARGIN: i32 = 24;

/// The widest the expanded card is ever drawn.
pub const MAX_WIDTH: i32 = 1222;

/// Room inside the window, on every side, for the drop shadow and the
/// call-state ring to fade out.
///
/// The window is sized to its content and the page is transparent, so without
/// this the shadow is sliced off square at the window edge: the card looks
/// like it has grey corners rather than a soft one. It is counted twice into
/// the window's size and subtracted once from the margin, so the CARD still
/// lands `MARGIN` from the edge of the work area.
pub const SHADOW_ROOM: i32 = 20;

/// The margin from the work area to the WINDOW, which sits further out than
/// the card by exactly the room the shadow needs.
pub const WINDOW_MARGIN: i32 = MARGIN - SHADOW_ROOM;

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// The primary monitor's work area.
///
/// `SPI_GETWORKAREA` reports the primary display only; per-monitor placement
/// wants `GetMonitorInfo`, which is a later job once the widget remembers which
/// screen it was on.
#[cfg(windows)]
pub fn work_area() -> Option<Rect> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    let mut rect = RECT::default();

    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };

    if ok.is_err() {
        return None;
    }

    Some(Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    })
}

/// Elsewhere there is no work area to ask for, so the caller falls back to the
/// monitor's own bounds. Kept so the crate builds on the CI's Linux and macOS
/// runners as well as Windows.
#[cfg(not(windows))]
pub fn work_area() -> Option<Rect> {
    None
}

/// Bottom-right of the work area, inset by the margin.
///
/// Compact and expanded share this anchor, so opening grows the card up and to
/// the left and the lamp itself never moves.
pub fn anchor(area: Rect, width: i32, height: i32, scale: f64) -> (i32, i32) {
    let margin = (WINDOW_MARGIN as f64 * scale).round() as i32;
    let x = area.right - margin - width;
    let y = area.bottom - margin - height;

    (x.max(area.left), y.max(area.top))
}

/// The bar is capped to what actually fits: 1222 does not go on a 1366 laptop.
pub fn expanded_width(area: Rect, scale: f64) -> i32 {
    let margin = (WINDOW_MARGIN as f64 * scale).round() as i32;
    let max = ((MAX_WIDTH + SHADOW_ROOM * 2) as f64 * scale).round() as i32;

    max.min(area.width() - margin * 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_hd() -> Rect {
        // 1920x1080 with a 48px taskbar already excluded.
        Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1032,
        }
    }

    #[test]
    fn anchors_to_the_bottom_right_inside_the_margin() {
        // The WINDOW sits WINDOW_MARGIN from the edge, which puts the card
        // itself MARGIN from it: the difference is the room the shadow needs
        // to fade out inside a transparent, content-sized window.
        let (x, y) = anchor(full_hd(), 292 + SHADOW_ROOM * 2, 76, 1.0);

        assert_eq!(x + SHADOW_ROOM, 1920 - MARGIN - 292, "the card, not the window");
        assert_eq!(y, 1032 - WINDOW_MARGIN - 76);
    }

    #[test]
    fn compact_and_expanded_share_their_right_edge() {
        let area = full_hd();
        let (compact_x, _) = anchor(area, 292, 76, 1.0);
        let (wide_x, _) = anchor(area, 1222, 420, 1.0);

        assert_eq!(
            compact_x + 292,
            wide_x + 1222,
            "the lamp must not move when it opens"
        );
    }

    #[test]
    fn the_width_is_capped_to_what_fits() {
        let wide = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1032,
        };
        assert_eq!(expanded_width(wide, 1.0), 1222 + SHADOW_ROOM * 2);

        // The card is 1222 and the window is that plus the shadow's room, so a
        // 1366 laptop still takes the full width. The cap only bites below.
        let laptop = Rect {
            left: 0,
            top: 0,
            right: 1366,
            bottom: 728,
        };
        assert_eq!(expanded_width(laptop, 1.0), 1222 + SHADOW_ROOM * 2);

        let narrow = Rect {
            left: 0,
            top: 0,
            right: 1024,
            bottom: 728,
        };
        assert_eq!(expanded_width(narrow, 1.0), 1024 - WINDOW_MARGIN * 2);
    }

    #[test]
    fn the_margin_scales_with_dpi() {
        let area = Rect {
            left: 0,
            top: 0,
            right: 2880,
            bottom: 1548,
        };
        let (x, y) = anchor(area, 438, 114, 1.5);
        let margin = (WINDOW_MARGIN as f64 * 1.5).round() as i32;

        assert_eq!(x, 2880 - margin - 438);
        assert_eq!(y, 1548 - margin - 114);
    }

    #[test]
    fn a_window_wider_than_the_screen_still_lands_on_it() {
        let tiny = Rect {
            left: 0,
            top: 0,
            right: 400,
            bottom: 300,
        };
        let (x, y) = anchor(tiny, 1222, 420, 1.0);

        assert_eq!((x, y), (0, 0), "never position off the left or top edge");
    }
}
