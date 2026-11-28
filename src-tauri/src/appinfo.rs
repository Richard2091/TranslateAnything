//! 读程序的友好名称和图标，供设置界面显示。
//!
//! 名称取 exe 版本信息里的"文件说明"（任务管理器显示的就是它），不合适时用"产品名称"；
//! 图标走系统外壳，和资源管理器里看到的一致。

use std::path::Path;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, HGDIOBJ,
    ReleaseDC,
};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY};
use windows::core::HSTRING;

/// 不能当应用名的通用名字（未改名的 Electron / CEF 壳）
const GENERIC: [&str; 6] = ["electron", "chromium", "cef", "app", "main", "launcher"];

/// 程序的友好名称；读不到版本信息或只有通用名字时返回 None
pub fn friendly_name(exe: &Path) -> Option<String> {
    let usable = |s: Option<String>| {
        s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && !GENERIC.contains(&s.to_lowercase().as_str()))
    };
    let info = version_info(exe)?;
    let desc = usable(info.query("FileDescription"));
    let product = usable(info.query("ProductName"));
    // 文件说明偶尔是一整句介绍，这时产品名称更像名字
    match (desc, product) {
        (Some(d), Some(p)) if d.chars().count() > 40 && p.chars().count() < d.chars().count() => Some(p),
        (d, p) => d.or(p),
    }
}

struct VersionInfo {
    data: Vec<u8>,
    /// 语言 + 代码页，如 `040904b0`
    langs: Vec<String>,
}

fn version_info(exe: &Path) -> Option<VersionInfo> {
    let path = HSTRING::from(exe.as_os_str());
    unsafe {
        let len = GetFileVersionInfoSizeW(&path, None);
        if len == 0 {
            return None;
        }
        let mut data = vec![0u8; len as usize];
        GetFileVersionInfoW(&path, None, len, data.as_mut_ptr().cast()).ok()?;
        let mut info = VersionInfo { data, langs: Vec::new() };
        // 优先界面语言对应的那组字符串，再按文件里的顺序，最后试常见的英文代码页
        let ui_lang = windows::Win32::Globalization::GetUserDefaultUILanguage();
        let mut langs: Vec<(u16, u16)> = info.raw("\\VarFileInfo\\Translation").map_or_else(Vec::new, |b| {
            b.chunks_exact(4).map(|c| (u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]]))).collect()
        });
        langs.sort_by_key(|(l, _)| *l != ui_lang);
        info.langs = langs.into_iter().map(|(l, c)| format!("{l:04x}{c:04x}")).collect();
        info.langs.extend(["040904b0", "040904e4", "000004b0"].map(String::from));
        Some(info)
    }
}

impl VersionInfo {
    /// 取一项的指针和长度（二进制项以字节计，字符串项以字符计）
    fn value(&self, sub: &str) -> Option<(*const std::ffi::c_void, u32)> {
        let sub = HSTRING::from(sub);
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        let ok = unsafe { VerQueryValueW(self.data.as_ptr().cast(), &sub, &mut ptr, &mut len).as_bool() };
        (ok && !ptr.is_null() && len > 0).then_some((ptr.cast_const(), len))
    }

    fn raw(&self, sub: &str) -> Option<&[u8]> {
        let (ptr, len) = self.value(sub)?;
        Some(unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len as usize) })
    }

    fn query(&self, key: &str) -> Option<String> {
        self.langs.iter().find_map(|lang| {
            let (ptr, len) = self.value(&format!("\\StringFileInfo\\{lang}\\{key}"))?;
            let chars = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), len as usize) };
            // 长度可能带结尾的 0
            let s = String::from_utf16_lossy(chars.split(|c| *c == 0).next().unwrap_or_default());
            (!s.trim().is_empty()).then_some(s)
        })
    }
}

/// 程序图标：`size`×`size` 的 RGBA 像素（非预乘）。取不到时返回 None
pub fn icon_rgba(exe: &Path, size: i32) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let inited = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let r = (|| {
            let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(exe.as_os_str()), None).ok()?;
            let hbm = factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY).ok()?;
            let r = bitmap_rgba(hbm);
            let _ = DeleteObject(HGDIOBJ(hbm.0));
            r
        })();
        if inited {
            CoUninitialize();
        }
        r
    }
}

/// 把外壳给的 32 位位图读成 RGBA；外壳给的是预乘 alpha，这里还原
unsafe fn bitmap_rgba(hbm: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(HGDIOBJ(hbm.0), size_of::<BITMAP>() as i32, Some((&raw mut bm).cast())) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 {
            return None;
        }
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 自上而下
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let dc = GetDC(None);
        let lines = GetDIBits(dc, hbm, 0, h as u32, Some(px.as_mut_ptr().cast()), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        if lines == 0 {
            return None;
        }
        // 老式图标没有 alpha 通道，整张当不透明
        let opaque = px.chunks_exact(4).all(|p| p[3] == 0);
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2); // BGRA → RGBA
            if opaque {
                p[3] = 255;
            } else if p[3] > 0 && p[3] < 255 {
                let a = p[3] as u32;
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        Some((w as u32, h as u32, px))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 读系统程序的名称和图标() {
        let exe = Path::new(&std::env::var("SystemRoot").unwrap()).join("explorer.exe");
        let name = friendly_name(&exe).unwrap();
        assert!(!name.eq_ignore_ascii_case("explorer"), "{name}");
        let (w, h, px) = icon_rgba(&exe, 64).unwrap();
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert!(px.chunks_exact(4).any(|p| p[3] > 0));
    }
}
