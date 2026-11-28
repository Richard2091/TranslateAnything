//! 单个页面的接管会话：注入脚本、处理页面发来的原文、回填译文、切换原文/译文、停用时恢复原文。

use super::{SharedStatus, cdp, notify};
use crate::config::{self, Config, ImmersiveTarget};
use crate::{store, translate};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tauri::AppHandle;
use tokio::sync::{broadcast, watch};

/// 注入脚本运行的隔离环境名
const WORLD: &str = "translate-anything";
/// 页面脚本回调主程序用的函数名
const BINDING: &str = "__tfSend";
const SCRIPT: &str = include_str!("inject.js");
const CONFIG_SLOT: &str = "/*__TF_CONFIG__*/ {}";
/// 注入时预加载的最近常用译文条数
const PRELOAD: usize = 5000;

#[derive(Debug, Clone, Deserialize)]
pub struct TargetInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "webSocketDebuggerUrl", default)]
    pub ws: String,
}

pub async fn list_targets(port: u16) -> Result<Vec<TargetInfo>, String> {
    // 必须直连：本机环境变量里的代理会把 127.0.0.1 的请求也转走
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;
    client
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

pub fn wanted(t: &TargetInfo, target: &ImmersiveTarget) -> bool {
    if !matches!(t.kind.as_str(), "page" | "iframe" | "webview") || t.ws.is_empty() {
        return false;
    }
    // 地址为空的多是还没导航的隐藏窗口（实测 Apifox 有两个），连上去 Runtime.enable 也不响应
    // edge:// 是 WebView2 的内部页面
    if t.url.is_empty()
        || t.url.starts_with("devtools://")
        || t.url.starts_with("chrome")
        || t.url.starts_with("edge://")
        || t.url == "about:blank"
    {
        return false;
    }
    target.url_prefixes.is_empty() || target.url_prefixes.iter().any(|p| t.url.starts_with(p.as_str()))
}

/// 组装注入脚本：把配置和预加载译文（译文库最近常用 + 术语表）填进占位符
fn build_script(cfg: &Config, target: &ImmersiveTarget, show: bool) -> String {
    let target_lang = cfg.target_lang.as_str();
    let mut preload: HashMap<String, String> = if cfg.persist_cache {
        store::recent(target_lang, PRELOAD).into_iter().collect()
    } else {
        HashMap::new()
    };
    preload.extend(translate::glossary(cfg));
    let cfg = json!({
        "enabled": show,
        "skipCjk": target_lang.to_ascii_lowercase().starts_with("zh"),
        "skipSelectors": target.skip_selectors,
        "preload": preload,
    });
    SCRIPT.replacen(CONFIG_SLOT, &cfg.to_string(), 1)
}

fn frame_ids(tree: &Value, out: &mut Vec<String>) {
    if let Some(id) = tree.pointer("/frame/id").and_then(Value::as_str) {
        out.push(id.to_string());
    }
    for child in tree.get("childFrames").and_then(Value::as_array).into_iter().flatten() {
        frame_ids(child, out);
    }
}

/// 会话需要的上下文
pub struct Ctx {
    pub app: AppHandle,
    pub target: ImmersiveTarget,
    pub show: watch::Receiver<bool>,
    pub stop: watch::Receiver<bool>,
    pub patch: broadcast::Receiver<String>,
    pub status: SharedStatus,
}

/// 接管一个页面，直到它关闭、连接断开或应用被停用
pub async fn run(t: TargetInfo, ctx: Ctx) {
    let (app, status, id) = (ctx.app.clone(), ctx.status.clone(), t.id.clone());
    if let Err(e) = run_inner(&t, ctx).await {
        eprintln!("沉浸模式：页面 {} 接管失败：{e}", t.url);
    }
    status.lock().unwrap().pages.retain(|p| p.id != id);
    notify(&app);
}

async fn eval(cdp: &cdp::Cdp, ctx_id: i64, expr: &str) -> Result<Value, String> {
    cdp.call("Runtime.evaluate", json!({ "expression": expr, "contextId": ctx_id })).await
}

async fn run_inner(t: &TargetInfo, mut ctx: Ctx) -> Result<(), String> {
    let (cdp, mut events) = cdp::connect(&t.ws).await?;
    let script = build_script(&config::load()?, &ctx.target, *ctx.show.borrow());

    cdp.call("Runtime.enable", json!({})).await?;
    cdp.call("Page.enable", json!({})).await?;
    cdp.call("Runtime.addBinding", json!({ "name": BINDING, "executionContextName": WORLD }))
        .await?;
    // 以后每次加载新文档都自动注入（刷新、跳转）
    cdp.call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": script, "worldName": WORLD }))
        .await?;

    // 我们的隔离环境 id：切换原文/译文、推送修改、停用时逐个通知
    let mut contexts: HashSet<i64> = HashSet::new();
    // 当前已加载的文档：给每个框架建隔离环境并注入一次
    let mut frames = Vec::new();
    frame_ids(&cdp.call("Page.getFrameTree", json!({})).await?["frameTree"], &mut frames);
    for frame in frames {
        let world = cdp
            .call("Page.createIsolatedWorld", json!({ "frameId": frame, "worldName": WORLD }))
            .await;
        if let Some(c) = world.ok().and_then(|w| w["executionContextId"].as_i64()) {
            contexts.insert(c);
            if let Err(e) = eval(&cdp, c, &script).await {
                eprintln!("沉浸模式：注入框架 {frame} 失败：{e}");
            }
        }
    }
    println!("沉浸模式：已接管页面 {}", t.url);

    let mut stopped = *ctx.stop.borrow();
    while !stopped {
        tokio::select! {
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                match ev.method.as_str() {
                    "Runtime.bindingCalled" if ev.params["name"] == BINDING => {
                        let c = ev.params["executionContextId"].as_i64().unwrap_or(0);
                        contexts.insert(c);
                        let payload = ev.params["payload"].as_str().unwrap_or("").to_string();
                        tauri::async_runtime::spawn(answer(cdp.clone(), c, payload, ctx.app.clone(), ctx.status.clone()));
                    }
                    "Runtime.executionContextCreated" => {
                        let c = &ev.params["context"];
                        if c["name"] == WORLD {
                            if let Some(id) = c["id"].as_i64() {
                                contexts.insert(id);
                            }
                        }
                    }
                    "Runtime.executionContextDestroyed" => {
                        if let Some(id) = ev.params["executionContextId"].as_i64() {
                            contexts.remove(&id);
                        }
                    }
                    "Runtime.executionContextsCleared" => contexts.clear(),
                    "Inspector.detached" | "Inspector.targetCrashed" => break,
                    _ => {}
                }
            }
            changed = ctx.show.changed() => {
                if changed.is_err() {
                    // 运行器已被释放（停用、删除、配置变更）：与收到停止信号同样处理
                    stopped = true;
                    break;
                }
                let on = *ctx.show.borrow_and_update();
                let expr = format!("globalThis.__tf && __tf.setEnabled({on})");
                for &c in &contexts {
                    let _ = eval(&cdp, c, &expr).await;
                }
            }
            msg = ctx.patch.recv() => {
                match msg {
                    Ok(expr) => {
                        for &c in &contexts {
                            let _ = eval(&cdp, c, &expr).await;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            _ = ctx.stop.changed() => {
                stopped = true;
                break;
            }
        }
    }
    if stopped {
        // 停用：恢复原文并卸载脚本；断开连接后 CDP 登记的自动注入和 binding 随之失效
        for &c in &contexts {
            let _ = eval(&cdp, c, "globalThis.__tf && __tf.dispose(true)").await;
        }
        println!("沉浸模式：已释放页面 {}", t.url);
    } else if !cdp.is_closed() {
        println!("沉浸模式：页面 {} 已断开", t.url);
    }
    Ok(())
}

#[derive(Deserialize)]
struct Request {
    texts: Vec<String>,
}

/// 处理页面发来的一批原文：翻译（术语表 → 译文库 → 引擎）后回填
async fn answer(cdp: cdp::Cdp, ctx_id: i64, payload: String, app: AppHandle, status: SharedStatus) {
    let Ok(req) = serde_json::from_str::<Request>(&payload) else { return };
    let texts: Vec<String> = req.texts.into_iter().filter(|t| !t.trim().is_empty()).take(200).collect();
    if texts.is_empty() {
        return;
    }
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    let start = std::time::Instant::now();
    let expr = match translate::translate_batch_with_fallback(&cfg, &texts, "沉浸模式").await {
        Ok((translated, engine)) => {
            {
                let mut s = status.lock().unwrap();
                s.translated += texts.len() as u64;
                if engine != translate::cache_engine() {
                    s.engine_batches += 1;
                    println!("沉浸模式：{} 条由【{engine}】翻译", texts.len());
                }
            }
            notify(&app);
            let map: HashMap<&String, String> = texts.iter().zip(translated).collect();
            format!("globalThis.__tf && __tf.receive({})", json!({ "map": map }))
        }
        Err(e) => {
            eprintln!("沉浸模式：翻译失败：{e}");
            format!("globalThis.__tf && __tf.failed({})", json!({ "texts": texts }))
        }
    };
    let translated_at = start.elapsed();
    let _ = eval(&cdp, ctx_id, &expr).await;
    tlog!(
        "沉浸模式 本批 {} 条：翻译 {} · 回填页面 {} · 共 {}",
        texts.len(),
        crate::timing::secs(translated_at),
        crate::timing::secs(start.elapsed() - translated_at),
        crate::timing::secs(start.elapsed())
    );
}
