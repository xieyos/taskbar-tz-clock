//! 绘制覆盖窗口：两行文字（时间、日期），版式对齐 Win11 系统时钟。

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DeleteObject, FW_NORMAL, FillRect, HDC, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode,
    SetTextAlign, SetTextColor, TA_BASELINE, TA_RIGHT, TRANSPARENT, TextOutW,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{PCWSTR, w};

use crate::theme::Palette;

const FONT_FACE: PCWSTR = w!("Segoe UI Variable Text");
/// 字号（96 DPI 下的 1/100 像素），按系统时钟实测文字宽度校准。
const FONT_CENTI_PX_AT_96: i32 = 1250;
/// 文字右边缘到时钟区域右边缘的距离（96 DPI 下的像素）。
const RIGHT_PADDING_AT_96: i32 = 3;
/// 两行文字基线在时钟区域高度中的位置：48 px 高的系统时钟实测为第 20 / 36 行。
const TIME_BASELINE: (i32, i32) = (20, 48);
const DATE_BASELINE: (i32, i32) = (36, 48);

pub struct Lines {
    pub time: String,
    pub date: String,
}

pub fn draw(hwnd: HWND, hdc: HDC, palette: Palette, lines: &Lines) {
    let mut client = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
        return;
    }
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
    draw_in(hdc, client, dpi, palette, lines);
}

/// 在任意 DC 上按给定区域和 DPI 绘制，窗口绘制和离屏测试共用。
fn draw_in(hdc: HDC, client: RECT, dpi: i32, palette: Palette, lines: &Lines) {
    unsafe {
        let brush = CreateSolidBrush(palette.background);
        FillRect(hdc, &client, brush);
        let _ = DeleteObject(brush.into());
    }

    // 四舍五入：125% 等非整数倍缩放下截断会让边距偏小
    let scale = |px: i32| (px * dpi + 48) / 96;
    let font_px = (FONT_CENTI_PX_AT_96 * dpi + 4800) / 9600;
    let height = client.bottom - client.top;
    let baseline = |(row, of): (i32, i32)| client.top + height * row / of;
    let right = client.right - scale(RIGHT_PADDING_AT_96);

    unsafe {
        let font = CreateFontW(
            -font_px, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0,
            FONT_FACE,
        );
        let old_font = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, palette.text);
        SetTextAlign(hdc, TA_RIGHT | TA_BASELINE);

        draw_text(hdc, right, baseline(TIME_BASELINE), &lines.time);
        draw_text(hdc, right, baseline(DATE_BASELINE), &lines.date);

        SelectObject(hdc, old_font);
        let _ = DeleteObject(font.into());
    }
}

fn draw_text(hdc: HDC, x: i32, y: i32, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let _ = unsafe { TextOutW(hdc, x, y, &wide) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
        DeleteDC,
    };

    /// 像素范围（含两端）：(首行, 末行, 首列, 末列)。
    type Bounds = (i32, i32, i32, i32);

    /// 离屏绘制系统时钟大小的区域，返回上下两半里文字像素的范围。
    fn ink_bounds(width: i32, height: i32, dpi: i32, lines: &Lines) -> (Bounds, Bounds) {
        let white = COLORREF(0x00FF_FFFF);
        let palette = Palette { background: white, text: COLORREF(0) };
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // 负数表示自上而下存储
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            let dc = CreateCompatibleDC(None);
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
                .expect("创建位图失败");
            let old = SelectObject(dc, bitmap.into());
            let client = RECT { left: 0, top: 0, right: width, bottom: height };
            draw_in(dc, client, dpi, palette, lines);

            let pixels = std::slice::from_raw_parts(bits as *const u32, (width * height) as usize);
            // 和系统时钟实测用同一个阈值：三通道合计比底色暗 150 以上算文字
            let is_ink = |x: i32, y: i32| {
                let p = pixels[(y * width + x) as usize];
                let sum = (p & 0xFF) + (p >> 8 & 0xFF) + (p >> 16 & 0xFF);
                765 - sum > 150
            };
            let band = |top: i32, bottom: i32| {
                let ink: Vec<_> = (top..=bottom)
                    .flat_map(|y| (0..width).map(move |x| (x, y)))
                    .filter(|&(x, y)| is_ink(x, y))
                    .collect();
                let ys = ink.iter().map(|p| p.1);
                let xs = ink.iter().map(|p| p.0);
                (ys.clone().min().unwrap(), ys.max().unwrap(), xs.clone().min().unwrap(), xs.max().unwrap())
            };
            let middle = height / 2;
            let result = (band(3, middle - 1), band(middle, height - 1));

            SelectObject(dc, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            result
        }
    }

    fn assert_near(actual: Bounds, expected: Bounds) {
        let diffs = [actual.0 - expected.0, actual.1 - expected.1, actual.2 - expected.2, actual.3 - expected.3];
        assert!(diffs.iter().all(|d| d.abs() <= 1), "实际 {actual:?}，系统时钟 {expected:?}");
    }

    /// 期望值是同一台机器上系统时钟的实测像素范围（`19:16` / `2026/10/19`），允许 1 px 误差。
    #[test]
    fn matches_system_clock_at_125_percent() {
        let lines = Lines { time: "19:16".into(), date: "2026/10/19".into() };
        let (time, date) = ink_bounds(91, 60, 120, &lines);
        assert_near(time, (14, 24, 53, 85));
        assert_near(date, (34, 46, 11, 85));
    }

    /// 100% 缩放下系统时钟（`10:13` / `2026/10/19`）实测：时间第 11–19 行，日期第 27–36 行、列 9–69。
    #[test]
    fn matches_system_clock_at_100_percent() {
        let lines = Lines { time: "10:13".into(), date: "2026/10/19".into() };
        let (time, date) = ink_bounds(73, 48, 96, &lines);
        // 100% 时只记录了时间行的纵向范围，横向沿用实测值本身以跳过比较
        assert_near((time.0, time.1, time.2, time.3), (11, 19, time.2, time.3));
        assert_near(date, (27, 36, 9, 69));
    }
}
