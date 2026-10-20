//! taskbar-tz-clock：用置顶悬浮窗覆盖任务栏时钟，显示指定时区的时间。
//!
//! 通过托盘菜单或 `%APPDATA%\taskbar-tz-clock\config.json` 设置时区和是否显示秒；
//! 从托盘菜单或按 Ctrl+Alt+Shift+Q 退出。
#![windows_subsystem = "windows"]

mod autostart;
mod clock;
mod config;
mod locator;
mod overlay;
mod render;
mod theme;
mod tray;
mod zorder;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Accessibility::UnhookWinEvent;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, TranslateMessage};
use windows::core::{Result, w};

fn main() -> Result<()> {
    unsafe {
        // 句柄在进程退出前一直持有，保证只运行一个实例
        let _mutex = CreateMutexW(None, true, w!("Local\\taskbar-tz-clock.SingleInstance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(());
        }
        // 失败时退回系统默认 DPI 模式：坐标可能有偏差，但不阻断启动
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        // 迁移改名前的开机自启项；失败只影响自启，用户可在托盘里重新勾选
        let _ = autostart::migrate_legacy();

        let hwnd = overlay::create()?;
        // 托盘创建失败时仍可用配置文件和快捷键操作
        let _ = tray::create(hwnd);
        let hook = zorder::install_foreground_hook(hwnd);

        let mut msg = MSG::default();
        // GetMessageW 出错时返回 -1，只有正数才继续循环
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // 用快捷键退出时托盘窗口没被销毁，这里补删图标
        tray::remove_icon();
        if !hook.is_invalid() {
            let _ = UnhookWinEvent(hook);
        }
    }
    Ok(())
}
