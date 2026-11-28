//! 沉浸模式：直接改写目标应用界面里的文字，而不是在上面盖一层浮窗。
//!
//! 第一个通道面向 Electron / Chromium 内核应用（Claude 客户端、VS Code 等）：
//! 应用带 `--remote-debugging-port=<端口>` 启动后，本模块通过 Chrome DevTools Protocol
//! 连上它的每个页面，往隔离环境里注入 `inject.js`，由脚本在 DOM 里原地替换文字。
//! 软件本体（exe、asar）一个字节都不改，停用或退出随译即恢复原文。
//!
//! 管理模型：配置里每个应用对应一个"运行器"（[`Runner`]），各自有开关、原文/译文状态和运行状态。
//! 改配置后调用 [`reload`]：只重启配置有变化的应用，术语表的变化直接推送到已打开的页面。
//!
//! 身份校验：调试端口背后的进程必须就是配置里的程序（`exe`），否则拒绝接管。

mod cdp;
mod launch;
pub mod procs;
mod session;

use crate::config::{self, ImmersiveTarget};
use crate::translate;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{broadcast, watch};

pub use launch::{DetectedApp, detect_apps, display_name, launch};

/// 应用的运行状态
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// 沉浸模式总开关已关闭
    Off,
    /// 这个应用被单独停用
    Disabled,
    /// 应用没在运行
    NotRunning,
    /// 应用在运行，但不是以沉浸模式启动的（没开调试端口）
    NoPort,
    /// 调试端口被别的程序占用，拒绝接管
    Conflict,
    /// 正在等用户退出应用，以便用沉浸模式重新打开
    Waiting,
    /// 已连上
    Attached,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageInfo {
    pub id: String,
    pub url: String,
    pub title: String,
}

/// 推给设置界面和托盘的单个应用状态
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TargetStatus {
    pub port: u16,
    pub name: String,
    pub enabled: bool,
    pub state: State,
    /// 给人看的状态说明
    pub message: String,
    /// 已接管的页面
    pub pages: Vec<PageInfo>,
    /// true = 显示译文，false = 显示原文
    pub show: bool,
    /// 本次运行中回填到页面的条数
    pub translated: u64,
    /// 本次运行中请求翻译引擎的批次（其余都命中术语表 / 译文库）
    pub engine_batches: u64,
    /// "以沉浸模式启动"流程中的提示；有值时覆盖探测结果显示为等待状态
    #[serde(skip)]
    pub waiting: Option<String>,
}

impl TargetStatus {
    fn new(t: &ImmersiveTarget, state: State, message: &str) -> Self {
        Self {
            port: t.port,
            name: t.name.clone(),
            enabled: t.enabled,
            state,
            message: message.into(),
            pages: Vec::new(),
            show: true,
            translated: 0,
            engine_batches: 0,
            waiting: None,
        }
    }
}

pub type SharedStatus = Arc<Mutex<TargetStatus>>;

struct Runner {
    cfg: ImmersiveTarget,
    /// 决定要不要重启的配置指纹
    key: String,
    show: watch::Sender<bool>,
    stop: watch::Sender<bool>,
    status: SharedStatus,
}

/// 沉浸模式运行状态（Tauri 托管）
pub struct ImmersiveState {
    runners: Mutex<Vec<Runner>>,
    /// 广播给所有已接管页面的 JS 表达式（术语表 / 译文库修改后推送）
    patch: broadcast::Sender<String>,
    /// 上次推送给页面的术语表，用来算差异
    glossary: Mutex<HashMap<String, String>>,
    /// 上次发出的状态（去重）
    last: Mutex<String>,
}

impl Default for ImmersiveState {
    fn default() -> Self {
        Self {
            runners: Mutex::default(),
            patch: broadcast::channel(64).0,
            glossary: Mutex::default(),
            last: Mutex::default(),
        }
    }
}

/// 启动或按最新配置重新加载。只重启配置有变化的应用；术语表变化直接推送到页面
pub fn reload(app: &AppHandle) {
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}\n沉浸模式未加载新配置");
            return;
        }
    };
    let state = app.state::<ImmersiveState>();
    {
        let mut runners = state.runners.lock().unwrap();
        let mut old: HashMap<u16, Runner> = runners.drain(..).map(|r| (r.cfg.port, r)).collect();
        for t in &cfg.immersive.targets {
            let active = cfg.immersive.enabled && t.enabled;
            let key = json!({ "t": t, "lang": cfg.target_lang, "active": active }).to_string();
            let mut show = true;
            if let Some(r) = old.remove(&t.port) {
                if r.key == key {
                    runners.push(r);
                    continue;
                }
                show = *r.show.borrow();
                let _ = r.stop.send(true);
            }
            let (st, msg) = match (cfg.immersive.enabled, t.enabled) {
                (false, _) => (State::Off, t!("status.off")),
                (true, false) => (State::Disabled, t!("status.disabled")),
                _ => (State::NotRunning, t!("status.checking")),
            };
            let runner = Runner {
                cfg: t.clone(),
                key,
                show: watch::channel(show).0,
                stop: watch::channel(false).0,
                status: Arc::new(Mutex::new(TargetStatus::new(t, st, &msg))),
            };
            if active {
                tauri::async_runtime::spawn(supervise(
                    app.clone(),
                    t.clone(),
                    runner.show.subscribe(),
                    runner.stop.subscribe(),
                    runner.status.clone(),
                ));
            }
            runners.push(runner);
        }
        // 配置里删掉的应用：停掉（页面恢复原文）
        for r in old.into_values() {
            let _ = r.stop.send(true);
        }
    }
    push_glossary(app, &cfg);
    // 配置可能是托盘、设置窗口或启动流程改的，通知设置窗口重新读取
    let _ = app.emit("config-changed", ());
    notify(app);
}

