//! 维持覆盖窗口在任务栏之上。
//!
//! 任务栏本身也是置顶窗口，被点击或前台切换时会盖到我们上面，
//! 所以在前台切换时和定时检查时都重新置顶。

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, GA_ROOT, GW_HWNDPREV, GetAncestor, GetWindow, HWND_TOPMOST,
    PostMessageW, WindowFromPoint,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SetWindowPos, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_APP,
};

/// 前台切换后投递给覆盖窗口的消息，收到后重新置顶。
pub const WM_APP_REASSERT: u32 = WM_APP + 1;

/// 沿 Z 序向上最多查找的窗口数；超过仍没到顶就当作被压住。
const MAX_WALK: usize = 64;

static TARGET: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

/// 监听前台窗口切换。返回的句柄需在退出时 `UnhookWinEvent`。
pub fn install_foreground_hook(hwnd: HWND) -> HWINEVENTHOOK {
    TARGET.store(hwnd.0, Ordering::Relaxed);
    unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    }
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    let target = HWND(TARGET.load(Ordering::Relaxed));
    if !target.is_invalid() {
        // 投递而不是直接置顶：让任务栏先完成它自己的 Z 序调整
        let _ = unsafe { PostMessageW(Some(target), WM_APP_REASSERT, WPARAM(0), LPARAM(0)) };
    }
}

/// 时钟中心点此刻最上面的窗口是不是任务栏。
///
/// 覆盖窗口带 `WS_EX_TRANSPARENT`，`WindowFromPoint` 会跳过它（本机实测）。
/// 全屏程序、收起的自动隐藏任务栏、盖在时钟上的菜单都会让这里返回 `false`，
/// 这时覆盖窗口应隐藏，避免浮在全屏程序上面。
pub fn clock_uncovered(clock: &RECT, taskbar: HWND) -> bool {
    let center = POINT { x: (clock.left + clock.right) / 2, y: (clock.top + clock.bottom) / 2 };
    let hit = unsafe { WindowFromPoint(center) };
    !hit.is_invalid() && unsafe { GetAncestor(hit, GA_ROOT) } == taskbar
}

/// 覆盖窗口是否被任务栏压在下面（或无法确认位置）。
pub fn is_below(hwnd: HWND, taskbar: HWND) -> bool {
    let mut current = hwnd;
    for _ in 0..MAX_WALK {
        match unsafe { GetWindow(current, GW_HWNDPREV) } {
            Ok(prev) if prev == taskbar => return true,
            Ok(prev) => current = prev,
            // 上面已经没有窗口，说明在最顶端
            Err(_) => return false,
        }
    }
    true
}

/// 把覆盖窗口放到置顶层的最上面，不抢焦点。
pub fn bring_to_top(hwnd: HWND) {
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
    let _ = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags) };
}
