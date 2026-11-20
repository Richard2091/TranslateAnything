//! 实时翻译框：透明、置顶、鼠标穿透；实时截取的是框下面的窗口画面（不含自身）。
//!
//! 每个翻译框有两个后台循环：
//! - 实时循环：画面变化时以截屏能达到的最快速度连续截取框内画面，估算滚动位移并立即推送，
//!   让译文跟手平移（不等 OCR）；同时在后台单独跑 OCR（同一时间只跑一个，总是识别最新画面）
//!   → 归并段落 → 命中缓存的直接出译文，新出现的文字批量请求翻译 → 推送版面给前端按原位置覆盖渲染。
//!   画面静止一段时间后放慢截屏频率以省电。
//! - 悬停循环：按各框的鼠标模式处理穿透，并保持边缘和工具条可操作。
//!
//! 坐标约定：实时循环维护"累计滚动量" `total`（自上次几何变化起，内容总共平移了多少）。
//! 每次 OCR 记下开始时的 `total` 作为该版面的基准 `base`；前端把版面整体平移 `total - base`，
//! 这样 OCR 期间发生的滚动也不会让译文"弹回"。
//!
//! 整体位移按全框估计，框里固定不动的部分（标题栏、侧栏、视频字幕）不该跟着走：
//! 每次估出位移后逐块核对原位置的画面，没动的块"钉"在屏幕坐标上（`Anchor::pinned`），
//! 前端把它们放进不随滚动平移的图层，避免这些译文漂离原文。

use crate::capture;
use crate::config;
use crate::input;
use crate::layout::{self, Block};
use crate::motion::{self, Frame};
use crate::ocr;
use crate::config::PortableWebview;
use crate::timing::ms;
use crate::translate;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

/// 画面在变化时的截屏间隔（实际受截屏耗时限制，一般 30~40ms 一帧）
const FAST_TICK: Duration = Duration::from_millis(12);
/// 画面静止时的截屏间隔
const IDLE_TICK: Duration = Duration::from_millis(100);
/// 画面静止多久后转入低频截屏
const IDLE_AFTER: Duration = Duration::from_millis(800);
/// 悬停检测间隔
const HOVER_TICK: Duration = Duration::from_millis(40);
/// 移动和缩放期间合并连续的窗口位置变化，优先翻译停下后的区域
const GEOMETRY_SETTLE: Duration = Duration::from_millis(90);
/// 画面连续这么久没变才算滚动停下
const STILL_AFTER: Duration = Duration::from_millis(30);
/// 变化像素比例超过该值才重新识别
const CHANGE_THRESHOLD: f32 = 0.002;
/// 翻译失败后多久再重试
const RETRY_AFTER: Duration = Duration::from_secs(3);
/// 每个翻译框最多缓存的译文条数
const CACHE_LIMIT: usize = 3000;
/// 边缘可拖拽缩放的宽度、工具条尺寸（CSS 像素，与前端保持一致）
const EDGE_CSS: f64 = 6.0;
const TOOLBAR_W_CSS: f64 = 152.0;
const TOOLBAR_H_CSS: f64 = 32.0;
/// 最小尺寸（CSS 像素），保留模式与关闭按钮以及工具条下方的拖动区域
const MIN_W: f64 = 80.0;
const MIN_H: f64 = 56.0;

#[derive(Default)]
pub struct LiveState {
    frames: Mutex<HashMap<String, Arc<FrameCtl>>>,
    seq: AtomicU32,
}

#[derive(Default)]
struct FrameCtl {
    paused: AtomicBool,
    /// 各框独立的鼠标穿透状态，不写回全局配置
    click_through: AtomicBool,
    /// 当前窗口是否拦截鼠标，由悬停循环同步原生标志后更新
    intercepting_mouse: AtomicBool,
    /// 原生移动、缩放和 DPI 事件的计数和最新时刻，一起读取以判断何时停稳
    geometry: Mutex<GeometryChange>,
    /// 有新译文到达，需要重新推送版面
    dirty: AtomicBool,
    /// 检测到翻页键：立即唤醒跟踪循环
    wake: tokio::sync::Notify,
    /// 后台 OCR / 翻译完成：立即唤醒跟踪循环推送结果，不必等到下一拍
    ready: tokio::sync::Notify,
}

#[derive(Clone, Copy, Default)]
struct GeometryChange {
    revision: u32,
    changed_at: Option<Instant>,
}

