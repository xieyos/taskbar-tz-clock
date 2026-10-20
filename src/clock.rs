//! 时区换算与时间文字格式化。
//!
//! 时区规则（含夏令时）和显示格式都直接取自 Windows，不自带时区数据库，
//! 系统更新时区规则后自动生效。

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Globalization::{
    DATE_SHORTDATE, GetDateFormatEx, GetLocaleInfoEx, GetTimeFormatEx, LOCALE_SSHORTTIME,
    TIME_FORMAT_FLAGS, TIME_NOSECONDS,
};
use windows::Win32::System::SystemInformation::GetSystemTime;
use windows::Win32::System::Time::{
    DYNAMIC_TIME_ZONE_INFORMATION, EnumDynamicTimeZoneInformation, GetDynamicTimeZoneInformation,
    SystemTimeToTzSpecificLocalTimeEx,
};
use windows::core::PCWSTR;

/// Windows 动态时区信息。
pub type Zone = DYNAMIC_TIME_ZONE_INFORMATION;

/// 格式化缓冲区长度；Windows 时间/日期字符串上限远小于此值。
const TEXT_BUFFER_LEN: usize = 128;
/// `GetDynamicTimeZoneInformation` 的失败返回值。
const TIME_ZONE_ID_INVALID: u32 = u32::MAX;

/// 枚举系统里的全部时区。
pub fn zones() -> impl Iterator<Item = Zone> {
    (0u32..).map_while(|index| {
        let mut zone = Zone::default();
        let status = unsafe { EnumDynamicTimeZoneInformation(index, &mut zone) };
        // 0 为 ERROR_SUCCESS，枚举完返回 ERROR_NO_MORE_ITEMS
        (status == 0).then_some(zone)
    })
}

/// 按 Windows 时区 ID（注册表键名，如 `Tokyo Standard Time`）查找，忽略大小写。
pub fn find_zone(id: &str) -> Option<Zone> {
    let id = id.trim();
    zones().find(|zone| key_name(zone).eq_ignore_ascii_case(id))
}

/// 时区 ID，即配置文件里填写的值。
pub fn key_name(zone: &Zone) -> String {
    from_wide(&zone.TimeZoneKeyName)
}

/// 系统语言下的标准时间名称，如"东京标准时间"。
pub fn standard_name(zone: &Zone) -> String {
    from_wide(&zone.StandardName)
}

/// 不含夏令时的基准 UTC 偏移，如 `UTC+05:30`。
pub fn base_offset(zone: &Zone) -> String {
    // Bias 的定义是 UTC = 本地 + Bias，所以符号相反
    let minutes = -zone.Bias;
    let sign = if minutes < 0 { '-' } else { '+' };
    let minutes = minutes.unsigned_abs();
    format!("UTC{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

pub fn now_utc() -> SYSTEMTIME {
    unsafe { GetSystemTime() }
}

/// 距下一个整秒（显示秒时）或整分的毫秒数，用于让刷新和系统时钟同时跳。
/// 所有时区的偏移都是整分钟，所以按 UTC 对齐即可。
pub fn millis_until_next_tick(now: &SYSTEMTIME, show_seconds: bool) -> u32 {
    let millis = u32::from(now.wMilliseconds).min(999);
    if show_seconds {
        1000 - millis
    } else {
        (60 - u32::from(now.wSecond).min(59)) * 1000 - millis
    }
}

/// 把 UTC 时间换算到指定时区；`zone` 为 `None` 时跟随系统当前时区。
pub fn to_zone(utc: &SYSTEMTIME, zone: Option<&Zone>) -> Option<SYSTEMTIME> {
    let system;
    let zone = match zone {
        Some(zone) => zone,
        None => {
            // 每次重新读取，系统改时区后无需重启
            system = current_system_zone()?;
            &system
        }
    };
    let mut local = SYSTEMTIME::default();
    unsafe { SystemTimeToTzSpecificLocalTimeEx(Some(zone), utc, &mut local) }.ok()?;
    Some(local)
}

fn current_system_zone() -> Option<Zone> {
    let mut zone = Zone::default();
    (unsafe { GetDynamicTimeZoneInformation(&mut zone) } != TIME_ZONE_ID_INVALID).then_some(zone)
}

/// 按用户区域设置格式化时间，和任务栏时钟一致：
/// 不显示秒时用"短时间"格式，显示秒时用"长时间"格式。
pub fn format_time(time: &SYSTEMTIME, show_seconds: bool) -> String {
    let short_pattern = if show_seconds { None } else { short_time_pattern() };
    let (flags, pattern) = match &short_pattern {
        Some(pattern) => (TIME_FORMAT_FLAGS(0), PCWSTR(pattern.as_ptr())),
        // 读不到短时间格式时，退回长时间格式去掉秒
        None if !show_seconds => (TIME_NOSECONDS, PCWSTR::null()),
        None => (TIME_FORMAT_FLAGS(0), PCWSTR::null()),
    };
    let mut buffer = [0u16; TEXT_BUFFER_LEN];
    let len = unsafe { GetTimeFormatEx(PCWSTR::null(), flags, Some(time), pattern, Some(&mut buffer)) };
    text_from(&buffer, len)
}

/// 按用户区域设置的短日期格式格式化，如 `2026/10/19`。
pub fn format_date(date: &SYSTEMTIME) -> String {
    let mut buffer = [0u16; TEXT_BUFFER_LEN];
    let len = unsafe {
        GetDateFormatEx(
            PCWSTR::null(),
            DATE_SHORTDATE,
            Some(date),
            PCWSTR::null(),
            Some(&mut buffer),
            PCWSTR::null(),
        )
    };
    text_from(&buffer, len)
}

/// 用户的短时间格式串（以 NUL 结尾），如 `H:mm`。
fn short_time_pattern() -> Option<Vec<u16>> {
    let mut buffer = vec![0u16; TEXT_BUFFER_LEN];
    let len = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_SSHORTTIME, Some(&mut buffer)) };
    (len > 1).then_some(buffer)
}

