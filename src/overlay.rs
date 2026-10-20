//! 覆盖在任务栏时钟上的悬浮窗：置顶、不抢焦点、鼠标点击穿透。
//!
//! 所有状态都在 UI 线程上，由定时器和窗口消息驱动：
//! - 每秒或每分钟（对齐整点）重绘时间；
//! - 每 100 ms 检查是否该显示、是否被任务栏压住；
//! - 每 2 秒重新定位时钟、刷新底色、检查配置文件。
//!
//! 托盘菜单在同一线程上，通过本模块的 `pub fn` 修改配置、暂停和重新定位。

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HTTRANSPARENT, LWA_ALPHA,
    PBT_APMRESUMEAUTOMATIC, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SetLayeredWindowAttributes, SetTimer,
    PostMessageW, SetWindowPos, ShowWindow, WM_APP, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_ERASEBKGND, WM_HOTKEY, WM_NCHITTEST, WM_PAINT, WM_POWERBROADCAST, WM_SETTINGCHANGE,
    WM_TIMECHANGE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{Error, PCWSTR, Result, w};

use crate::clock::{self, Zone};
use crate::config::{Config, ConfigFile};
use crate::locator::{self, Locator};
use crate::render::{self, Lines};
use crate::theme::{self, Palette};
use crate::zorder::{self, WM_APP_REASSERT};

const CLASS_NAME: PCWSTR = w!("TaskbarTzClock.Overlay");
const HOTKEY_QUIT: i32 = 1;

const TIMER_TICK: usize = 1;
const TIMER_WATCH: usize = 2;
const TIMER_LOCATE: usize = 3;
/// 可见性和 Z 序检查间隔：决定任务栏顶上来时原时钟最多露出多久，检查本身只走几个窗口。
const WATCH_INTERVAL_MS: u32 = 100;
const LOCATE_INTERVAL_MS: u32 = 2000;
/// 整秒/整分后多等几毫秒，避免定时器略早触发而显示旧值。
const TICK_SLACK_MS: u32 = 5;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// 配置生效后投递给监听窗口（托盘）的消息，用来刷新提示文字。
pub const WM_APP_CONFIG_APPLIED: u32 = WM_APP + 2;

static LISTENER: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

#[derive(Default)]
struct State {
    clock: Option<RECT>,
    palette: Option<Palette>,
    shown: bool,
    /// 用户在托盘里暂停了覆盖。
    paused: bool,
    config: Config,
    /// 目标时区；`None` 表示跟随系统时区（未配置或 ID 无效）。
    zone: Option<Zone>,
}

