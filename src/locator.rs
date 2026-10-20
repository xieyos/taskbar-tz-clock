//! 定位主任务栏时钟的屏幕区域（物理像素）。
//!
//! Win11 的时钟是 XAML 元素，没有独立 HWND，只能通过 UI Automation 找；
//! Win10 退回到 `TrayClockWClass` 窗口。

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationCondition, IUIAutomationElement,
    TreeScope_Descendants, UIA_ClassNamePropertyId,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowRect};
use windows::core::{Interface, w};

/// Win11 托盘里时钟按钮可能的 UIA ClassName（本机 26100 实测）。
///
/// 后缀取决于时钟在"网络/音量/时钟"按钮组里的位置：通常是组的左端 `Left`，
/// 网络按钮单独显示（例如断网时变成 `AccentButton`）后时钟会变成不带后缀的 `OmniButton`。
const CLOCK_CLASSES: [&str; 4] = [
    "SystemTray.OmniButtonLeft",
    "SystemTray.OmniButton",
    "SystemTray.OmniButtonMiddle",
    "SystemTray.OmniButtonRight",
];
/// 名字对不上时可以当作时钟的类名，按优先级排列；`Middle`/`Right` 是音量、通知等按钮。
const FALLBACK_CLASSES: [&str; 2] = ["SystemTray.OmniButtonLeft", "SystemTray.OmniButton"];
/// 时钟按钮的 UIA Name 前缀，同类按钮有多个时用它挑出时钟。
const CLOCK_NAME_PREFIXES: [&str; 2] = ["时钟", "Clock"];
/// UIA 跨进程调用超时：Explorer 卡住时不能把我们的 UI 线程一起拖住。
const UIA_TIMEOUT_MS: u32 = 1000;

pub struct Locator {
    automation: Option<IUIAutomation>,
    /// 上次找到的时钟元素。用 Cell 而不是 RefCell：UIA 调用期间会重入，不能持有借用。
    cached: Cell<Option<IUIAutomationElement>>,
}

impl Locator {
    pub fn new() -> Self {
        let automation: Option<IUIAutomation> =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok();
        if let Some(v2) = automation.as_ref().and_then(|a| a.cast::<IUIAutomation2>().ok()) {
            // 设置失败时沿用系统默认超时
            unsafe {
                let _ = v2.SetConnectionTimeout(UIA_TIMEOUT_MS);
                let _ = v2.SetTransactionTimeout(UIA_TIMEOUT_MS);
            }
        }
        Self { automation, cached: Cell::new(None) }
    }

    /// 返回主任务栏时钟区域；任务栏或时钟不存在时返回 `None`。
    ///
    /// 优先读缓存元素的当前区域（一次跨进程调用），失效时才重新搜索整棵树。
    pub fn clock_rect(&self) -> Option<RECT> {
        let taskbar = taskbar()?;
        if let Some(element) = self.cached.take() {
            if let Some(rect) = unsafe { element.CurrentBoundingRectangle() }.ok().filter(is_valid) {
                self.cached.set(Some(element));
                return Some(rect);
            }
        }
        self.search_uia(taskbar).or_else(|| find_win10_clock(taskbar))
    }

    /// 丢弃缓存，下次定位时重新搜索（Explorer 重启、显示设置变化后调用）。
    pub fn reset(&self) {
        self.cached.set(None);
    }

    fn search_uia(&self, taskbar: HWND) -> Option<RECT> {
        let automation = self.automation.as_ref()?;
        let (element, rect) = unsafe {
            let root = automation.ElementFromHandle(taskbar).ok()?;
            let mut condition = None::<IUIAutomationCondition>;
            for class in CLOCK_CLASSES {
                let next = automation
                    .CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(class))
                    .ok()?;
                condition = Some(match condition {
                    Some(prev) => automation.CreateOrCondition(&prev, &next).ok()?,
                    None => next,
                });
            }
            let items = root.FindAll(TreeScope_Descendants, &condition?).ok()?;
            let mut best: Option<(u8, IUIAutomationElement, RECT)> = None;
            for i in 0..items.Length().ok()? {
                let Ok(element) = items.GetElement(i) else { continue };
                let class = element.CurrentClassName().map(|n| n.to_string()).unwrap_or_default();
                let name = element.CurrentName().map(|n| n.to_string()).unwrap_or_default();
                let Some(rank) = clock_rank(&class, &name) else { continue };
                if best.as_ref().is_some_and(|(r, ..)| *r <= rank) {
                    continue;
                }
                let Some(rect) = element.CurrentBoundingRectangle().ok().filter(is_valid) else {
                    continue;
                };
                best = Some((rank, element, rect));
                if rank == 0 {
                    break;
                }
            }
            best.map(|(_, element, rect)| (element, rect))?
        };
        self.cached.set(Some(element));
        Some(rect)
    }
}

/// 候选按钮像时钟的程度，越小越像；`None` 表示不是时钟。
///
/// 名字以"时钟"/"Clock"开头的最可靠；非中英文系统名字对不上时，按类名退回。
fn clock_rank(class: &str, name: &str) -> Option<u8> {
    if CLOCK_NAME_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return Some(0);
    }
    FALLBACK_CLASSES.iter().position(|c| *c == class).map(|i| i as u8 + 1)
}

/// 主任务栏窗口。
pub fn taskbar() -> Option<HWND> {
    unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }.ok()
}

fn find_win10_clock(taskbar: HWND) -> Option<RECT> {
    unsafe {
        let notify = FindWindowExW(Some(taskbar), None, w!("TrayNotifyWnd"), None).ok()?;
        let clock = FindWindowExW(Some(notify), None, w!("TrayClockWClass"), None).ok()?;
        let mut rect = RECT::default();
        GetWindowRect(clock, &mut rect).ok()?;
        is_valid(&rect).then_some(rect)
    }
}

fn is_valid(rect: &RECT) -> bool {
    rect.right > rect.left && rect.bottom > rect.top
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    #[test]
    fn rank_prefers_name_then_left_then_plain() {
        assert_eq!(clock_rank("SystemTray.OmniButton", "时钟 21:16"), Some(0));
        assert_eq!(clock_rank("SystemTray.OmniButtonRight", "Clock 9:16 PM"), Some(0));
        assert_eq!(clock_rank("SystemTray.OmniButtonLeft", "Uhr"), Some(1));
        assert_eq!(clock_rank("SystemTray.OmniButton", "Uhr"), Some(2));
        assert_eq!(clock_rank("SystemTray.OmniButtonRight", "音量 100%"), None);
        assert_eq!(clock_rank("SystemTray.OmniButtonMiddle", ""), None);
    }

    /// 依赖真实任务栏，默认跳过：`cargo test -- --ignored`
    #[test]
    #[ignore]
    fn locates_clock_on_live_taskbar() {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok().unwrap();
        let locator = Locator::new();
        assert!(locator.automation.is_some(), "UIA 创建失败");
        let taskbar = taskbar().expect("找不到 Shell_TrayWnd");
        let rect = locator.search_uia(taskbar);
        assert!(rect.is_some(), "UIA 搜索失败");
        assert!(locator.clock_rect().is_some(), "缓存路径失败");
    }
}