impl GeometryChange {
    /// 截屏只在最后一次几何变化的等待时间结束后恢复。
    fn settle_remaining(self) -> Duration {
        self.changed_at.map_or(Duration::ZERO, |at| GEOMETRY_SETTLE.saturating_sub(at.elapsed()))
    }
}

/// 推送给前端的版面
#[derive(Serialize, Clone)]
struct LayoutPayload {
    /// 几何变化计数：翻译框移动/缩放后加一，前端丢弃旧计数的消息
    epoch: u32,
    /// 版面坐标对应的累计滚动量（物理像素）
    base: [i32; 2],
    /// 当前累计滚动量
    total: [i32; 2],
    blocks: Vec<RenderBlock>,
    /// 等待翻译的块数
    pending: usize,
    /// 最近一次错误（OCR 或翻译）
    error: Option<String>,
    /// 最近一次成功使用的引擎名
    engine: Option<String>,
}

#[derive(Serialize, Clone)]
struct RenderBlock {
    #[serde(flatten)]
    block: Block,
    text: String,
    /// 钉在屏幕上（x / y 为相对翻译框的当前位置，不随滚动平移）
    pinned: bool,
}

/// 块在哪个坐标系里：随滚动平移（版面坐标，对应基准滚动量 `base`）或钉在屏幕上
#[derive(Clone, Copy, Debug)]
struct Anchor {
    pinned: bool,
    x: f32,
    y: f32,
}

impl Anchor {
    fn of(b: &Block) -> Self {
        Self { pinned: false, x: b.x, y: b.y }
    }

    /// 累计滚动量为 `total` 时块在屏幕上（相对翻译框）的位置
    fn screen(self, total: [i32; 2], base: [i32; 2]) -> (f32, f32) {
        if self.pinned {
            (self.x, self.y)
        } else {
            (self.x + (total[0] - base[0]) as f32, self.y + (total[1] - base[1]) as f32)
        }
    }
}

/// 块在截图（Frame）里的矩形：屏幕位置减去截图裁剪偏移
fn frame_rect(b: &Block, (sx, sy): (f32, f32), (ox, oy): (i32, i32)) -> (i32, i32, i32, i32) {
    (sx.round() as i32 - ox, sy.round() as i32 - oy, b.w.round() as i32, b.h.round() as i32)
}

