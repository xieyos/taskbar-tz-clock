//! 托盘图标和菜单：选时区、显示秒、暂停覆盖、开机自启、打开配置、重新定位、退出。
//!
//! 用一个隐藏的普通窗口接收托盘消息：覆盖窗口带 `WS_EX_NOACTIVATE`，
//! 拿不到前台，弹出的菜单点别处时不会自动关闭。

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    ShellExecuteW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, GetCursorPos, HICON, HMENU, IDI_APPLICATION, IMAGE_ICON, KillTimer,
    LR_DEFAULTCOLOR, LoadIconW, LoadImageW, MB_ICONERROR, MB_OK, MENU_ITEM_FLAGS, MF_CHECKED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, MessageBoxW, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SM_CXSMICON, SW_SHOWNORMAL, SetForegroundWindow, SetTimer,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{Error, HSTRING, PCWSTR, Result, w};

use crate::overlay::{self, WM_APP_CONFIG_APPLIED};
use crate::{autostart, clock};

const CLASS_NAME: PCWSTR = w!("TaskbarTzClock.Tray");
const APP_TITLE: &str = "任务栏时区时钟";
const TRAY_ID: u32 = 1;
const WM_APP_TRAY: u32 = WM_APP + 3;
/// exe 里图标组资源的编号，与 build.rs 的 `GROUP_ID` 保持一致。
const APP_ICON_ID: u16 = 1;
const TIMER_RETRY_ADD: usize = 1;
const RETRY_ADD_MS: u32 = 2000;

const CMD_FOLLOW_SYSTEM: usize = 1;
const CMD_SECONDS: usize = 2;
const CMD_PAUSE: usize = 3;
const CMD_AUTOSTART: usize = 4;
const CMD_OPEN_CONFIG: usize = 5;
const CMD_RELOCATE: usize = 6;
const CMD_EXIT: usize = 7;
/// 时区菜单项的起始命令 ID，第 i 个时区为 `CMD_ZONE_BASE + i`。
const CMD_ZONE_BASE: usize = 1000;

/// 菜单第一层列出的常用时区；系统里没有的会被跳过。
const COMMON_ZONES: [&str; 10] = [
    "China Standard Time",
    "Tokyo Standard Time",
    "Singapore Standard Time",
    "India Standard Time",
    "GMT Standard Time",
    "W. Europe Standard Time",
    "Eastern Standard Time",
    "Central Standard Time",
    "Pacific Standard Time",
    "UTC",
];

static OVERLAY: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static TRAY: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static ICON: Cell<Option<HICON>> = const { Cell::new(None) };
}

/// 创建托盘窗口并添加图标。`overlay` 是覆盖窗口，菜单操作作用于它。
pub fn create(overlay: HWND) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(Error::from_thread());
        }
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW, CLASS_NAME, w!("taskbar-tz-clock tray"), WS_POPUP,
            0, 0, 0, 0, None, None, Some(instance), None,
        )?;
        OVERLAY.store(overlay.0, Ordering::Relaxed);
        TRAY.store(hwnd.0, Ordering::Relaxed);
        overlay::set_listener(hwnd);
        add_icon(hwnd);
        Ok(hwnd)
    }
}

