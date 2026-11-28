//! 屏幕截取。所有坐标均为物理像素、虚拟桌面坐标系。
//!
//! - 显示器信息：xcap。
//! - 翻译框的实时截屏：用 `PrintWindow(PW_RENDERFULLCONTENT)` 直接让翻译框**下面的窗口**
//!   把自己画出来，再拼成该区域的画面。这样截到的永远是原始内容，不会把翻译框自己的译文截进去，
//!   也无需隐藏窗口（不闪烁）。
//!   （曾用 `WDA_EXCLUDEFROMCAPTURE` 排除翻译框，但它会让透明的 WebView2 窗口在屏幕上也不可见，已弃用。）
//! - 能关掉截取边框的系统（Windows 11）上，框下只有一个窗口时改用 WGC 按窗口截取，延迟更低（`wgc.rs`）。

use image::RgbaImage;
use std::sync::Arc;
use std::time::Instant;
use windows::Win32::Foundation::{HWND, RECT};
use xcap::Monitor;

/// 显示器的位置与尺寸
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

pub fn monitor_at(x: i32, y: i32) -> Result<MonitorRect, String> {
    let m = Monitor::from_point(x, y).map_err(|e| t!("err.capture.no_monitor", e = e))?;
    let err = |e: xcap::XCapError| e.to_string();
    Ok(MonitorRect {
        x: m.x().map_err(err)?,
        y: m.y().map_err(err)?,
        w: m.width().map_err(err)?,
        h: m.height().map_err(err)?,
    })
}

/// 截取区域下的窗口信息（按 Z 序自顶向下，坐标相对本次截图左上角）
#[derive(Debug, Clone)]
pub struct WindowLayer {
    pub rect: (i32, i32, i32, i32),
    pub app_id: String,
    pub app_name: String,
    pub title: String,
}

/// 一次截取的结果
pub struct Shot {
    pub image: RgbaImage,
    /// 图像左上角相对请求区域的偏移
    pub ox: i32,
    pub oy: i32,
    /// 画面对应的时刻（PrintWindow 为开始绘制时，WGC 为 DWM 合成该帧时）
    pub at: Instant,
    /// 框下涉及的窗口列表（用于悬浮窗按应用隔离与过滤）
    pub windows: Vec<WindowLayer>,
    /// 是否来自 WGC
    #[cfg_attr(not(test), allow(dead_code))]
    pub wgc: bool,
}

/// 翻译框的持续截取器：框下只有一个窗口且区域落在它的可见范围内时用 WGC（见 `wgc.rs`），
/// 否则用 `PrintWindow` 逐层拼接。
pub struct Capturer {
    on_frame: Arc<dyn Fn() + Send + Sync>,
    wgc: Option<crate::wgc::WindowCapture>,
    /// 建立 WGC 会话失败过的窗口，不再重试
    failed: Option<isize>,
    /// 窗口句柄 → (app_id, app_name) 的短缓存，避免每帧查进程
    app_cache: std::collections::HashMap<isize, (String, String)>,
}

impl Capturer {
    /// `on_frame`：WGC 有新帧时调用，用来立即唤醒跟踪循环
    pub fn new(on_frame: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            on_frame,
            wgc: None,
            failed: None,
            app_cache: std::collections::HashMap::new(),
        }
    }

    fn describe_layers(&mut self, layers: &[(HWND, RECT)], x: i32, y: i32) -> Vec<WindowLayer> {
        if self.app_cache.len() > 256 {
            self.app_cache.clear();
        }
        layers
            .iter()
            .map(|&(hwnd, r)| {
                let key = hwnd.0 as isize;
                let (app_id, app_name) = self
                    .app_cache
                    .entry(key)
                    .or_insert_with(|| resolve_hwnd_app(hwnd))
                    .clone();
                WindowLayer {
                    rect: (r.left - x, r.top - y, r.right - r.left, r.bottom - r.top),
                    app_id,
                    app_name,
                    title: window_title(hwnd),
                }
            })
            .collect()
    }

    pub fn grab(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<Shot, String> {
        let layers = layers_under(x, y, w, h)?;
        let windows = self.describe_layers(&layers, x, y);
        if layers.len() == 1 && crate::wgc::available() {
            let hwnd = layers[0].0.0 as isize;
            if self.wgc.as_ref().is_some_and(|c| c.hwnd != hwnd) {
                self.wgc = None;
            }
            if self.wgc.is_none() && self.failed != Some(hwnd) {
                match crate::wgc::WindowCapture::new(hwnd, self.on_frame.clone()) {
                    Ok(c) => self.wgc = Some(c),
                    Err(_) => self.failed = Some(hwnd),
                }
            }
            if let Some(c) = &mut self.wgc {
                // 还没有帧、区域超出窗口可见边界等：这一次先用 PrintWindow
                if let Ok(Some((image, at))) = c.grab(x, y, w, h) {
                    return Ok(Shot { image, ox: 0, oy: 0, at, windows, wgc: true });
                }
            }
        }
        // 框下暂时多出别的窗口（菜单、弹窗）时保留 WGC 会话，回来后接着用
        let at = Instant::now();
        let image = print_layers(&layers, x, y, w, h)?;
        Ok(Shot { image, ox: 0, oy: 0, at, windows, wgc: false })
    }
}