/// 术语表有变化时，把差异推送到所有已接管的页面
fn push_glossary(app: &AppHandle, cfg: &config::Config) {
    let state = app.state::<ImmersiveState>();
    let new = translate::glossary(cfg);
    let mut last = state.glossary.lock().unwrap();
    let set: HashMap<&String, &String> = new.iter().filter(|(k, v)| last.get(*k) != Some(*v)).collect();
    let del: Vec<&String> = last.keys().filter(|k| !new.contains_key(*k)).collect();
    if !set.is_empty() || !del.is_empty() {
        broadcast_patch(app, json!({ "set": set, "del": del }));
    }
    *last = new;
}

/// 推送给页面：`set` 原文 → 新译文（立即替换已显示的），`del` 原文列表（忘掉旧译文、重新翻译）
pub fn broadcast_patch(app: &AppHandle, patch: serde_json::Value) {
    let expr = format!("globalThis.__tf && __tf.patch({patch})");
    let _ = app.state::<ImmersiveState>().patch.send(expr);
}

/// 当前所有应用的状态
pub fn statuses(app: &AppHandle) -> Vec<TargetStatus> {
    let state = app.state::<ImmersiveState>();
    let runners = state.runners.lock().unwrap();
    runners
        .iter()
        .map(|r| {
            let mut s = r.status.lock().unwrap().clone();
            s.show = *r.show.borrow();
            s
        })
        .collect()
}

/// 状态有变化时通知设置界面和托盘
pub fn notify(app: &AppHandle) {
    let list = statuses(app);
    let text = serde_json::to_string(&list).unwrap_or_default();
    {
        let state = app.state::<ImmersiveState>();
        let mut last = state.last.lock().unwrap();
        if *last == text {
            return;
        }
        *last = text;
    }
    let _ = app.emit("immersive-status", &list);
    crate::tray::refresh(app, &list);
}

/// 设置原文 / 译文显示。`port` 为空表示所有应用
pub fn set_show(app: &AppHandle, port: Option<u16>, show: bool) {
    {
        let state = app.state::<ImmersiveState>();
        for r in state.runners.lock().unwrap().iter() {
            if port.is_none_or(|p| p == r.cfg.port) {
                let _ = r.show.send(show);
            }
        }
    }
    notify(app);
}

/// 全局快捷键：有任一应用在显示译文就全部切回原文，否则全部显示译文。返回切换后是否显示译文
pub fn toggle(app: &AppHandle) -> bool {
    let any_shown = statuses(app).iter().any(|s| s.state == State::Attached && s.show);
    set_show(app, None, !any_shown);
    println!("沉浸模式：{}", if any_shown { "显示原文" } else { "显示译文" });
    !any_shown
}

