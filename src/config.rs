//! 配置文件 `%APPDATA%\taskbar-tz-clock\config.json`，修改后约 2 秒内生效。
//!
//! 首次运行时写入默认配置，并生成 `time-zones.txt` 列出所有可填的时区 ID。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::clock;

const APP_DIR: &str = "taskbar-tz-clock";
/// 改名前（TaskbarClockTz）使用的配置目录，启动时迁移。
const LEGACY_APP_DIR: &str = "TaskbarClockTz";
const CONFIG_FILE: &str = "config.json";
const ZONE_LIST_FILE: &str = "time-zones.txt";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Windows 时区 ID（见 time-zones.txt），空字符串表示跟随系统时区。
    pub time_zone: String,
    /// 是否显示秒，默认和系统时钟一样只显示时分。
    pub show_seconds: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { time_zone: String::new(), show_seconds: false }
    }
}

/// 解析配置；记事本保存的 UTF-8 BOM 会被忽略，缺失字段取默认值。
pub fn parse(text: &str) -> Option<Config> {
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// 跟踪配置文件的修改时间，变化时重新读取。
pub struct ConfigFile {
    path: Option<PathBuf>,
    modified: Option<SystemTime>,
    loaded: bool,
}

impl ConfigFile {
    /// 定位配置目录，缺失时写入默认配置和时区列表。
    /// 目录不可用时仍可运行，只是一直使用默认配置。
    pub fn open() -> Self {
        let dir = std::env::var_os("APPDATA").map(|appdata| {
            let appdata = PathBuf::from(appdata);
            let dir = appdata.join(APP_DIR);
            migrate_legacy_dir(&appdata.join(LEGACY_APP_DIR), &dir);
            dir
        });
        let path = dir.and_then(|dir| {
            fs::create_dir_all(&dir).ok()?;
            write_if_missing(&dir.join(ZONE_LIST_FILE), &zone_list());
            let path = dir.join(CONFIG_FILE);
            let default = serde_json::to_string_pretty(&Config::default()).ok()?;
            write_if_missing(&path, &default);
            Some(path)
        });
        Self { path, modified: None, loaded: false }
    }

    /// 配置文件路径；`%APPDATA%` 不可用时为 `None`。
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 写回配置（托盘菜单修改时调用），并记下新的修改时间，避免随后被当作外部修改再读一遍。
    pub fn save(&mut self, config: &Config) -> std::io::Result<()> {
        let path = self.path.as_ref().ok_or(std::io::ErrorKind::NotFound)?;
        let text = serde_json::to_string_pretty(config).map_err(std::io::Error::other)?;
        fs::write(path, text)?;
        self.modified = fs::metadata(path)?.modified().ok();
        self.loaded = true;
        Ok(())
    }

    /// 首次调用或文件有变化时返回新配置，否则返回 `None`。
    /// 文件内容有误（比如正在编辑一半）时保留当前配置，首次读取则用默认值。
    pub fn reload_if_changed(&mut self) -> Option<Config> {
        let modified = self.path.as_ref().and_then(|p| fs::metadata(p).ok()?.modified().ok());
        if self.loaded && modified == self.modified {
            return None;
        }
        self.modified = modified;
        let parsed = self.path.as_ref().and_then(|p| parse(&fs::read_to_string(p).ok()?));
        let first = !std::mem::replace(&mut self.loaded, true);
        match parsed {
            Some(config) => Some(config),
            None if first => Some(Config::default()),
            None => None,
        }
    }
}

/// 改名前的配置目录还在、新目录还没有时，把整个目录挪过来，保留用户设置。
/// 挪不动（比如被占用）时放弃迁移，新目录会写入默认配置。
fn migrate_legacy_dir(legacy: &Path, dir: &Path) {
    if legacy.is_dir() && !dir.exists() {
        let _ = fs::rename(legacy, dir);
    }
}

fn write_if_missing(path: &Path, content: &str) {
    if !path.exists() {
        // 写不了只影响提示文件，不影响运行
        let _ = fs::write(path, content);
    }
}

/// 时区列表：每行"基准偏移、时区 ID、标准时间名称"，按偏移排序。
fn zone_list() -> String {
    let mut zones: Vec<_> = clock::zones().collect();
    zones.sort_by_key(|zone| -zone.Bias);
    let mut text = String::from(
        "# 把下面第二列的时区 ID 填到 config.json 的 time_zone 里（大小写不敏感）\n\
         # 填错或留空时显示系统时区的时间\n",
    );
    for zone in &zones {
        let line =
            format!("{}\t{}\t{}\n", clock::base_offset(zone), clock::key_name(zone), clock::standard_name(zone));
        text.push_str(&line);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shows_hours_minutes_and_follows_system() {
        let config = Config::default();
        assert!(!config.show_seconds);
        assert!(config.time_zone.is_empty());
    }

    #[test]
    fn missing_fields_take_defaults() {
        let config = parse(r#"{"time_zone":"Tokyo Standard Time"}"#).unwrap();
        assert_eq!(config.time_zone, "Tokyo Standard Time");
        assert!(!config.show_seconds);
    }

    #[test]
    fn bom_is_ignored() {
        let config = parse("\u{feff}{\"show_seconds\":true}").unwrap();
        assert!(config.show_seconds);
    }

    #[test]
    fn legacy_dir_moves_only_when_new_dir_is_missing() {
        let root = std::env::temp_dir().join(format!("taskbar-tz-clock-test-{}", std::process::id()));
        let (legacy, dir) = (root.join("old"), root.join("new"));
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join(CONFIG_FILE), "{}").unwrap();

        migrate_legacy_dir(&legacy, &dir);
        assert!(dir.join(CONFIG_FILE).exists() && !legacy.exists());

        // 新目录已存在时不覆盖
        fs::create_dir_all(&legacy).unwrap();
        migrate_legacy_dir(&legacy, &dir);
        assert!(legacy.exists());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn invalid_json_is_rejected() {
        assert!(parse("{\"time_zone\":").is_none());
        assert!(parse("").is_none());
    }
}
