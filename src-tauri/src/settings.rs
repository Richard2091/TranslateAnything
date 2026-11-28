//! 设置窗口与它用到的命令：配置读写、沉浸模式管理、译文库与术语表管理

use crate::config::{self, Config, PortableWebview};
use crate::immersive::{self, DetectedApp, TargetStatus};
use crate::{store, translate};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const SETTINGS_LABEL: &str = "settings";

/// 从设置窗口读取 Windows 登录自启动状态。
#[tauri::command]
pub fn autostart_status(window: WebviewWindow) -> Result<crate::autostart::Status, String> {
    if window.label() != SETTINGS_LABEL {
        return Err(t!("general.autostartReadFailed"));
    }
    crate::autostart::status().map_err(|error| {
        eprintln!("读取开机自启动状态失败：{error}");
        t!("general.autostartReadFailed")
    })
}

/// 只允许设置窗口切换当前用户的登录自启动。
#[tauri::command]
pub fn autostart_set(window: WebviewWindow, enabled: bool) -> Result<crate::autostart::Status, String> {
    if window.label() != SETTINGS_LABEL {
        return Err(t!("general.autostartSaveFailed"));
    }
    crate::autostart::set(enabled).map_err(|error| {
        eprintln!("更改开机自启动失败：{error}");
        t!("general.autostartSaveFailed")
    })
}

/// 下次打开设置窗口时要跳到的页面（托盘"发现新版本"用）
static GOTO: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

/// 打开设置窗口并跳到指定页面：已打开的窗口收事件跳转，新窗口载入后取走 GOTO
pub fn open_settings_at(app: &AppHandle, tab: &'static str) -> Result<(), String> {
    *GOTO.lock().unwrap() = Some(tab);
    let _ = app.emit_to(SETTINGS_LABEL, "settings-goto", tab);
    open_settings(app)
}

#[tauri::command]
pub fn settings_take_goto() -> Option<&'static str> {
    GOTO.lock().unwrap().take()
}

/// 打开设置窗口（已打开则调到前台）
pub fn open_settings(app: &AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(app, SETTINGS_LABEL, WebviewUrl::App("settings.html".into()))
        .portable_data()
        .title(t!("window.settings", app = t!("app.name")))
        .inner_size(1000.0, 720.0)
        .min_inner_size(760.0, 520.0)
        // 不用系统标题栏：顶部空白处作拖动区，最小化 / 最大化 / 关闭按钮由页面自己画（见 settings.html 的 #titlebar）
        .decorations(false)
        .center()
        .build()
        .map(|w| {
            let app = app.clone();
            w.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    let _ = app.emit("frame-corner-radius-preview", Option::<u8>::None);
                }
            });
        })
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct SettingsView {
    config: Config,
    config_path: String,
    /// 配置文件大小（字节），文件还没创建时为 0
    config_size: u64,
    db_path: String,
    /// 译文库大小（字节，含预写日志）
    db_size: u64,
    /// 译文库是否放在了自定义位置
    db_custom: bool,
    /// 当前目标语言的内置术语表
    builtin_glossary: BTreeMap<String, String>,
}

#[tauri::command]
pub fn settings_get() -> Result<SettingsView, String> {
    let config = config::load()?;
    let config_path = config::config_path();
    Ok(SettingsView {
        builtin_glossary: translate::builtin_glossary(&config.target_lang),
        db_custom: !config.store_dir.trim().is_empty(),
        config,
        config_size: config_path.as_ref().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len()),
        config_path: config_path.map(|p| p.display().to_string()).unwrap_or_default(),
        db_path: store::db_path().map(|p| p.display().to_string()).unwrap_or_default(),
        db_size: store::db_size(),
    })
}

/// 保存整份配置并立即生效（快捷键除外，需重启）
#[tauri::command]
pub fn settings_save(app: AppHandle, mut config: Config) -> Result<(), String> {
    // 译文库位置只能通过 store_relocate 修改（要连同文件一起搬），这里沿用磁盘上的值
    let previous = config::load().unwrap_or_default();
    config.store_dir = previous.store_dir;
    config::save(&config)?;
    translate::clear_all_cooldowns();
    if config.frame_corner_radius != previous.frame_corner_radius {
        let _ = app.emit("frame-corner-radius", config.frame_corner_radius);
    }
    apply_ui_lang(&app, &config.ui_lang);
    immersive::reload(&app);
    if config.update.skip_version != previous.update.skip_version {
        crate::tray::refresh_now(&app);
    }
    Ok(())
}

