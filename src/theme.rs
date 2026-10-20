//! 覆盖窗口的配色：底色取任务栏实际像素，字色跟随系统浅色/深色主题。

use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS, POINT, RECT};
use windows::Win32::Graphics::Gdi::{CLR_INVALID, GetDC, GetPixel, ReleaseDC};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows::core::w;

const LIGHT_TEXT: COLORREF = rgb(0x1A, 0x1A, 0x1A);
const DARK_TEXT: COLORREF = rgb(0xFF, 0xFF, 0xFF);
const LIGHT_FALLBACK_BG: COLORREF = rgb(0xEE, 0xEE, 0xEE);
const DARK_FALLBACK_BG: COLORREF = rgb(0x20, 0x20, 0x20);

/// 取色点在时钟左侧多远、顶边往下多远（物理像素）：
/// 落在相邻托盘按钮的悬停高亮之外。
const SAMPLE_LEFT: i32 = 2;
const SAMPLE_DOWN: i32 = 2;
/// 鼠标离取色点这么近时（物理像素）不重新取色，避免取到按钮悬停高亮。
const CURSOR_GUARD: i32 = 80;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Palette {
    pub background: COLORREF,
    pub text: COLORREF,
}

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

/// 按当前主题和时钟位置生成配色。
///
/// 任务栏是亚克力材质，颜色随壁纸变化，所以底色取时钟左侧紧邻的任务栏像素
/// （取在覆盖窗口外面，避免取到自己）。鼠标在取色点附近时沿用上次的底色。
pub fn current(clock: &RECT, previous: Option<Palette>) -> Palette {
    let light = uses_light_theme();
    let point = POINT { x: clock.left - SAMPLE_LEFT, y: clock.top + SAMPLE_DOWN };
    let sampled = if cursor_near(point) { None } else { sample_pixel(point) };
    let fallback = if light { LIGHT_FALLBACK_BG } else { DARK_FALLBACK_BG };
    Palette {
        background: sampled.or(previous.map(|p| p.background)).unwrap_or(fallback),
        text: if light { LIGHT_TEXT } else { DARK_TEXT },
    }
}

fn cursor_near(point: POINT) -> bool {
    let mut cursor = POINT::default();
    // 读不到鼠标位置（如锁屏时）按"在附近"处理，宁可不取色
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return true;
    }
    (cursor.x - point.x).abs() < CURSOR_GUARD && (cursor.y - point.y).abs() < CURSOR_GUARD
}

fn sample_pixel(point: POINT) -> Option<COLORREF> {
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let color = GetPixel(screen, point.x, point.y);
        ReleaseDC(None, screen);
        (color != COLORREF(CLR_INVALID)).then_some(color)
    }
}

fn uses_light_theme() -> bool {
    let mut value: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    // 读不到时按浅色处理（Win11 默认）
    status != ERROR_SUCCESS || value != 0
}