/// 移除托盘图标；退出前调用，避免留下失效图标。可重复调用。
pub fn remove_icon() {
    let hwnd = HWND(TRAY.swap(null_mut(), Ordering::Relaxed));
    if !hwnd.is_invalid() {
        let data = icon_data(hwnd);
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }
    if let Some(icon) = ICON.take() {
        let _ = unsafe { DestroyIcon(icon) };
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_TRAY => {
            let event = (lparam.0 & 0xFFFF) as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                show_menu(hwnd);
            }
        }
        WM_APP_CONFIG_APPLIED => update_tip(hwnd),
        WM_TIMER if wparam.0 == TIMER_RETRY_ADD => add_icon(hwnd),
        WM_DISPLAYCHANGE | WM_DPICHANGED => refresh_icon(hwnd),
        WM_DESTROY => {
            remove_icon();
            let overlay = overlay_hwnd();
            if !overlay.is_invalid() {
                // 覆盖窗口销毁时会结束消息循环
                let _ = unsafe { DestroyWindow(overlay) };
            }
        }
        // Explorer 重启后托盘图标会丢失，需要重新添加
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => add_icon(hwnd),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

fn overlay_hwnd() -> HWND {
    HWND(OVERLAY.load(Ordering::Relaxed))
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

/// 按托盘窗口当前 DPI 从 exe 资源加载合适尺寸的图标，替换并销毁旧图标。
/// 资源缺失时退回系统默认程序图标。
fn load_icon(hwnd: HWND) -> HICON {
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(hwnd).max(96)) };
    let loaded = unsafe {
        let instance = GetModuleHandleW(None).ok().map(HINSTANCE::from);
        // MAKEINTRESOURCE：资源编号放在指针的低 16 位
        let name = PCWSTR(usize::from(APP_ICON_ID) as *const u16);
        LoadImageW(instance, name, IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
    };
    let icon = match loaded {
        Ok(handle) => HICON(handle.0),
        // 共享的系统图标不能 DestroyIcon，所以不记入 ICON
        Err(_) => return unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default(),
    };
    if let Some(old) = ICON.replace(Some(icon)) {
        let _ = unsafe { DestroyIcon(old) };
    }
    icon
}

/// DPI 或显示设置变化后换成新尺寸的图标。
fn refresh_icon(hwnd: HWND) {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_ICON;
    data.hIcon = load_icon(hwnd);
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

fn add_icon(hwnd: HWND) {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_APP_TRAY;
    data.hIcon = load_icon(hwnd);
    fill_tip(&mut data.szTip);
    // 开机自启时托盘可能还没就绪，失败后定时重试，成功即停
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool();
    unsafe {
        if added {
            let _ = KillTimer(Some(hwnd), TIMER_RETRY_ADD);
        } else {
            SetTimer(Some(hwnd), TIMER_RETRY_ADD, RETRY_ADD_MS, None);
        }
    }
}

fn update_tip(hwnd: HWND) {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_TIP;
    fill_tip(&mut data.szTip);
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

/// 鼠标悬停提示：程序名 + 当前时区（+ 暂停状态），超长时截断。
fn fill_tip(tip: &mut [u16; 128]) {
    let mut text = format!("{APP_TITLE}\n{}", zone_label());
    if overlay::is_paused() {
        text.push_str("（已暂停）");
    }
    let wide: Vec<u16> = text.encode_utf16().take(tip.len() - 1).collect();
    tip[..wide.len()].copy_from_slice(&wide);
    tip[wide.len()] = 0;
}

/// 当前时区的说明文字。
fn zone_label() -> String {
    match overlay::zone_name() {
        Some(name) => name,
        None if overlay::current_config().time_zone.trim().is_empty() => "跟随系统时区".into(),
        None => "时区 ID 无效，按系统时区显示".into(),
    }
}

fn show_menu(hwnd: HWND) {
    let Ok((menu, zone_ids)) = build_menu() else { return };
    let mut cursor = POINT::default();
    let command = unsafe {
        let _ = GetCursorPos(&mut cursor);
        // 不先置前台，点菜单外面时菜单不会关闭（TrackPopupMenu 文档要求）
        let _ = SetForegroundWindow(hwnd);
        let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY;
        let command = TrackPopupMenu(menu, flags, cursor.x, cursor.y, None, hwnd, None).0 as usize;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        // 销毁主菜单时会一并销毁挂在它下面的子菜单
        let _ = DestroyMenu(menu);
        command
    };
    run_command(hwnd, command, &zone_ids);
}

/// 构建菜单，同时返回时区命令对应的时区 ID 列表。
fn build_menu() -> Result<(HMENU, Vec<String>)> {
    let config = overlay::current_config();
    let selected = config.time_zone.trim();
    let mut zones: Vec<_> = clock::zones().collect();
    zones.sort_by_key(|zone| -zone.Bias);
    let mut zone_ids = Vec::new();

    unsafe {
        let zone_menu = CreatePopupMenu()?;
        append(zone_menu, CMD_FOLLOW_SYSTEM, "跟随系统时区", selected.is_empty())?;
        append_separator(zone_menu)?;
        let common = COMMON_ZONES.iter().filter_map(|id| zones.iter().find(|z| clock::key_name(z) == *id));
        let all_menu = CreatePopupMenu()?;
        for (target, zone) in common.map(|z| (zone_menu, z)).chain(zones.iter().map(|z| (all_menu, z))) {
            let id = clock::key_name(zone);
            let label = format!("{}  {}", clock::base_offset(zone), clock::standard_name(zone));
            append(target, CMD_ZONE_BASE + zone_ids.len(), &label, id.eq_ignore_ascii_case(selected))?;
            zone_ids.push(id);
        }
        append_separator(zone_menu)?;
        AppendMenuW(zone_menu, MF_POPUP, all_menu.0 as usize, &HSTRING::from("全部时区"))?;

        let menu = CreatePopupMenu()?;
        let title = format!("时区：{}", zone_label());
        AppendMenuW(menu, MF_POPUP, zone_menu.0 as usize, &HSTRING::from(title))?;
        append(menu, CMD_SECONDS, "显示秒", config.show_seconds)?;
        append(menu, CMD_PAUSE, "暂停覆盖", overlay::is_paused())?;
        append_separator(menu)?;
        append(menu, CMD_AUTOSTART, "开机自启", autostart::is_enabled())?;
        append(menu, CMD_OPEN_CONFIG, "打开配置文件", false)?;
        append(menu, CMD_RELOCATE, "重新定位时钟", false)?;
        append_separator(menu)?;
        append(menu, CMD_EXIT, "退出", false)?;
        Ok((menu, zone_ids))
    }
}

fn append(menu: HMENU, command: usize, text: &str, checked: bool) -> Result<()> {
    let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
    unsafe { AppendMenuW(menu, flags, command, &HSTRING::from(text)) }
}

fn append_separator(menu: HMENU) -> Result<()> {
    unsafe { AppendMenuW(menu, MF_SEPARATOR | MENU_ITEM_FLAGS(0), 0, PCWSTR::null()) }
}

fn run_command(hwnd: HWND, command: usize, zone_ids: &[String]) {
    let overlay = overlay_hwnd();
    match command {
        CMD_FOLLOW_SYSTEM => overlay::update_config(overlay, |c| c.time_zone.clear()),
        CMD_SECONDS => overlay::update_config(overlay, |c| c.show_seconds = !c.show_seconds),
        CMD_PAUSE => {
            overlay::set_paused(overlay, !overlay::is_paused());
            update_tip(hwnd);
        }
        CMD_AUTOSTART => {
            if autostart::set_enabled(!autostart::is_enabled()).is_err() {
                show_error(hwnd, "修改开机自启失败：无法写入注册表。");
            }
        }
        CMD_OPEN_CONFIG => open_config(hwnd),
        CMD_RELOCATE => overlay::relocate_fresh(overlay),
        CMD_EXIT => {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        id if id >= CMD_ZONE_BASE => {
            if let Some(zone) = zone_ids.get(id - CMD_ZONE_BASE) {
                overlay::update_config(overlay, |c| c.time_zone = zone.clone());
            }
        }
        // 0：菜单被取消
        _ => {}
    }
}

/// 用关联程序打开配置文件；.json 没有关联程序时退回记事本。
fn open_config(hwnd: HWND) {
    let Some(path) = overlay::config_path() else {
        show_error(hwnd, "找不到配置目录（%APPDATA% 不可用）。");
        return;
    };
    let file = HSTRING::from(path.as_os_str());
    // ShellExecuteW 返回值大于 32 表示成功
    let opened = |result: windows::Win32::Foundation::HINSTANCE| result.0 as usize > 32;
    unsafe {
        if opened(ShellExecuteW(Some(hwnd), w!("open"), &file, None, None, SW_SHOWNORMAL)) {
            return;
        }
        let quoted = HSTRING::from(format!("\"{}\"", path.display()));
        if !opened(ShellExecuteW(Some(hwnd), w!("open"), w!("notepad.exe"), &quoted, None, SW_SHOWNORMAL)) {
            show_error(hwnd, "无法打开配置文件。");
        }
    }
}

fn show_error(hwnd: HWND, text: &str) {
    unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), &HSTRING::from(APP_TITLE), MB_OK | MB_ICONERROR) };
}
