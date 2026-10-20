//! 把 assets/app.ico 嵌入 exe：作为文件图标，同时供托盘按 DPI 加载。
//!
//! 本机没有 Windows SDK 的 rc.exe，所以直接生成 .res 二进制资源，交给 MSVC 链接器
//! （link.exe 可以直接接收 .res 输入）。格式见 Win32 文档"Resource File Formats"。

use std::env;
use std::fs;
use std::path::PathBuf;

const ICON_FILE: &str = "assets/app.ico";
/// 与 src/tray.rs 的 `APP_ICON_ID` 保持一致；编号最小的图标组即为 exe 文件图标。
const GROUP_ID: u16 = 1;
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
/// MOVEABLE | PURE | DISCARDABLE，rc.exe 对图标资源的默认标志。
const ICON_FLAGS: u16 = 0x1030;
/// 中性语言，任何系统语言下都能找到。
const LANG_NEUTRAL: u16 = 0;

struct IconImage<'a> {
    width: u8,
    height: u8,
    colors: u8,
    planes: u16,
    bit_count: u16,
    data: &'a [u8],
}

fn main() {
    println!("cargo:rerun-if-changed={ICON_FILE}");
    let ico = fs::read(ICON_FILE).unwrap_or_else(|e| panic!("读取 {ICON_FILE} 失败：{e}"));
    let images = parse_ico(&ico);

    let mut res = Vec::new();
    // .res 文件以一条空资源开头，标识 32 位格式
    write_resource(&mut res, 0, 0, 0, &[]);
    for (index, image) in images.iter().enumerate() {
        write_resource(&mut res, RT_ICON, icon_id(index), ICON_FLAGS, image.data);
    }
    write_resource(&mut res, RT_GROUP_ICON, GROUP_ID, ICON_FLAGS, &group_directory(&images));

    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo 应提供 OUT_DIR")).join("app.res");
    fs::write(&out, res).expect("写入 app.res 失败");
    // 只链接进可执行文件，测试程序不需要
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// 第 index 个图像的 RT_ICON 编号，从 1 开始。
fn icon_id(index: usize) -> u16 {
    u16::try_from(index + 1).expect("图标尺寸过多")
}

/// 解析 ICO：6 字节文件头 + 每项 16 字节目录 + 图像数据（PNG 或 DIB 原样保留）。
fn parse_ico(ico: &[u8]) -> Vec<IconImage<'_>> {
    let u16_at = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(ico[at..at + 4].try_into().unwrap()) as usize;
    assert!(ico.len() >= 6 && u16_at(0) == 0 && u16_at(2) == 1, "{ICON_FILE} 不是 ICO 文件");
    let count = usize::from(u16_at(4));
    assert!(count > 0 && ico.len() >= 6 + 16 * count, "{ICON_FILE} 目录不完整");
    (0..count)
        .map(|i| {
            let entry = 6 + 16 * i;
            let (size, offset) = (u32_at(entry + 8), u32_at(entry + 12));
            assert!(offset + size <= ico.len(), "{ICON_FILE} 第 {i} 个图像越界");
            IconImage {
                width: ico[entry],
                height: ico[entry + 1],
                colors: ico[entry + 2],
                planes: u16_at(entry + 4),
                bit_count: u16_at(entry + 6),
                data: &ico[offset..offset + size],
            }
        })
        .collect()
}

/// RT_GROUP_ICON 数据：和 ICO 目录相同，只是最后的文件偏移换成 2 字节的 RT_ICON 编号。
fn group_directory(images: &[IconImage]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (index, image) in images.iter().enumerate() {
        out.extend_from_slice(&[image.width, image.height, image.colors, 0]);
        out.extend_from_slice(&image.planes.to_le_bytes());
        out.extend_from_slice(&image.bit_count.to_le_bytes());
        out.extend_from_slice(&(image.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&icon_id(index).to_le_bytes());
    }
    out
}

/// 写一条资源：类型和名称都用数字编号，头部固定 32 字节，数据按 4 字节对齐。
fn write_resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    const HEADER_SIZE: u32 = 32;
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&HEADER_SIZE.to_le_bytes());
    for ordinal in [kind, name] {
        // 0xFFFF 前缀表示后面是数字编号而不是字符串
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
        out.extend_from_slice(&ordinal.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&LANG_NEUTRAL.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);
    out.resize(out.len().next_multiple_of(4), 0);
}