/// 解析窗口句柄对应的应用标识与友好名称；若为 UWP 宿主（ApplicationFrameHost.exe）则下钻子窗口
fn resolve_hwnd_app(hwnd: HWND) -> (String, String) {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetWindowThreadProcessId};
    use windows::core::BOOL;

    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    let mut exe = if pid != 0 {
        crate::immersive::procs::exe_path(pid)
    } else {
        None
    };

    let is_uwp_host = exe
        .as_ref()
        .and_then(|p| p.file_name())
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("ApplicationFrameHost.exe"));
    if is_uwp_host {
        struct Search {
            host_pid: u32,
            found: Option<std::path::PathBuf>,
        }
        unsafe extern "system" fn each_child(child: HWND, lp: LPARAM) -> BOOL {
            let s = unsafe { &mut *(lp.0 as *mut Search) };
            let mut cpid = 0u32;
            unsafe { GetWindowThreadProcessId(child, Some(&mut cpid)) };
            if cpid != 0 && cpid != s.host_pid {
                if let Some(p) = crate::immersive::procs::exe_path(cpid) {
                    s.found = Some(p);
                    return false.into();
                }
            }
            true.into()
        }
        let mut s = Search { host_pid: pid, found: None };
        unsafe {
            let _ = EnumChildWindows(Some(hwnd), Some(each_child), LPARAM(&mut s as *mut Search as isize));
        }
        if let Some(real_exe) = s.found {
            exe = Some(real_exe);
        }
    }

    let Some(path) = exe else {
        return (String::new(), String::new());
    };
    let app_id = crate::config::normalize_app_id(&path.to_string_lossy());
    let app_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| app_id.clone());
    (app_id, app_name)
}

fn window_title(hwnd: HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    let mut buf = [0u16; 128];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize]).trim().chars().take(80).collect()
}

/// 截取某区域内"本程序窗口之下"的画面（只用 `PrintWindow`）。
///
/// 返回的图像与请求区域等大，坐标偏移恒为 (0, 0)（保留偏移字段，便于以后换成会裁剪的截屏方式）。
#[cfg_attr(not(test), allow(dead_code))]
pub fn capture_under(x: i32, y: i32, w: u32, h: u32) -> Result<(RgbaImage, i32, i32), String> {
    let layers = layers_under(x, y, w, h)?;
    Ok((print_layers(&layers, x, y, w, h)?, 0, 0))
}

/// 按 Z 序自顶向下收集与区域相交的其他程序窗口，直到遇到一个完全盖住区域的窗口
fn layers_under(x: i32, y: i32, w: u32, h: u32) -> Result<Vec<(HWND, RECT)>, String> {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::*;

    if w == 0 || h == 0 {
        return Err(t!("err.capture.empty_region"));
    }
    let (rw, rh) = (w as i32, h as i32);
    let own_pid = std::process::id();

    let mut layers: Vec<(HWND, RECT)> = Vec::new();
    unsafe {
        let mut cur = GetTopWindow(None).ok();
        while let Some(hwnd) = cur {
            cur = GetWindow(hwnd, GW_HWNDNEXT).ok();
            if !IsWindowVisible(hwnd).as_bool() {
                continue;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == own_pid {
                continue;
            }
            let mut cloaked = 0u32;
            let is_cloaked = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as _, 4).is_ok() && cloaked != 0;
            if is_cloaked {
                continue;
            }
            let ex = WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32);
            // 鼠标穿透的悬浮层（别的程序的提示框、覆盖层等）不算内容
            if ex.contains(WS_EX_TRANSPARENT) {
                continue;
            }
            let mut r = RECT::default();
            if GetWindowRect(hwnd, &mut r).is_err() || r.right <= r.left || r.bottom <= r.top {
                continue;
            }
            if r.right <= x || r.left >= x + rw || r.bottom <= y || r.top >= y + rh {
                continue;
            }
            let covers = r.left <= x && r.top <= y && r.right >= x + rw && r.bottom >= y + rh;
            // 只盖住一部分的分层窗口多半是阴影/半透明特效，PrintWindow 画不出透明度，跳过
            if !covers && ex.contains(WS_EX_LAYERED) {
                continue;
            }
            layers.push((hwnd, r));
            if covers {
                break;
            }
        }
    }
    if layers.is_empty() {
        return Err(t!("err.capture.no_window"));
    }
    Ok(layers)
}

