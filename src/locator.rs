//! 定位主任务栏时钟的屏幕区域（物理像素）。
//!
//! Win11 的时钟是 XAML 元素，没有独立 HWND，只能通过 UI Automation 找；
//! Win10 退回到 `TrayClockWClass` 窗口。

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationElement, TreeScope_Descendants,
    UIA_ClassNamePropertyId,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowRect};
use windows::core::{Interface, w};

/// Win11 托盘里时钟按钮的 UIA ClassName（本机 26100 实测）。
const CLOCK_CLASS: &str = "SystemTray.OmniButtonLeft";
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
            let condition = automation
                .CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(CLOCK_CLASS))
                .ok()?;
            let items = root.FindAll(TreeScope_Descendants, &condition).ok()?;
            let mut fallback = None;
            let mut matched = None;
            for i in 0..items.Length().ok()? {
                let Ok(element) = items.GetElement(i) else { continue };
                let Some(rect) = element.CurrentBoundingRectangle().ok().filter(is_valid) else {
                    continue;
                };
                let name = element.CurrentName().map(|n| n.to_string()).unwrap_or_default();
                if CLOCK_NAME_PREFIXES.iter().any(|p| name.starts_with(p)) {
                    matched = Some((element, rect));
                    break;
                }
                // 非中英文系统名字对不上时，退回第一个同类按钮
                fallback.get_or_insert((element, rect));
            }
            matched.or(fallback)?
        };
        self.cached.set(Some(element));
        Some(rect)
    }
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