/// 退出前：停掉所有应用（页面恢复原文），最多等 1.5 秒
pub async fn shutdown(app: &AppHandle) {
    let statuses: Vec<SharedStatus> = {
        let state = app.state::<ImmersiveState>();
        let runners = state.runners.lock().unwrap();
        for r in runners.iter() {
            let _ = r.stop.send(true);
        }
        runners.iter().map(|r| r.status.clone()).collect()
    };
    for _ in 0..15 {
        if statuses.iter().all(|s| s.lock().unwrap().pages.is_empty()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) fn status_of(app: &AppHandle, port: u16) -> Option<SharedStatus> {
    let state = app.state::<ImmersiveState>();
    let runners = state.runners.lock().unwrap();
    runners.iter().find(|r| r.cfg.port == port).map(|r| r.status.clone())
}

// ───────────────────────── 守护任务 ─────────────────────────

/// 只尝试自动切换刚打开的应用；更早打开的留给用户手动重启。
const AUTO_FRESH: Duration = Duration::from_secs(30);
/// 两次自动重启的最小间隔
const AUTO_COOLDOWN: Duration = Duration::from_secs(120);

enum Probe {
    Ready,
    Other(State, String),
}

/// 探测应用当前情况（含身份校验）。会枚举进程，放在阻塞线程里跑
fn probe(t: &ImmersiveTarget) -> Probe {
    let exe = PathBuf::from(t.exe.trim());
    match procs::port_owner(t.port) {
        Some(pid) => {
            if t.exe.trim().is_empty() {
                return Probe::Ready; // 没配程序路径：无法校验，按端口接管
            }
            // WebView2 应用监听端口的是它启动的 msedgewebview2.exe，要沿父进程链找到配置的程序
            if procs::owned_by(pid, &exe) {
                return Probe::Ready;
            }
            match procs::exe_path(pid) {
                Some(p) => Probe::Other(
                    State::Conflict,
                    t!("status.port_taken", port = t.port, path = p.display()),
                ),
                None => Probe::Other(
                    State::Conflict,
                    t!("status.port_unknown", port = t.port, pid = pid),
                ),
            }
        }
        None if !t.exe.trim().is_empty() && !procs::pids_of(&exe).is_empty() => Probe::Other(
            State::NoPort,
            t!("status.no_port"),
        ),
        None => Probe::Other(State::NotRunning, t!("status.not_running")),
    }
}

async fn supervise(
    app: AppHandle,
    t: ImmersiveTarget,
    show: watch::Receiver<bool>,
    mut stop: watch::Receiver<bool>,
    status: SharedStatus,
) {
    let mut announced = false;
    // 上次自动重启的时间：防止应用不认调试端口参数时反复重启
    let mut last_auto: Option<Instant> = None;
    loop {
        if *stop.borrow() {
            break;
        }
        let tc = t.clone();
        let probed = tauri::async_runtime::spawn_blocking(move || probe(&tc))
            .await
            .unwrap_or_else(|e| Probe::Other(State::NotRunning, e.to_string()));
        match probed {
            Probe::Ready => match session::list_targets(t.port).await {
                Ok(list) => {
                    if !announced {
                        announced = true;
                        println!("沉浸模式：已连上「{}」", t.name);
                    }
                    let mut new_pages = Vec::new();
                    {
                        let mut s = status.lock().unwrap();
                        s.waiting = None;
                        for p in list.iter().filter(|p| session::wanted(p, &t)) {
                            match s.pages.iter_mut().find(|x| x.id == p.id) {
                                Some(x) => {
                                    x.url = p.url.clone();
                                    x.title = p.title.clone();
                                }
                                None => {
                                    s.pages.push(PageInfo { id: p.id.clone(), url: p.url.clone(), title: p.title.clone() });
                                    new_pages.push(p.clone());
                                }
                            }
                        }
                        s.state = State::Attached;
                        s.message = if s.pages.is_empty() {
                            t!("status.attached_empty")
                        } else {
                            t!("status.attached", n = s.pages.len())
                        };
                    }
                    for p in new_pages {
                        let ctx = session::Ctx {
                            app: app.clone(),
                            target: t.clone(),
                            show: show.clone(),
                            stop: stop.clone(),
                            patch: app.state::<ImmersiveState>().patch.subscribe(),
                            status: status.clone(),
                        };
                        tauri::async_runtime::spawn(session::run(p, ctx));
                    }
                }
                Err(e) => {
                    let mut s = status.lock().unwrap();
                    s.state = State::Conflict;
                    s.message = t!("status.list_failed", port = t.port, e = e);
                }
            },
            Probe::Other(state, message) => {
                if announced {
                    announced = false;
                    println!("沉浸模式：「{}」已断开", t.name);
                }
                let idle = {
                    let mut s = status.lock().unwrap();
                    match s.waiting.clone() {
                        Some(w) => {
                            s.state = State::Waiting;
                            s.message = w;
                            false
                        }
                        None => {
                            s.state = state;
                            s.message = message;
                            true
                        }
                    }
                };
                // 用户刚直接打开了应用（没带调试端口）：尝试正常退出并重启成沉浸模式。
                if state == State::NoPort
                    && idle
                    && t.auto_relaunch
                    && last_auto.is_none_or(|i| i.elapsed() > AUTO_COOLDOWN)
                {
                    let exe = PathBuf::from(t.exe.trim());
                    let fresh = tauri::async_runtime::spawn_blocking(move || procs::main_process(&exe).and_then(procs::age))
                        .await
                        .ok()
                        .flatten()
                        .is_some_and(|a| a < AUTO_FRESH);
                    if fresh {
                        last_auto = Some(Instant::now());
                        println!("沉浸模式：「{}」未带调试端口启动，自动重启", t.name);
                        let (app, port, name) = (app.clone(), t.port, t.name.clone());
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = launch(&app, port, false).await {
                                eprintln!("沉浸模式：自动重启「{name}」失败：{e}");
                            }
                        });
                    }
                }
            }
        }
        notify(&app);
        tokio::select! {
            _ = stop.changed() => break,
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
}
