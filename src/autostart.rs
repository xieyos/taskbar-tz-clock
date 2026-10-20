//! 开机自启：写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，不需要管理员权限。

use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, Result, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: PCWSTR = w!("taskbar-tz-clock");
/// 改名前（TaskbarClockTz）使用的值名，启动时迁移。
const LEGACY_VALUE_NAME: PCWSTR = w!("TaskbarClockTz");
/// Run 值的最大长度（UTF-16 字符）；路径再长也远小于此值。
const MAX_VALUE_LEN: usize = 1024;

/// Run 里登记的命令，路径加引号以兼容空格。
fn command_for(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// 当前 exe 是否已登记为开机自启。登记的是别的路径（比如 exe 被移动过）时视为未开启。
pub fn is_enabled() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    read_value(VALUE_NAME).is_some_and(|value| value.eq_ignore_ascii_case(&command_for(&exe)))
}

/// 开启或关闭开机自启。
pub fn set_enabled(enabled: bool) -> Result<()> {
    if enabled {
        let command = command_for(&std::env::current_exe()?);
        let data: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE_NAME,
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * size_of::<u16>()) as u32,
            )
        };
        status.ok()
    } else {
        delete_value(VALUE_NAME)
    }
}

/// 把改名前登记的自启项换成新名字，并指向当前 exe，保留用户"已开启自启"的选择。
/// 没有旧项时什么也不做。
pub fn migrate_legacy() -> Result<()> {
    if read_value(LEGACY_VALUE_NAME).is_none() {
        return Ok(());
    }
    set_enabled(true)?;
    delete_value(LEGACY_VALUE_NAME)
}

fn delete_value(name: PCWSTR) -> Result<()> {
    match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, name) } {
        // 本来就没有登记，也算删除成功
        ERROR_FILE_NOT_FOUND => Ok(()),
        status => status.ok(),
    }
}

fn read_value(name: PCWSTR) -> Option<String> {
    let mut buffer = vec![0u16; MAX_VALUE_LEN];
    let mut size = (buffer.len() * size_of::<u16>()) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_value(name: PCWSTR, value: &str) {
        let data: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
        let size = (data.len() * size_of::<u16>()) as u32;
        let status =
            unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, name, REG_SZ.0, Some(data.as_ptr().cast()), size) };
        assert_eq!(status, ERROR_SUCCESS);
    }

    /// 测试默认并行运行，读写同一注册表项的测试必须串行。
    static REGISTRY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 保存两个值名的原状，执行测试后恢复。
    fn with_saved_values(test: impl FnOnce()) {
        // 前一个测试断言失败会让锁中毒，但注册表已恢复，可以继续用
        let _guard = REGISTRY_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved = [VALUE_NAME, LEGACY_VALUE_NAME].map(|name| (name, read_value(name)));
        test();
        for (name, value) in saved {
            match value {
                Some(value) => write_value(name, &value),
                None => delete_value(name).unwrap(),
            }
        }
    }

    /// 会读写当前用户的 Run 注册表并恢复原状，默认跳过：`cargo test -- --ignored`
    #[test]
    #[ignore]
    fn toggles_run_key_and_restores() {
        with_saved_values(|| {
            set_enabled(true).unwrap();
            assert!(is_enabled());
            set_enabled(false).unwrap();
            assert!(!is_enabled() && read_value(VALUE_NAME).is_none());
            // 重复关闭也应成功
            set_enabled(false).unwrap();
        });
    }

    /// 同上，默认跳过。
    #[test]
    #[ignore]
    fn migrates_legacy_value() {
        with_saved_values(|| {
            delete_value(VALUE_NAME).unwrap();
            write_value(LEGACY_VALUE_NAME, r#""C:\old\taskbar-clock-tz.exe""#);
            migrate_legacy().unwrap();
            assert!(is_enabled(), "新值应指向当前 exe");
            assert!(read_value(LEGACY_VALUE_NAME).is_none(), "旧值应被删除");
            // 没有旧值时不应自动开启
            delete_value(VALUE_NAME).unwrap();
            migrate_legacy().unwrap();
            assert!(!is_enabled());
        });
    }

    #[test]
    fn command_quotes_paths_with_spaces() {
        let exe = Path::new(r"D:\新建文件夹 (6)\taskbar-tz-clock.exe");
        assert_eq!(command_for(exe), r#""D:\新建文件夹 (6)\taskbar-tz-clock.exe""#);
    }
}
