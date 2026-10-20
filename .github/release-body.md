## 安装

1. 下载下面的 `taskbar-tz-clock-<版本>-x64.zip`，解压到任意目录（例如 `%LOCALAPPDATA%\taskbar-tz-clock`）。
2. 双击 `taskbar-tz-clock.exe` 运行。单个 exe，不需要安装，也不依赖运行时；同一时间只会运行一个实例。
3. 适用于 64 位 Windows 11（Windows 10 有兼容处理，测试较少）。exe 没有代码签名，首次运行时 SmartScreen 可能提示"已保护你的电脑"，点"更多信息 → 仍要运行"。

## 配置

首次运行时生成 `%APPDATA%\taskbar-tz-clock\config.json`，也可以从托盘菜单"打开配置文件"直接打开。`time_zone` 填 Windows 时区 ID（可选值见同目录的 `time-zones.txt`），`show_seconds` 控制是否显示秒。保存后约 2 秒内生效。

## 托盘菜单

右键（或左键）点托盘里的时钟图标：切换时区、显示秒、暂停覆盖、开机自启、打开配置文件、重新定位时钟、退出。Win11 默认把新图标收进"^"隐藏区，可以拖到任务栏上常驻。

勾选"开机自启"后如果移动了 exe 的位置，需要重新勾选一次。

## 退出

托盘菜单"退出"，或按 **Ctrl+Alt+Shift+Q**。

详细说明见压缩包里的 README.md。