/// 内容在 `a` → `b` 之间整体平移了 `d`：逐块核对，原地没动的钉住、钉住后又跟着走的放回滚动层。
/// `before` / `after` 为 `a` / `b` 两帧的累计滚动量；返回是否有块换了图层
fn reclassify(
    blocks: &[Block],
    anchors: &mut [Anchor],
    (a, b): (&Frame, &Frame),
    d: (i32, i32),
    (before, after, base): ([i32; 2], [i32; 2], [i32; 2]),
    off: (i32, i32),
) -> bool {
    let mut changed = false;
    for (blk, anchor) in blocks.iter().zip(anchors.iter_mut()) {
        let pos = anchor.screen(before, base);
        match (anchor.pinned, motion::classify_region(a, b, frame_rect(blk, pos, off), d)) {
            (false, Some(motion::RegionMotion::Static)) => {
                *anchor = Anchor { pinned: true, x: pos.0, y: pos.1 };
                changed = true;
            }
            (true, Some(motion::RegionMotion::Moved)) => {
                // 换回版面坐标：使它在 after 时出现在 pos + d
                *anchor = Anchor {
                    pinned: false,
                    x: pos.0 + d.0 as f32 - (after[0] - base[0]) as f32,
                    y: pos.1 + d.1 as f32 - (after[1] - base[1]) as f32,
                };
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

/// 内容滚动：当前累计滚动量（发绝对值而非增量，丢一两条也不会累积误差）
#[derive(Serialize, Clone)]
struct ShiftPayload {
    epoch: u32,
    total: [i32; 2],
    /// 这帧画面截于发出前多少毫秒（前端据此还原真实时刻，学习界面的滑动时长）
    age: u32,
    /// 发出时刻（Unix 毫秒）：前端据此再扣除事件传到页面所花的时间
    sent: f64,
}

/// 翻译框内的竖直滚轮输入：前端据此预测平移（一格 = ±120，向上为正）
#[derive(Serialize, Clone)]
struct WheelPayload {
    epoch: u32,
    delta: i32,
    /// 发出时刻（Unix 毫秒）
    sent: f64,
}

/// 当前时刻（Unix 毫秒），与页面里的 `Date.now()` 同一时钟
fn unix_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// 新建翻译框（物理像素，虚拟桌面坐标）
pub fn open_frame(app: &AppHandle, x: i32, y: i32, w: u32, h: u32) -> Result<(), String> {
    let state = app.state::<LiveState>();
    let label = format!("frame-{}", state.seq.fetch_add(1, Ordering::Relaxed) + 1);
    let click_through = config::load().unwrap_or_default().frame_click_through;
    let ctl = Arc::new(FrameCtl {
        click_through: AtomicBool::new(click_through),
        intercepting_mouse: AtomicBool::new(!click_through),
        ..FrameCtl::default()
    });
    // WebView 一加载就会查询当前模式，必须先登记状态。
    state.frames.lock().unwrap().insert(label.clone(), ctl.clone());
    if let Err(e) = build_frame(app, &label, x, y, w, h, &ctl) {
        state.frames.lock().unwrap().remove(&label);
        return Err(e);
    }
    tauri::async_runtime::spawn(live_loop(app.clone(), label.clone(), ctl.clone()));
    tauri::async_runtime::spawn(hover_loop(app.clone(), label, ctl, !click_through));
    Ok(())
}

/// 建立窗口并订阅后续的原生几何变化。
fn build_frame(app: &AppHandle, label: &str, x: i32, y: i32, w: u32, h: u32, ctl: &Arc<FrameCtl>) -> Result<(), String> {
    let win = WebviewWindowBuilder::new(app, label, WebviewUrl::App("frame.html".into()))
        .portable_data()
        .title(t!("app.name"))
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .min_inner_size(MIN_W, MIN_H)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    crate::commands::disable_transitions(&win);
    let init = || -> tauri::Result<()> {
        win.set_position(PhysicalPosition::new(x, y))?;
        let scale = app.monitor_from_point(x as f64, y as f64)?
            .map_or_else(|| win.scale_factor().unwrap_or(1.0), |m| m.scale_factor());
        win.set_size(PhysicalSize::new(
            w.max((MIN_W * scale).ceil() as u32),
            h.max((MIN_H * scale).ceil() as u32),
        ))?;
        win.set_ignore_cursor_events(ctl.click_through.load(Ordering::Relaxed))?;
        let ctl = ctl.clone();
        win.on_window_event(move |event| {
            if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
                {
                    let mut geometry = ctl.geometry.lock().unwrap();
                    geometry.revision = geometry.revision.wrapping_add(1);
                    geometry.changed_at = Some(Instant::now());
                }
                ctl.ready.notify_one();
            }
        });
        win.show()
    };
    if let Err(e) = init() {
        let _ = win.destroy();
        return Err(e.to_string());
    }
    Ok(())
}

/// 当前打开的翻译框个数
pub fn frame_count(app: &AppHandle) -> usize {
    app.state::<LiveState>().frames.lock().unwrap().len()
}

/// 关闭所有翻译框；各自的刷新循环发现窗口没了会自行退出
pub fn close_all_frames(app: &AppHandle) -> usize {
    let labels: Vec<String> = app.state::<LiveState>().frames.lock().unwrap().keys().cloned().collect();
    for label in &labels {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.destroy();
        }
    }
    labels.len()
}

#[tauri::command]
pub fn frame_set_paused(window: WebviewWindow, state: tauri::State<'_, LiveState>, paused: bool) {
    if let Some(ctl) = state.frames.lock().unwrap().get(window.label()) {
        ctl.paused.store(paused, Ordering::Relaxed);
        ctl.dirty.store(true, Ordering::Relaxed);
        ctl.ready.notify_one();
    }
}

/// 查询发起调用的翻译框的鼠标模式，页面重新加载时也从这里恢复。
#[tauri::command]
pub fn frame_get_click_through(window: WebviewWindow, state: tauri::State<'_, LiveState>) -> Result<bool, String> {
    let frames = state.frames.lock().unwrap();
    let ctl = frames.get(window.label()).ok_or_else(|| t!("frame.unavailable"))?;
    Ok(ctl.click_through.load(Ordering::Relaxed))
}

/// 查询全局圆角。页面先订阅变化，再读取持久化值，覆盖新建和重载的翻译框。
#[tauri::command]
pub fn frame_get_corner_radius() -> u8 {
    config::load().unwrap_or_default().frame_corner_radius
}

/// 只切换当前框。悬停循环统一更新原生命中，避免两个线程同时设置窗口标志。
#[tauri::command]
pub fn frame_set_click_through(window: WebviewWindow, state: tauri::State<'_, LiveState>, click_through: bool) -> Result<bool, String> {
    let frames = state.frames.lock().unwrap();
    let ctl = frames.get(window.label()).ok_or_else(|| t!("frame.unavailable"))?;
    ctl.click_through.store(click_through, Ordering::Relaxed);
    Ok(click_through)
}

/// 后台任务（OCR、翻译）与实时循环之间的共享状态
#[derive(Default)]
struct Shared {
    cache: HashMap<String, String>,
    in_flight: bool,
    error: Option<String>,
    failed_at: Option<Instant>,
    engine: Option<String>,
    /// OCR 正在进行
    ocr_busy: bool,
    /// 最新一次 OCR 的结果，等实时循环取走
    ocr_done: Option<OcrDone>,
}

struct OcrDone {
    epoch: u32,
    base: [i32; 2],
    /// 识别的那一帧（灰度），结果到达时与最新画面对比，找出识别期间没随滚动移动的块
    frame: Arc<Frame>,
    /// 截图裁剪偏移
    off: (i32, i32),
    result: Result<Vec<Block>, String>,
}

async fn live_loop(app: AppHandle, label: String, ctl: Arc<FrameCtl>) {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let mut prev: Option<Arc<Frame>> = None;
    let mut last_rect: Option<(i32, i32, u32, u32)> = None;
    let mut observed_revision = ctl.geometry.lock().unwrap().revision;
    let mut last_scale = 1.0;
    // 当前版面及其基准滚动量
    let mut blocks: Vec<Block> = Vec::new();
    // 与 blocks 一一对应：随滚动平移还是钉在屏幕上
    let mut anchors: Vec<Anchor> = Vec::new();
    let mut base = [0i32; 2];
    let mut ocr_error: Option<String> = None;
    // 累计滚动量与几何变化计数
    let mut total = [0i32; 2];
    let mut epoch = 0u32;
    // 画面自上次 OCR 开始后又变了，需要再识别一次
    let mut ocr_stale = true;
    let mut last_change = Instant::now();
    let mut wheel_rx = input::subscribe();
    // 上一次推送的是"在动"的位移：画面一停就再补发一次不变的位移，前端据此立刻停止外推
    let mut moving = false;
    let mut last_move = Instant::now();
    // 截取器（WGC 会话跨帧保持）；WGC 有新帧时立即唤醒循环
    let frame_arrived = Arc::new(tokio::sync::Notify::new());
    let capturer = Arc::new(Mutex::new(capture::Capturer::new({
        let n = frame_arrived.clone();
        Arc::new(move || n.notify_one())
    })));
    // 耗时日志：本轮从画面变化起算，直到译文全部上屏
    let mut round_start: Option<Instant> = None;

    loop {
        let geometry = *ctl.geometry.lock().unwrap();
        let settling = geometry.settle_remaining();
        if !settling.is_zero() && !ctl.paused.load(Ordering::Relaxed) {
            // 等待期间不响应 WGC/滚轮等截屏唤醒，结束时重新读取最新的变化时刻。
            tokio::time::sleep(settling).await;
        } else {
            let tick = if last_change.elapsed() < IDLE_AFTER { FAST_TICK } else { IDLE_TICK };
            let next_capture = tokio::time::sleep(tick);
            tokio::pin!(next_capture);
            // 无关滚轮不重设截屏计时，否则连续滚轮会饿死同位置的画面刷新。
            loop {
                tokio::select! {
                    biased;
                    _ = &mut next_capture => break,
                    _ = ctl.ready.notified() => break,
                    _ = ctl.wake.notified() => {
                        last_change = Instant::now();
                        break;
                    }
                    _ = frame_arrived.notified() => break,
                    w = wheel_rx.recv() => {
                        let Ok(w) = w else { continue };
                        let predict = ctl.click_through.load(Ordering::Relaxed)
                            && !ctl.paused.load(Ordering::Relaxed)
                            && !ctl.intercepting_mouse.load(Ordering::Relaxed)
                            && observed_revision == ctl.geometry.lock().unwrap().revision;
                        if !predict { continue; }
                        let mut delta = 0;
                        let mut hit = false;
                        for w in std::iter::once(w).chain(std::iter::from_fn(|| wheel_rx.try_recv().ok())) {
                            if last_rect.is_some_and(|r| zone_at(r, last_scale, w.x, w.y) == Zone::Inside) {
                                hit = true;
                                if !w.horizontal {
                                    delta += w.delta;
                                }
                            }
                        }
                        if hit {
                            last_change = Instant::now();
                            if delta != 0 {
                                let _ = app.emit_to(&label, "wheel", WheelPayload { epoch, delta, sent: unix_ms() });
                            }
                            break;
                        }
                    }
                }
            }
        }
        let Some(win) = app.get_webview_window(&label) else { break };
        let geometry = *ctl.geometry.lock().unwrap();
        if ctl.paused.load(Ordering::Relaxed) || !geometry.settle_remaining().is_zero() {
            continue;
        }
        let revision = geometry.revision;
        if last_rect.is_none() || observed_revision != revision {
            let (Ok(pos), Ok(size), Ok(scale)) = (win.outer_position(), win.outer_size(), win.scale_factor()) else {
                continue;
            };
            if revision != ctl.geometry.lock().unwrap().revision {
                continue;
            }
            // 作废旧版面和旧位置的滚轮输入，正在跑的 OCR 按计数丢弃。
            wheel_rx = input::subscribe();
            observed_revision = revision;
            last_rect = Some((pos.x, pos.y, size.width, size.height));
            last_scale = scale;
            epoch += 1;
            total = [0, 0];
            base = [0, 0];
            blocks.clear();
            anchors.clear();
            prev = None;
            ocr_stale = true;
            ocr_error = None;
            round_start = None;
            moving = false;
            last_change = Instant::now();
        }
        let rect = last_rect.expect("已读取翻译框几何位置");

        // 截屏 + 灰度化（阻塞操作放到线程池）
        let cap = capturer.clone();
        let shot = tauri::async_runtime::spawn_blocking(move || {
            let t = Instant::now();
            cap.lock().unwrap().grab(rect.0, rect.1, rect.2, rect.3).map(|s| {
                let cap_time = t.elapsed();
                let t = Instant::now();
                let frame = Frame::from_rgba(&s.image);
                (s.image, frame, s.ox, s.oy, s.at, cap_time, t.elapsed())
            })
        })
        .await;
        if revision != ctl.geometry.lock().unwrap().revision || ctl.paused.load(Ordering::Relaxed) {
            continue;
        }
        let (image, frame, ox, oy, shot_at, cap_time, gray_time) = match shot {
            Ok(Ok((image, frame, ox, oy, at, c, g))) => (image, Arc::new(frame), ox, oy, at, c, g),
            Ok(Err(e)) => {
                ocr_error = Some(e);
                continue;
            }
            Err(_) => continue,
        };

        let mut layout_changed = prev.is_none();
        let cmp_start = Instant::now();
        if prev
            .as_ref()
            .is_none_or(|p| motion::changed_fraction(p, &frame) > CHANGE_THRESHOLD)
        {
            // 内容滚动：立即推送新的累计位移，前端马上平移现有译文
            moving = false;
            if let Some((dx, dy)) = prev.as_ref().and_then(|p| motion::estimate_shift(p, &frame)) {
                let before = total;
                total[0] += dx;
                total[1] += dy;
                // 固定不动的块钉住、不随整体位移漂走；换了图层要重新推送版面
                if let Some(p) = &prev {
                    if reclassify(&blocks, &mut anchors, (&**p, &*frame), (dx, dy), (before, total, base), (ox, oy)) {
                        ctl.dirty.store(true, Ordering::Relaxed);
                    }
                }
                let age = shot_at.elapsed().as_millis() as u32;
                let _ = app.emit_to(&label, "shift", ShiftPayload { epoch, total, age, sent: unix_ms() });
                moving = dx != 0 || dy != 0;
                last_move = shot_at;
            }
            ocr_stale = true;
            last_change = Instant::now();
            prev = Some(frame.clone());
        } else if moving && shot_at.saturating_duration_since(last_move) >= STILL_AFTER {
            // 画面停了：补发一次不变的位移（截屏比界面刷新快时相邻两帧可能相同，所以要静止一小段才算）
            moving = false;
            let age = shot_at.elapsed().as_millis() as u32;
            let _ = app.emit_to(&label, "shift", ShiftPayload { epoch, total, age, sent: unix_ms() });
        }
        let cmp_time = cmp_start.elapsed();
        if ocr_stale && round_start.is_none() {
            round_start = Some(shot_at);
        }

        let mut s = shared.lock().unwrap();

        // 取走已完成的 OCR 结果（几何变化前发起的作废）
        if let Some(done) = s.ocr_done.take() {
            if done.epoch == epoch {
                match done.result {
                    Ok(b) => {
                        // 上一版里钉住的块：同样的文字出现在同一位置的继续钉住，下次滚动时不必先漂一下再纠正
                        let mut next: Vec<Anchor> = b
                            .iter()
                            .map(|nb| {
                                let kept = blocks.iter().zip(&anchors).any(|(ob, oa)| {
                                    oa.pinned && ob.source == nb.source && (oa.x - nb.x).abs() < 4.0 && (oa.y - nb.y).abs() < 4.0
                                });
                                Anchor { pinned: kept, ..Anchor::of(nb) }
                            })
                            .collect();
                        // 识别期间画面又滚动过：没跟着动的块直接钉在识别时的位置
                        let d = (total[0] - done.base[0], total[1] - done.base[1]);
                        if let Some(p) = prev.as_ref().filter(|_| d != (0, 0)) {
                            reclassify(&b, &mut next, (&*done.frame, &**p), d, (done.base, total, done.base), done.off);
                        }
                        blocks = b;
                        anchors = next;
                        base = done.base;
                        ocr_error = None;
                    }
                    Err(e) => ocr_error = Some(e),
                }
                layout_changed = true;
            }
        }

        // 画面有变化且没有 OCR 在跑：识别最新这一帧（OCR 期间的滚动由 total - base 补偿）
        if ocr_stale && !s.ocr_busy {
            ocr_stale = false;
            s.ocr_busy = true;
            let (shared, ctl, ocr_base, ocr_epoch, ocr_frame) = (shared.clone(), ctl.clone(), total, epoch, frame.clone());
            tauri::async_runtime::spawn_blocking(move || {
                let cfg = config::load().unwrap_or_default();
                let skip_cjk = cfg.target_lang.to_ascii_lowercase().starts_with("zh");
                let t = Instant::now();
                let result = ocr::recognize(&image, &cfg.ocr_lang, &cfg.target_lang).map(|out| {
                    let ocr_time = t.elapsed();
                    let t = Instant::now();
                    let mut blocks = layout::build_blocks(&out, &image, skip_cjk);
                    tlog!(
                        "悬浮窗 {}×{}：截屏 {} · 灰度化 {} · 画面比对 {} · OCR {}（{} 行）· 分段 {}（{} 段）",
                        image.width(),
                        image.height(),
                        ms(cap_time),
                        ms(gray_time),
                        ms(cmp_time),
                        ms(ocr_time),
                        out.lines.len(),
                        ms(t.elapsed()),
                        blocks.len()
                    );
                    // 截图可能因超出屏幕被裁过，换算回相对翻译框的坐标
                    for b in &mut blocks {
                        b.x += ox as f32;
                        b.y += oy as f32;
                    }
                    blocks
                });
                let mut s = shared.lock().unwrap();
                s.ocr_busy = false;
                s.ocr_done = Some(OcrDone {
                    epoch: ocr_epoch,
                    base: ocr_base,
                    frame: ocr_frame,
                    off: (ox, oy),
                    result,
                });
                ctl.dirty.store(true, Ordering::Relaxed);
                ctl.ready.notify_one();
            });
        }

        // 新出现的文字：批量请求翻译（同一时间只有一个请求在途）
        let mut seen = HashSet::new();
        let missing: Vec<String> = blocks
            .iter()
            .map(|b| b.source.clone())
            .filter(|t| !s.cache.contains_key(t) && seen.insert(t.clone()))
            .collect();
        let can_retry = s.failed_at.is_none_or(|t| t.elapsed() > RETRY_AFTER);
        if !missing.is_empty() && !s.in_flight && can_retry {
            s.in_flight = true;
            let (shared, ctl) = (shared.clone(), ctl.clone());
            tauri::async_runtime::spawn(async move {
                let cfg = config::load().unwrap_or_default();
                let result = translate::translate_batch_with_fallback(&cfg, &missing, "悬浮窗").await;
                let mut s = shared.lock().unwrap();
                s.in_flight = false;
                match result {
                    Ok((texts, engine)) => {
                        if s.cache.len() > CACHE_LIMIT {
                            s.cache.clear();
                        }
                        s.cache.extend(missing.into_iter().zip(texts));
                        s.error = None;
                        s.failed_at = None;
                        s.engine = Some(engine);
                    }
                    Err(e) => {
                        s.error = Some(t!("err.engine.failed", e = e));
                        s.failed_at = Some(Instant::now());
                    }
                }
                ctl.dirty.store(true, Ordering::Relaxed);
                ctl.ready.notify_one();
            });
        }

        let dirty = ctl.dirty.swap(false, Ordering::Relaxed);
        if !layout_changed && !dirty {
            continue;
        }
        let payload = LayoutPayload {
            epoch,
            base,
            total,
            blocks: blocks
                .iter()
                .zip(&anchors)
                .filter_map(|(b, a)| {
                    s.cache.get(&b.source).map(|t| RenderBlock {
                        block: Block { x: a.x, y: a.y, ..b.clone() },
                        text: t.clone(),
                        pinned: a.pinned,
                    })
                })
                .collect(),
            pending: blocks.iter().filter(|b| !s.cache.contains_key(&b.source)).count(),
            error: ocr_error.clone().or_else(|| s.error.clone()),
            engine: s.engine.clone(),
        };
        let settled = !ocr_stale && !s.ocr_busy && s.ocr_done.is_none() && !s.in_flight;
        drop(s);
        if let Some(t) = round_start {
            if payload.error.is_some() {
                tlog!("悬浮窗 本轮出错，从画面变化起已过 {}", ms(t.elapsed()));
                round_start = None;
            } else if settled && payload.pending == 0 {
                tlog!("悬浮窗 本轮 {} 段：画面变化 → 译文全部上屏 共 {}", payload.blocks.len(), ms(t.elapsed()));
                round_start = None;
            }
        }
        if revision == ctl.geometry.lock().unwrap().revision && !ctl.paused.load(Ordering::Relaxed) {
            let _ = app.emit_to(&label, "layout", payload);
        }
    }

    app.state::<LiveState>().frames.lock().unwrap().remove(&label);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 窄框仍以边缘缩放优先，工具条之外是正文。
    #[test]
    fn 窄框命中区随缩放换算() {
        let rect = (-400, 200, 120, 66);
        let scale = 1.5;
        assert!(zone_at(rect, scale, -401, 220) == Zone::Outside);
        assert!(zone_at(rect, scale, -392, 209) == Zone::Edge);
        assert!(zone_at(rect, scale, -391, 208) == Zone::Edge);
        assert!(zone_at(rect, scale, -391, 209) == Zone::Toolbar);
        assert!(zone_at(rect, scale, -290, 256) == Zone::Toolbar);
        assert!(zone_at(rect, scale, -289, 256) == Zone::Edge);
        assert!(zone_at(rect, scale, -280, 256) == Zone::Outside);

        let tall = (-400, 200, 450, 300);
        assert!(zone_at(tall, scale, -188, 230) == Zone::Inside);
        assert!(zone_at(tall, scale, -187, 230) == Zone::Toolbar);
        assert!(zone_at(tall, scale, -100, 257) == Zone::Inside);
    }

    /// 旧配置缺少鼠标模式时，保持默认穿透。
    #[test]
    fn 旧配置默认鼠标穿透() {
        let legacy: config::Config = serde_json::from_str(r#"{"mode":"ocr"}"#).unwrap();
        let drag: config::Config = serde_json::from_str(r#"{"frame_click_through":false}"#).unwrap();
        assert!(legacy.frame_click_through);
        assert!(!drag.frame_click_through);
    }

    /// 真机计时（只读屏幕，不操作窗口）：连续"截屏 + 灰度化 + 变化检测 + 位移估计"的帧率，
    /// 以及画面的平均延迟（截取完成时距画面对应时刻）。TA_CAPTURE=wgc / printwindow 可强制截屏方式。
    /// TF_RECT=x,y,w,h cargo test 跟踪帧率 -- --ignored --nocapture
    #[test]
    #[ignore]
    fn 跟踪帧率() {
        let v: Vec<i32> = std::env::var("TF_RECT")
            .unwrap_or_else(|_| "300,150,900,750".into())
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        capture::tests::dpi_aware();
        let mut capturer = capture::Capturer::new(Arc::new(|| {}));
        let mut prev: Option<Frame> = None;
        let n = 60;
        let t = Instant::now();
        let (mut cap, mut age) = (Duration::ZERO, Duration::ZERO);
        let mut wgc = 0;
        for _ in 0..n {
            let t1 = Instant::now();
            let s = capturer.grab(v[0], v[1], v[2] as u32, v[3] as u32).unwrap();
            cap += t1.elapsed();
            age += s.at.elapsed();
            wgc += s.wgc as u32;
            let f = Frame::from_rgba(&s.image);
            if let Some(p) = &prev {
                let _ = motion::changed_fraction(p, &f);
                let _ = motion::estimate_shift(p, &f);
            }
            prev = Some(f);
        }
        let el = t.elapsed();
        println!(
            "每帧 {:?}（其中截屏 {:?}，画面平均已过去 {:?}），约 {:.0} 帧/秒；WGC {wgc}/{n} 帧",
            el / n,
            cap / n,
            age / n,
            n as f64 / el.as_secs_f64()
        );
    }

    /// 真机端到端：截取主屏左上角 → OCR → 分段 → 批量翻译。cargo test 端到端 -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn 端到端实测屏幕() {
        let (image, _, _) = capture::capture_under(0, 0, 1000, 700).unwrap();
        let t = Instant::now();
        let out = ocr::recognize(&image, "auto", "zh-CN").unwrap();
        let blocks = layout::build_blocks(&out, &image, true);
        println!("OCR + 分段耗时 {:?}，{} 行 → {} 段", t.elapsed(), out.lines.len(), blocks.len());
        let texts: Vec<String> = blocks.iter().map(|b| b.source.clone()).collect();
        let t = Instant::now();
        let (tr, engine) = translate::translate_batch_with_fallback(&config::Config::default(), &texts, "测试")
            .await
            .unwrap();
        println!("批量翻译耗时 {:?}（{engine}）", t.elapsed());
        for (b, t) in blocks.iter().zip(&tr).take(12) {
            println!("[{:.0},{:.0} {:.0}x{:.0} {} {}] {} → {}", b.x, b.y, b.w, b.h, b.bg, b.fg, b.source, t);
        }
    }
}

/// 鼠标悬停区域
#[derive(Clone, Copy, PartialEq)]
enum Zone {
    Outside,
    Inside,
    Edge,
    Toolbar,
}

impl Zone {
    fn name(self) -> &'static str {
        match self {
            Zone::Outside => "outside",
            Zone::Inside => "inside",
            Zone::Edge => "edge",
            Zone::Toolbar => "toolbar",
        }
    }
}

/// 边缘优先。工具条的尺寸在窄框内与前端一起收缩。
fn zone_at((x, y, w, h): (i32, i32, u32, u32), scale: f64, cx: i32, cy: i32) -> Zone {
    let cx = f64::from(cx) - f64::from(x);
    let cy = f64::from(cy) - f64::from(y);
    let (w, h) = (f64::from(w), f64::from(h));
    let edge = EDGE_CSS * scale;
    if cx < 0.0 || cy < 0.0 || cx >= w || cy >= h {
        Zone::Outside
    } else if cx < edge || cy < edge || cx >= w - edge || cy >= h - edge {
        Zone::Edge
    } else if cx >= w - edge - TOOLBAR_W_CSS * scale && cy < edge + TOOLBAR_H_CSS * scale {
        Zone::Toolbar
    } else {
        Zone::Inside
    }
}

/// 保持当前鼠标手势的窗口命中状态，避免按住时把事件交给下方应用或从它手里抢走。
async fn hover_loop(app: AppHandle, label: String, ctl: Arc<FrameCtl>, mut interactive: bool) {
    let mut zone = Zone::Outside;
    let mut nav_down = false;
    let mut geometry_cache = None;
    loop {
        tokio::time::sleep(HOVER_TICK).await;
        let Some(win) = app.get_webview_window(&label) else { break };
        // 翻页键（只查这几个键是否按下，不监听其他按键）：按下瞬间唤醒跟踪循环
        let nav = capture::nav_key_down();
        if nav && !nav_down {
            ctl.wake.notify_one();
        }
        nav_down = nav;
        let Some((cx, cy, button_down)) = capture::cursor_state() else { continue };
        if button_down {
            continue;
        }
        let revision = ctl.geometry.lock().unwrap().revision;
        if geometry_cache.is_none_or(|(cached, _, _)| cached != revision) {
            let (Ok(pos), Ok(size), Ok(scale)) = (win.outer_position(), win.outer_size(), win.scale_factor()) else {
                continue;
            };
            if ctl.geometry.lock().unwrap().revision != revision {
                continue;
            }
            geometry_cache = Some((revision, (pos.x, pos.y, size.width, size.height), scale));
        }
        let (_, rect, scale) = geometry_cache.expect("已读取翻译框几何位置");
        let new_zone = zone_at(rect, scale, cx, cy);
        let want = !ctl.click_through.load(Ordering::Relaxed) || matches!(new_zone, Zone::Edge | Zone::Toolbar);
        if want != interactive && win.set_ignore_cursor_events(!want).is_ok() {
            interactive = want;
            ctl.intercepting_mouse.store(interactive, Ordering::Relaxed);
        }
        if new_zone != zone {
            zone = new_zone;
            let _ = app.emit_to(&label, "hover", zone.name());
        }
    }
}