/// 界面语言变了：重建托盘菜单，并通知各窗口重新载入文案
pub fn apply_ui_lang(app: &AppHandle, pref: &str) {
    if crate::i18n::set_pref(pref) {
        crate::tray::refresh(app, &immersive::statuses(app));
        let _ = app.emit("locale-changed", crate::i18n::locale());
    }
}

/// 单独开关某个应用
pub fn set_target_enabled(app: &AppHandle, port: u16, enabled: bool) -> Result<(), String> {
    let mut cfg = config::load()?;
    let t = cfg.immersive.targets.iter_mut().find(|t| t.port == port).ok_or_else(|| t!("err.immersive.app_not_found"))?;
    t.enabled = enabled;
    config::save(&cfg)?;
    immersive::reload(app);
    Ok(())
}

#[tauri::command]
pub fn immersive_set_enabled(app: AppHandle, port: u16, enabled: bool) -> Result<(), String> {
    set_target_enabled(&app, port, enabled)
}

#[tauri::command]
pub fn immersive_status(app: AppHandle) -> Vec<TargetStatus> {
    immersive::statuses(&app)
}

/// 原文 / 译文切换；`port` 为空表示所有应用
#[tauri::command]
pub fn immersive_set_show(app: AppHandle, port: Option<u16>, show: bool) {
    immersive::set_show(&app, port, show);
}

#[tauri::command]
pub async fn immersive_launch(app: AppHandle, port: u16) -> Result<String, String> {
    immersive::launch(&app, port, true).await
}

#[tauri::command]
pub async fn immersive_detect() -> Vec<DetectedApp> {
    immersive::detect_apps().await
}

/// 程序的显示名（版本信息里的名称，没有则用文件名）
#[tauri::command]
pub async fn app_name(exe: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || immersive::display_name(std::path::Path::new(exe.trim())))
        .await
        .map_err(|e| e.to_string())
}

/// 程序图标：宽、高各 4 字节（小端）+ RGBA 像素；取不到时为空
#[tauri::command]
pub async fn app_icon(exe: String) -> tauri::ipc::Response {
    let px = tauri::async_runtime::spawn_blocking(move || crate::appinfo::icon_rgba(std::path::Path::new(exe.trim()), 64))
        .await
        .ok()
        .flatten();
    tauri::ipc::Response::new(px.map_or_else(Vec::new, |(w, h, px)| [&w.to_le_bytes()[..], &h.to_le_bytes(), &px].concat()))
}

#[tauri::command]
pub fn store_query(keyword: String, manual_only: bool, offset: i64, limit: i64) -> Result<store::Page, String> {
    let lang = config::load()?.target_lang;
    store::query(&lang, &keyword, manual_only, offset, limit.clamp(1, 500))
}

/// 改译：写成手工译文，并推送到已接管的页面立即生效
#[tauri::command]
pub fn store_set(app: AppHandle, source: String, target: String) -> Result<(), String> {
    if source.trim().is_empty() || target.trim().is_empty() {
        return Err(t!("err.store.empty_pair"));
    }
    let lang = config::load()?.target_lang;
    store::set_manual(&lang, &source, &target)?;
    let set: BTreeMap<String, String> = [(source, target)].into();
    immersive::broadcast_patch(&app, json!({ "set": set }));
    Ok(())
}

/// 删除若干条译文；已接管的页面会重新翻译这些原文
#[tauri::command]
pub fn store_delete(app: AppHandle, sources: Vec<String>) -> Result<usize, String> {
    let lang = config::load()?.target_lang;
    let n = store::delete(&lang, &sources)?;
    immersive::broadcast_patch(&app, json!({ "del": sources }));
    Ok(n)
}

/// 清空当前目标语言的译文（可保留手工译文）。已显示在页面上的译文不受影响，页面刷新后才会重新翻译
#[tauri::command]
pub fn store_clear(keep_manual: bool) -> Result<usize, String> {
    let lang = config::load()?.target_lang;
    store::clear(&lang, keep_manual)
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle) -> Result<(), String> {
    open_settings(&app)
}

/// 当前目标语言的译文库概况
#[tauri::command]
pub fn store_stats() -> Result<store::Stats, String> {
    let lang = config::load()?.target_lang;
    store::stats(&lang)
}

/// 用已保存的配置测试某个引擎
#[tauri::command]
pub async fn engine_test(id: String) -> Result<translate::EngineTest, String> {
    let cfg = config::load()?;
    translate::test_engine(&cfg, &id).await
}

/// 列出大模型接口提供的模型（用已保存的接口地址、密钥和代理，按协议分发）
#[tauri::command]
pub async fn llm_models(id: String) -> Result<Vec<String>, String> {
    let cfg = config::load()?;
    let provider = cfg.provider(&id);
    let kind = provider.kind_or(&id);
    translate::list_models(&kind, &provider).await
}