thread_local! {
    // 定位器不放进 RefCell：UIA 是跨进程 COM 调用，STA 等待期间会处理消息并重入 wnd_proc，
    // 不能在调用期间持有借用。首次访问时创建，此前 main 已初始化 COM。
    static LOCATOR: Locator = Locator::new();
    static CONFIG: RefCell<ConfigFile> = RefCell::new(ConfigFile::open());
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// 创建覆盖窗口并启动各定时器。窗口在时钟可见时才显示。
pub fn create() -> Result<HWND> {
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

        let ex_style =
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT;
        let hwnd = CreateWindowExW(
            ex_style, CLASS_NAME, w!("taskbar-tz-clock"), WS_POPUP,
            0, 0, 0, 0, None, None, Some(instance), None,
        )?;
        // 分层窗口必须设置一次属性才会显示；255 为完全不透明
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;

        // 热键被占用时只是没法用快捷键退出，不影响覆盖本身
        let modifiers = MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT;
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_QUIT, modifiers, u32::from(b'Q'));

        apply_config_changes(hwnd);
        relocate(hwnd);
        SetTimer(Some(hwnd), TIMER_WATCH, WATCH_INTERVAL_MS, None);
        SetTimer(Some(hwnd), TIMER_LOCATE, LOCATE_INTERVAL_MS, None);
        Ok(hwnd)
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => on_timer(hwnd, wparam.0),
        WM_PAINT => paint(hwnd),
        WM_ERASEBKGND => return LRESULT(1),
        WM_NCHITTEST => return LRESULT(HTTRANSPARENT as isize),
        WM_DISPLAYCHANGE | WM_DPICHANGED => relocate_fresh(hwnd),
        // 主题或区域格式变化
        WM_SETTINGCHANGE => {
            relocate(hwnd);
            invalidate(hwnd);
        }
        // 系统时间或时区被修改
        WM_TIMECHANGE => {
            invalidate(hwnd);
            schedule_tick(hwnd);
        }
        WM_POWERBROADCAST => {
            // 睡眠唤醒后定时器节奏已乱，显示器布局也可能变了
            if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize {
                relocate_fresh(hwnd);
                invalidate(hwnd);
                schedule_tick(hwnd);
            }
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        WM_APP_REASSERT => update_visibility(hwnd),
        WM_HOTKEY if wparam.0 == HOTKEY_QUIT as usize => {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        // Explorer 重启后任务栏重建，旧的 UIA 元素已失效
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => relocate_fresh(hwnd),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

fn on_timer(hwnd: HWND, id: usize) {
    match id {
        TIMER_TICK => {
            invalidate(hwnd);
            schedule_tick(hwnd);
        }
        TIMER_WATCH => update_visibility(hwnd),
        TIMER_LOCATE => {
            apply_config_changes(hwnd);
            relocate(hwnd);
        }
        _ => {}
    }
}

/// 配置生效时通知该窗口（`WM_APP_CONFIG_APPLIED`）。
pub fn set_listener(hwnd: HWND) {
    LISTENER.store(hwnd.0, Ordering::Relaxed);
}

/// 当前生效的配置。
pub fn current_config() -> Config {
    STATE.with_borrow(|state| state.config.clone())
}

/// 当前时区的显示名，跟随系统时为 `None`。
pub fn zone_name() -> Option<String> {
    STATE.with_borrow(|state| state.zone.as_ref().map(clock::standard_name))
}

/// 配置文件路径。
pub fn config_path() -> Option<PathBuf> {
    CONFIG.with_borrow(|file| file.path().map(PathBuf::from))
}

/// 修改配置：写回文件并立即生效。写文件失败时仍在本次运行中生效。
pub fn update_config(hwnd: HWND, change: impl FnOnce(&mut Config)) {
    let mut config = current_config();
    change(&mut config);
    let _ = CONFIG.with_borrow_mut(|file| file.save(&config));
    apply_config(hwnd, config);
}

pub fn is_paused() -> bool {
    STATE.with_borrow(|state| state.paused)
}

/// 暂停或恢复覆盖（只在本次运行中有效）。
pub fn set_paused(hwnd: HWND, paused: bool) {
    STATE.with_borrow_mut(|state| state.paused = paused);
    update_visibility(hwnd);
}

/// 配置文件有变化时更新时区和秒显示。
fn apply_config_changes(hwnd: HWND) {
    if let Some(config) = CONFIG.with_borrow_mut(ConfigFile::reload_if_changed) {
        apply_config(hwnd, config);
    }
}

fn apply_config(hwnd: HWND, config: Config) {
    // 时区 ID 为空或无效时跟随系统时区
    let zone = Some(config.time_zone.trim())
        .filter(|id| !id.is_empty())
        .and_then(clock::find_zone);
    STATE.with_borrow_mut(|state| {
        state.zone = zone;
        state.config = config;
    });
    invalidate(hwnd);
    schedule_tick(hwnd);
    let listener = HWND(LISTENER.load(Ordering::Relaxed));
    if !listener.is_invalid() {
        let _ = unsafe { PostMessageW(Some(listener), WM_APP_CONFIG_APPLIED, WPARAM(0), LPARAM(0)) };
    }
}

/// 安排下一次重绘，对齐到整秒（显示秒时）或整分。
fn schedule_tick(hwnd: HWND) {
    let show_seconds = STATE.with_borrow(|state| state.config.show_seconds);
    let delay = clock::millis_until_next_tick(&clock::now_utc(), show_seconds);
    unsafe { SetTimer(Some(hwnd), TIMER_TICK, delay + TICK_SLACK_MS, None) };
}

/// 丢弃缓存的时钟元素后重新定位（也供托盘菜单"重新定位"调用）。
pub fn relocate_fresh(hwnd: HWND) {
    LOCATOR.with(Locator::reset);
    relocate(hwnd);
}

/// 重新定位到时钟上方并刷新底色。
fn relocate(hwnd: HWND) {
    // 定位和取色都在借用之外完成，借用只用来交换结果；
    // 之后再调窗口 API，因为 SetWindowPos 会同步回调 wnd_proc
    let current = LOCATOR.with(Locator::clock_rect);
    let previous_palette = STATE.with_borrow(|state| state.palette);
    let palette = current.as_ref().map(|rect| theme::current(rect, previous_palette));
    let (previous, repaint) = STATE.with_borrow_mut(|state| {
        let previous = std::mem::replace(&mut state.clock, current);
        let repaint = palette != std::mem::replace(&mut state.palette, palette);
        (previous, repaint)
    });

    if let Some(rect) = current
        && Some(rect) != previous
    {
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        let flags = SWP_NOACTIVATE | SWP_NOZORDER;
        let _ = unsafe { SetWindowPos(hwnd, None, rect.left, rect.top, width, height, flags) };
        invalidate(hwnd);
    } else if repaint {
        invalidate(hwnd);
    }
    update_visibility(hwnd);
}

/// 时钟露在外面时显示并保持在任务栏之上，否则隐藏。
fn update_visibility(hwnd: HWND) {
    let (clock, paused) = STATE.with_borrow(|state| (state.clock, state.paused));
    let taskbar = locator::taskbar();
    let visible = match (clock, taskbar) {
        (Some(clock), Some(taskbar)) if !paused => zorder::clock_uncovered(&clock, taskbar),
        _ => false,
    };
    let changed = STATE.with_borrow_mut(|state| std::mem::replace(&mut state.shown, visible) != visible);
    if changed {
        let command = if visible { SW_SHOWNOACTIVATE } else { SW_HIDE };
        let _ = unsafe { ShowWindow(hwnd, command) };
    }
    if visible
        && let Some(taskbar) = taskbar
        && zorder::is_below(hwnd, taskbar)
    {
        zorder::bring_to_top(hwnd);
    }
}

fn invalidate(hwnd: HWND) {
    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
}

fn paint(hwnd: HWND) {
    let (palette, zone, show_seconds) =
        STATE.with_borrow(|state| (state.palette, state.zone, state.config.show_seconds));
    let lines = clock::to_zone(&clock::now_utc(), zone.as_ref()).map(|time| Lines {
        time: clock::format_time(&time, show_seconds),
        date: clock::format_date(&time),
    });
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        if let (Some(palette), Some(lines)) = (palette, lines) {
            render::draw(hwnd, hdc, palette, &lines);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}
