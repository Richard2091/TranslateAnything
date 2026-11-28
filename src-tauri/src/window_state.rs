//! 窗口位置与大小记忆：用户拖动、缩放、最大化后记下来，下次打开按原样还原。
//!
//! 单独存数据目录下的 `window-state.json`，不放进 config.json——设置页保存配置时会整份覆盖，
//! 混在一起容易被旧值冲掉。坐标一律用物理像素，还原前确认仍落在某个显示器上，
//! 显示器拔掉或分辨率变了就退回默认大小并居中。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{PhysicalPosition, PhysicalSize, Runtime, WebviewWindow};

const FILE_NAME: &str = "window-state.json";

/// 拖动 / 缩放过程中事件很密，停下这么久后再落盘
const SAVE_DELAY: Duration = Duration::from_millis(600);

/// 窗口外框左上角位置 + 内容区大小（物理像素），以及是否最大化
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    #[serde(default)]
    maximized: bool,
}

/// 内存里的最新状态：最大化时沿用最近一次的正常位置，取消最大化才能回到原处
static STATE: Mutex<Option<BTreeMap<String, Geometry>>> = Mutex::new(None);
static SAVE_SCHEDULED: AtomicBool = AtomicBool::new(false);

fn path() -> Option<std::path::PathBuf> {
    crate::config::data_dir().map(|d| d.join(FILE_NAME))
}

fn with_state<T>(f: impl FnOnce(&mut BTreeMap<String, Geometry>) -> T) -> T {
    let mut guard = STATE.lock().unwrap();
    let map = guard.get_or_insert_with(|| {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    });
    f(map)
}

/// 窗口刚建好、还没显示时调用：还原上次的位置与大小（没有可用记录就居中）后显示出来
pub fn restore_and_show<R: Runtime>(window: &WebviewWindow<R>) {
    let saved = with_state(|map| map.get(window.label()).copied());
    let maximized = match saved.and_then(|g| fit_to_monitors(window, g)) {
        Some(g) => {
            // 先移位置再定大小：跨不同缩放比例的显示器时，系统会在移动时按新缩放调整一次大小
            let _ = window.set_position(PhysicalPosition::new(g.x, g.y));
            let _ = window.set_size(PhysicalSize::new(g.width, g.height));
            g.maximized
        }
        None => {
            // 默认大小在小屏幕上可能超出工作区，先缩到放得下再居中
            if let (Ok(Some(monitor)), Ok(size)) = (window.current_monitor(), window.inner_size()) {
                let area = monitor.work_area().size;
                if size.width > area.width || size.height > area.height {
                    let _ = window.set_size(PhysicalSize::new(size.width.min(area.width), size.height.min(area.height)));
                }
            }
            let _ = window.center();
            false
        }
    };
    let _ = window.show();
    // 显示之后再最大化，取消最大化时才能回到上面还原的位置
    if maximized {
        let _ = window.maximize();
    }
    let _ = window.set_focus();
}

/// 窗口移动或缩放后调用：记下新状态，稍后统一落盘
pub fn remember<R: Runtime>(window: &WebviewWindow<R>) {
    if window.is_minimized().unwrap_or(false) || !window.is_visible().unwrap_or(false) {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let label = window.label().to_owned();
    let current = if maximized {
        with_state(|map| map.get(&label).copied())
            .map(|g| Geometry { maximized: true, ..g })
    } else {
        match (window.outer_position(), window.inner_size()) {
            (Ok(pos), Ok(size)) if size.width > 0 && size.height > 0 => Some(Geometry {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
                maximized: false,
            }),
            _ => None,
        }
    };
    let Some(current) = current else { return };
    let changed = with_state(|map| map.insert(label, current) != Some(current));
    if changed && !SAVE_SCHEDULED.swap(true, Ordering::AcqRel) {
        std::thread::spawn(|| {
            std::thread::sleep(SAVE_DELAY);
            SAVE_SCHEDULED.store(false, Ordering::Release);
            save();
        });
    }
}

fn save() {
    let Some(path) = path() else { return };
    let Ok(text) = with_state(|map| serde_json::to_string_pretty(map)) else { return };
    if let Err(e) = std::fs::write(&path, text) {
        eprintln!("保存窗口位置失败（{}）：{e}", path.display());
    }
}

/// 记录的位置要有一块标题栏落在某个显示器的工作区里，才算还能用；
/// 大小超过那个显示器时缩到工作区以内
fn fit_to_monitors<R: Runtime>(window: &WebviewWindow<R>, g: Geometry) -> Option<Geometry> {
    // 标题栏这条区域（顶部 40 像素，至少露出 120 像素宽）能看见、能拖，就不算丢窗口
    const BAR_HEIGHT: i32 = 40;
    const MIN_VISIBLE: i32 = 120;
    let monitors = window.available_monitors().ok()?;
    let monitor = monitors.iter().find(|m| {
        let area = m.work_area();
        let (ax, ay) = (area.position.x, area.position.y);
        let (aw, ah) = (area.size.width as i32, area.size.height as i32);
        let overlap_w = (g.x + g.width as i32).min(ax + aw) - g.x.max(ax);
        let overlap_h = (g.y + BAR_HEIGHT).min(ay + ah) - g.y.max(ay);
        overlap_w >= MIN_VISIBLE && overlap_h >= BAR_HEIGHT / 2
    })?;
    let area = monitor.work_area();
    Some(Geometry {
        width: g.width.min(area.size.width),
        height: g.height.min(area.size.height),
        ..g
    })
}