/// 按当前生效调用链逐节点测速，并通过事件向设置窗口推送每步状态
#[tauri::command]
pub async fn engine_test_chain(app: AppHandle, run_id: String) -> Result<Vec<translate::ChainTestStep>, String> {
    let cfg = config::load()?;
    let steps = translate::test_chain(&cfg, &run_id, |step| {
        let _ = app.emit_to(SETTINGS_LABEL, "chain-test-step", step);
    })
    .await;
    Ok(steps)
}

/// 当前处于失败冷却期的引擎 id
#[tauri::command]
pub fn engine_cooldowns() -> Vec<String> {
    translate::cooling_ids()
}

/// 选择要接管的程序：弹出文件选择框，返回所选 .exe 的路径；用户取消时返回 None
#[tauri::command]
pub async fn pick_exe(app: AppHandle, current: String) -> Result<Option<String>, String> {
    let owner = app
        .get_webview_window(SETTINGS_LABEL)
        .and_then(|w| w.hwnd().ok())
        .map_or(0, |h| h.0 as isize);
    let initial = std::path::Path::new(current.trim()).parent().map(std::path::Path::to_path_buf);
    let title = t!("dialog.pick_exe");
    tauri::async_runtime::spawn_blocking(move || crate::dialog::pick_exe(owner, &title, initial.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

/// 从设置窗口发起框选：先把设置窗口收起来，免得挡住要翻译的内容，也免得被截进画面
#[tauri::command]
pub async fn overlay_start(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = w.minimize();
    }
    // 等最小化动画结束再出遮罩
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    tauri::async_runtime::spawn_blocking(move || crate::commands::open_overlay(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct OcrLanguage {
    tag: String,
    name: String,
}

/// 系统已安装的 OCR 语言
#[tauri::command]
pub async fn ocr_languages() -> Result<Vec<OcrLanguage>, String> {
    tauri::async_runtime::spawn_blocking(crate::ocr::installed_languages)
        .await
        .map_err(|e| e.to_string())?
        .map(|v| v.into_iter().map(|(tag, name)| OcrLanguage { tag, name }).collect())
}

/// 当前打开的翻译框个数
#[tauri::command]
pub fn frames_count(app: AppHandle) -> usize {
    crate::live::frame_count(&app)
}

/// 选择译文库的新位置：弹出文件夹选择框，选好后把译文库搬过去。返回新路径；用户取消时返回 None
#[tauri::command]
pub async fn store_pick_and_relocate(app: AppHandle) -> Result<Option<String>, String> {
    let owner = app
        .get_webview_window(SETTINGS_LABEL)
        .and_then(|w| w.hwnd().ok())
        .map_or(0, |h| h.0 as isize);
    let initial = store::db_dir();
    let title = t!("dialog.pick_store_folder");
    let picked = tauri::async_runtime::spawn_blocking(move || {
        crate::dialog::pick_folder(owner, &title, initial.as_deref())
    })
    .await
    .map_err(|e| e.to_string())??;
    let Some(dir) = picked else { return Ok(None) };
    let path = store::relocate(&dir)?;
    let _ = app.emit("config-changed", ());
    Ok(Some(path.display().to_string()))
}

/// 把译文库搬回默认的数据目录
#[tauri::command]
pub fn store_reset_location(app: AppHandle) -> Result<String, String> {
    let path = store::relocate("")?;
    let _ = app.emit("config-changed", ());
    Ok(path.display().to_string())
}

/// 在资源管理器中定位配置文件（`config`）或译文库（`store`）
#[tauri::command]
pub fn reveal_file(which: String) -> Result<(), String> {
    let path = match which.as_str() {
        "config" => config::config_path(),
        "store" => store::db_path(),
        _ => None,
    }
    .ok_or_else(|| t!("err.dialog.unknown_file"))?;
    crate::dialog::reveal(&path)
}

/// 关闭所有翻译框，返回关闭的个数
#[tauri::command]
pub fn frames_close_all(app: AppHandle) -> usize {
    crate::live::close_all_frames(&app)
}

#[tauri::command]
pub fn app_version() -> crate::update::AppVersion {
    crate::update::current()
}

/// 最近一次检查更新的结果（还没检查过为空）
#[tauri::command]
pub fn update_state() -> Option<crate::update::UpdateInfo> {
    crate::update::last_checked()
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<crate::update::UpdateInfo, String> {
    crate::update::check(&app).await
}

/// 下载并启动新版本的安装程序（成功后本程序退出）；便携版打开下载页
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    crate::update::install(&app).await
}

#[tauri::command]
pub fn update_open_page() -> Result<(), String> {
    crate::update::open_page()
}