/// 取 Win32 格式化函数的输出；`len` 含结尾 NUL，失败时为 0。
fn text_from(buffer: &[u16], len: i32) -> String {
    let len = usize::try_from(len).unwrap_or(0).saturating_sub(1).min(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(year: u16, month: u16, day: u16, hour: u16, minute: u16) -> SYSTEMTIME {
        SYSTEMTIME {
            wYear: year,
            wMonth: month,
            wDay: day,
            wHour: hour,
            wMinute: minute,
            ..Default::default()
        }
    }

    fn convert(zone_id: &str, time: SYSTEMTIME) -> (u16, u16, u16, u16, u16) {
        let zone = find_zone(zone_id).expect("系统应包含该时区");
        let local = to_zone(&time, Some(&zone)).expect("换算应成功");
        (local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute)
    }

    #[test]
    fn tokyo_crosses_date_line() {
        assert_eq!(convert("Tokyo Standard Time", utc(2026, 12, 31, 15, 30)), (2027, 1, 1, 0, 30));
    }

    #[test]
    fn india_has_half_hour_offset() {
        assert_eq!(convert("India Standard Time", utc(2026, 6, 1, 0, 0)), (2026, 6, 1, 5, 30));
    }

    #[test]
    fn pacific_enters_daylight_time() {
        // 2026-03-08 当地 02:00 切到夏令时，即 UTC 10:00
        assert_eq!(convert("Pacific Standard Time", utc(2026, 3, 8, 9, 59)), (2026, 3, 8, 1, 59));
        assert_eq!(convert("Pacific Standard Time", utc(2026, 3, 8, 10, 0)), (2026, 3, 8, 3, 0));
    }

    #[test]
    fn pacific_leaves_daylight_time() {
        // 2026-11-01 当地夏令时 02:00 回拨到 01:00，即 UTC 09:00
        assert_eq!(convert("Pacific Standard Time", utc(2026, 11, 1, 8, 59)), (2026, 11, 1, 1, 59));
        assert_eq!(convert("Pacific Standard Time", utc(2026, 11, 1, 9, 0)), (2026, 11, 1, 1, 0));
    }

    #[test]
    fn next_tick_aligns_to_second_or_minute() {
        let at = |second, millis| SYSTEMTIME { wSecond: second, wMilliseconds: millis, ..Default::default() };
        assert_eq!(millis_until_next_tick(&at(10, 250), true), 750);
        assert_eq!(millis_until_next_tick(&at(10, 0), true), 1000);
        assert_eq!(millis_until_next_tick(&at(10, 250), false), 49_750);
        assert_eq!(millis_until_next_tick(&at(59, 999), false), 1);
        assert_eq!(millis_until_next_tick(&at(0, 0), false), 60_000);
    }

    #[test]
    fn find_zone_ignores_case_and_rejects_unknown() {
        assert!(find_zone(" tokyo standard time ").is_some());
        assert!(find_zone("Mars Standard Time").is_none());
        assert!(find_zone("").is_none());
    }

    #[test]
    fn base_offset_formats_sign_and_minutes() {
        assert_eq!(base_offset(&find_zone("India Standard Time").unwrap()), "UTC+05:30");
        assert_eq!(base_offset(&find_zone("Pacific Standard Time").unwrap()), "UTC-08:00");
        assert_eq!(base_offset(&find_zone("UTC").unwrap()), "UTC+00:00");
    }

    #[test]
    fn system_zone_conversion_succeeds() {
        assert!(to_zone(&utc(2026, 10, 19, 2, 0), None).is_some());
    }

    #[test]
    fn seconds_only_appear_when_enabled() {
        let time = SYSTEMTIME { wHour: 10, wMinute: 13, wSecond: 47, ..utc(2026, 10, 19, 10, 13) };
        let without = format_time(&time, false);
        let with = format_time(&time, true);
        assert!(without.contains("13") && !without.contains("47"), "{without}");
        assert!(with.contains("13") && with.contains("47"), "{with}");
    }

    #[test]
    fn short_date_contains_year_month_day() {
        let text = format_date(&utc(2026, 10, 19, 0, 0));
        assert!(text.contains("2026") && text.contains("10") && text.contains("19"), "{text}");
    }
}