/// 把各层窗口从下往上 `PrintWindow` 到画布，读出区域画面
fn print_layers(layers: &[(HWND, RECT)], x: i32, y: i32, w: u32, h: u32) -> Result<RgbaImage, String> {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::PW_RENDERFULLCONTENT;

    let (rw, rh) = (w as i32, h as i32);
    unsafe {
        let screen = GetDC(None);
        let canvas_dc = CreateCompatibleDC(Some(screen));
        let canvas = CreateCompatibleBitmap(screen, rw, rh);
        let old_canvas = SelectObject(canvas_dc, canvas.into());
        let _ = PatBlt(canvas_dc, 0, 0, rw, rh, BLACKNESS);
        for &(hwnd, r) in layers.iter().rev() {
            let (ww, wh) = (r.right - r.left, r.bottom - r.top);
            let win_dc = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, ww, wh);
            let old = SelectObject(win_dc, bmp.into());
            if PrintWindow(hwnd, win_dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool() {
                let _ = BitBlt(canvas_dc, r.left - x, r.top - y, ww, wh, Some(win_dc), 0, 0, SRCCOPY);
            }
            SelectObject(win_dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(win_dc);
        }

        // 读出像素（BGRA，自上而下）
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: rw,
                biHeight: -rh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (rw * rh * 4) as usize];
        SelectObject(canvas_dc, old_canvas);
        let lines = GetDIBits(canvas_dc, canvas, 0, rh as u32, Some(buf.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
        let _ = DeleteObject(canvas.into());
        let _ = DeleteDC(canvas_dc);
        ReleaseDC(None, screen);
        if lines == 0 {
            return Err(t!("err.capture.read_pixels"));
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        RgbaImage::from_raw(w, h, buf).ok_or_else(|| t!("err.capture.bad_size"))
    }
}

/// 当前光标位置（物理像素）与鼠标左键是否按下
pub fn cursor_state() -> Option<(i32, i32, bool)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe {
        GetCursorPos(&mut p).ok()?;
        let down = GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0;
        Some((p.x, p.y, down))
    }
}

/// 是否按着翻页类按键（PgUp/PgDn/方向上下/空格/Home/End）。只查询这几个键的当前状态，不记录任何输入。
pub fn nav_key_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    [VK_PRIOR, VK_NEXT, VK_UP, VK_DOWN, VK_SPACE, VK_HOME, VK_END]
        .iter()
        .any(|k| unsafe { GetAsyncKeyState(k.0 as i32) } < 0)
}

/// 测试用：直接让指定窗口把自己画出来（窗口可以在后台被遮挡，不必切到前台）
#[cfg(test)]
pub fn capture_window(hwnd_raw: isize) -> Result<RgbaImage, String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let hwnd = HWND(hwnd_raw as _);
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).map_err(|e| e.to_string())?;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bmp.into());
        let ok = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        SelectObject(dc, old);
        GetDIBits(dc, bmp, 0, h as u32, Some(buf.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        if !ok {
            return Err(t!("err.capture.print_window"));
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        RgbaImage::from_raw(w as u32, h as u32, buf).ok_or_else(|| t!("err.capture.bad_size"))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// 测试进程默认不感知 DPI，窗口坐标会被系统按缩放比换算，与 DWM 的物理像素对不上；
    /// 正式程序由清单声明了按显示器感知 DPI，这里与之保持一致
    pub fn dpi_aware() {
        use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    /// 真机实测：截取屏幕中央区域下层窗口画面并存成 PNG。
    /// TF_SHOT=输出路径 cargo test 下层窗口截取 -- --ignored --nocapture
    #[test]
    #[ignore]
    fn 下层窗口截取() {
        let out = std::env::var("TF_SHOT").unwrap_or_else(|_| "under.png".into());
        let (x, y, w, h) = std::env::var("TF_RECT")
            .ok()
            .and_then(|s| {
                let v: Vec<i32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                (v.len() == 4).then(|| (v[0], v[1], v[2] as u32, v[3] as u32))
            })
            .unwrap_or((400, 300, 600, 400));
        // TA_CAPTURE=wgc / printwindow 可强制截屏方式，便于对比两者画面是否一致
        dpi_aware();
        let mut capturer = Capturer::new(Arc::new(|| {}));
        let t = std::time::Instant::now();
        let shot = capturer.grab(x, y, w, h).unwrap();
        println!("耗时 {:?}，WGC：{}", t.elapsed(), shot.wgc);
        shot.image.save(&out).unwrap();
    }
}
