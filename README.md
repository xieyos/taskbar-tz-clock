# taskbar-tz-clock · 任务栏时区时钟(Claude改时区必备)

<img src="assets/icon-source.png" width="96" align="right" alt="图标">

用置顶悬浮窗盖住 Windows 任务栏右下角的时钟，显示**指定时区**的时间。系统时区不变，也不改注册表。

- 默认和系统时钟一样显示"时:分"和短日期（如 `10:13` / `2026/10/19`），格式跟随系统区域设置
- 可选显示秒
- 鼠标点击穿透：点它就是点系统时钟，日历面板照常弹出
- 全屏程序、任务栏自动隐藏、时钟被菜单挡住时自动隐藏
- 托盘菜单切换时区、秒显示、开机自启
- 单个 exe，约 280 KB（含图标），常驻私有内存约 3 MB，CPU 约 0.2%，不依赖运行时

## 下载

到 [Releases](https://github.com/xieyos/taskbar-tz-clock/releases/latest) 页面下载 `taskbar-tz-clock-<版本>-x64.zip`，解压后双击 `taskbar-tz-clock.exe` 即可，不需要安装 Rust。exe 没有代码签名，首次运行时 SmartScreen 可能拦截，点"更多信息 → 仍要运行"。

## 从源码构建

```bash
cargo build --release
```

需要 Rust 1.88 以上（edition 2024）和 MSVC 工具链。运行 `target\release\taskbar-tz-clock.exe`，同一时间只会运行一个实例。

## 使用

右键（或左键）点托盘里的时钟图标打开菜单。Win11 默认把新图标收进"^"隐藏区，可以把它拖到任务栏上常驻：

| 菜单项 | 作用 |
|---|---|
| 时区 ▸ | 跟随系统时区 / 常用时区 / 全部时区，选中后立即生效并写回配置文件 |
| 显示秒 | 开关秒显示 |
| 暂停覆盖 | 临时露出系统原时钟（只在本次运行中有效） |
| 开机自启 | 写入或删除 `HKCU\...\CurrentVersion\Run`，不需要管理员权限；exe 换了位置需要重新勾选 |
| 打开配置文件 | 用关联程序打开 config.json，没有关联程序时用记事本 |
| 重新定位时钟 | 位置不对时手动触发重新定位 |
| 退出 | 也可以按 **Ctrl+Alt+Shift+Q** |

## 配置

首次运行时生成 `%APPDATA%\taskbar-tz-clock\config.json`（旧版本的 `%APPDATA%\TaskbarClockTz` 会自动迁移过来）：

```json
{
  "time_zone": "Tokyo Standard Time",
  "show_seconds": false
}
```

| 字段 | 说明 |
|---|---|
| `time_zone` | Windows 时区 ID，可选值见同目录下的 `time-zones.txt`（第二列，大小写不敏感）。**留空或填错时显示系统时区的时间。**注意：远程桌面开启时区重定向时，"系统时区"是客户端的时区。 |
| `show_seconds` | `true` 显示秒（用系统"长时间"格式），默认 `false` |

保存后约 2 秒内生效，不用重启程序。用记事本保存的 UTF-8 BOM 不影响读取；如果 JSON 写错，会保留上一次的有效配置。

## 已知限制

- 这是视觉覆盖：鼠标悬停时系统弹出的提示、日历面板里显示的仍是系统时区的时间。
- 只覆盖主显示器任务栏上的时钟。
- 任务栏是半透明亚克力材质，底色取时钟左侧的任务栏像素，壁纸颜色变化很大时边缘可能看得出来。
- 时钟按钮靠 UI Automation 定位（Win11：`SystemTray.OmniButtonLeft`），Windows 大版本更新可能改变这个结构。
- 被任务栏盖住后，最多 100 ms 内重新回到上层，期间原时钟可能闪一下。

## 图标

`assets/icon-source.png` 是原图（透明背景的大图），`assets/app.ico` 由它生成，包含 16–256 px 共 8 个尺寸。换图标时替换原图后执行：

```bash
pwsh tools/make-icon.ps1
```

`build.rs` 会在编译时把 `app.ico` 转成 `.res` 资源嵌进 exe，作为文件图标；托盘按当前缩放从中挑选合适的尺寸。不需要安装 Windows SDK 的 `rc.exe`。

## 测试

```bash
cargo test
```

覆盖时区换算（含夏令时切换、半小时时区、跨日）、时间日期格式化、刷新对齐、配置解析，
以及离屏渲染校准：在 100% 和 125% 缩放下，文字像素位置和系统时钟实测值相差不超过 1 px。

依赖真实任务栏的定位测试默认跳过，手动运行：

```bash
cargo test -- --ignored
```

（这些测试会临时读写当前用户的开机自启注册表项，结束后恢复原状。）

## 许可证

[MIT](LICENSE)
