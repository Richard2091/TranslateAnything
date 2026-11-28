//! 翻译 Provider 抽象。配置决定一切，代码里不写死任何服务商的 key。
//!
//! 引擎清单见 [`CATALOG`]（设置界面也从这里读取可选引擎和所需字段）。

mod anthropic;
mod deepl;
mod deeplx;
mod edge;
mod gemini;
mod google;
mod google_free;
mod limiter;
mod llm;
mod microsoft;
mod openai;

use crate::config::{Config, ProviderConfig, normalize_app_id};
use crate::filter::{self, Decision, FilterContext, Mode as FilterMode, Reason};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub enum TranslateError {
    /// 配置缺失或不合法
    Config(String),
    /// 网络错误（连不上、超时等）
    Network(String),
    /// 服务端返回了错误
    Api(String),
    /// 触发速率限制（HTTP 429）
    RateLimited {
        message: String,
        retry_after: Option<Duration>,
    },
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(m) => f.write_str(&t!("err.engine.config", m = m)),
            Self::Network(m) => f.write_str(&t!("err.engine.network", m = m)),
            Self::Api(m) | Self::RateLimited { message: m, .. } => f.write_str(&t!("err.engine.api", m = m)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceMode {
    #[default]
    Ocr,
    Dom,
}

#[derive(Debug, Clone, Default)]
pub struct BatchContext {
    pub mode: SourceMode,
    pub app_id: String,
    pub app_name: String,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranslateItem {
    pub text: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub group: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslatedItem {
    pub text: String,
    pub keep: bool,
}

#[async_trait]
pub trait Translator: Send + Sync {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError>;

    /// 批量翻译；默认逐条调用，支持批量的接口应覆盖以减少请求次数
    async fn translate_batch(&self, texts: &[String], target_lang: &str) -> Result<Vec<String>, TranslateError> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.translate(t, target_lang).await?);
        }
        Ok(out)
    }

    /// 带轻量上下文与语义保留判定的批量翻译（传统引擎默认复用 `translate_batch`）
    async fn translate_items(
        &self,
        items: &[TranslateItem],
        _ctx: &BatchContext,
        _keep_terms: &[String],
        target_lang: &str,
    ) -> Result<Vec<TranslatedItem>, TranslateError> {
        let texts: Vec<String> = items.iter().map(|it| it.text.clone()).collect();
        let translated = self.translate_batch(&texts, target_lang).await?;
        Ok(items
            .iter()
            .zip(translated)
            .map(|(it, tr)| {
                let keep = tr.trim() == it.text.trim();
                TranslatedItem { text: if keep { it.text.clone() } else { tr }, keep }
            })
            .collect())
    }

    /// 指定原文语言批量翻译（`from` 为 BCP 47 代码，如 `zh`、`ja`）。
    /// 用于自动识别出错时重试，见 [`retry_misdetected`]；不支持指定原文语言的引擎返回 `None`
    async fn translate_batch_from(
        &self,
        _texts: &[String],
        _from: &str,
        _target_lang: &str,
    ) -> Result<Option<Vec<String>>, TranslateError> {
        Ok(None)
    }

    // 扩展点（P2 视觉直译）：translate_image(&self, png: &[u8], target_lang)，默认返回"不支持"
}

// ───────────────────────── 引擎清单 ─────────────────────────

/// 引擎元信息，供设置界面渲染（M3）。
/// `name` / `note` 存的是文案键，序列化时按当前界面语言换成文字
#[derive(Debug, Serialize)]
pub struct EngineInfo {
    /// 即配置里的 `type`
    pub id: &'static str,
    #[serde(serialize_with = "localized")]
    pub name: &'static str,
    /// 是否必须填写 api_key
    pub needs_key: bool,
    /// 是否为官方公开 API（否则是非官方免费接口，随时可能失效）
    pub official: bool,
    /// 该引擎会用到的配置字段
    pub fields: &'static [&'static str],
    /// 接口地址留空时实际使用的地址（DeepL 为付费版，免费版 key 以 `:fx` 结尾时改用免费版地址）；没有默认值的为空
    pub default_base_url: &'static str,
    #[serde(serialize_with = "localized")]
    pub note: &'static str,
}

fn localized<S: serde::Serializer>(key: &&'static str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&t!(key))
}

pub const CATALOG: &[EngineInfo] = &[
    EngineInfo {
        id: "edge",
        name: "engine.edge.name",
        needs_key: false,
        official: false,
        fields: &["proxy"],
        default_base_url: "",
        note: "engine.edge.note",
    },
    EngineInfo {
        id: "bing",
        name: "engine.bing.name",
        needs_key: false,
        official: false,
        fields: &["proxy"],
        default_base_url: "",
        note: "engine.bing.note",
    },
    EngineInfo {
        id: "google_free",
        name: "engine.google_free.name",
        needs_key: false,
        official: false,
        fields: &["base_url", "proxy"],
        default_base_url: google_free::DEFAULT_BASE,
        note: "engine.google_free.note",
    },
    EngineInfo {
        id: "microsoft",
        name: "engine.microsoft.name",
        needs_key: true,
        official: true,
        fields: &["api_key", "region", "base_url", "proxy"],
        default_base_url: microsoft::DEFAULT_BASE,
        note: "engine.microsoft.note",
    },
    EngineInfo {
        id: "deepl",
        name: "engine.deepl.name",
        needs_key: true,
        official: true,
        fields: &["api_key", "base_url", "proxy"],
        default_base_url: deepl::PRO_BASE,
        note: "engine.deepl.note",
    },
    EngineInfo {
        id: "deeplx",
        name: "engine.deeplx.name",
        needs_key: false,
        official: false,
        fields: &["base_url", "api_key", "proxy"],
        default_base_url: deeplx::DEFAULT_URL,
        note: "engine.deeplx.note",
    },
    EngineInfo {
        id: "google",
        name: "engine.google.name",
        needs_key: true,
        official: true,
        fields: &["api_key", "base_url", "proxy"],
        default_base_url: google::DEFAULT_BASE,
        note: "engine.google.note",
    },
    EngineInfo {
        id: "openai",
        name: "engine.openai.name",
        needs_key: true,
        official: true,
        fields: &["base_url", "api_key", "model", "concurrency", "proxy"],
        default_base_url: "",
        note: "engine.openai.note",
    },
    EngineInfo {
        id: "anthropic",
        name: "engine.anthropic.name",
        needs_key: true,
        official: true,
        fields: &["base_url", "api_key", "model", "concurrency", "proxy"],
        default_base_url: anthropic::DEFAULT_BASE,
        note: "engine.anthropic.note",
    },
    EngineInfo {
        id: "gemini",
        name: "engine.gemini.name",
        needs_key: true,
        official: true,
        fields: &["base_url", "api_key", "model", "concurrency", "proxy"],
        default_base_url: gemini::DEFAULT_BASE,
        note: "engine.gemini.note",
    },
];

/// 按引擎类型和实例 id 创建 Provider
pub fn create_with_id(id: &str, kind: &str, cfg: &ProviderConfig) -> Result<Box<dyn Translator>, TranslateError> {
    Ok(match kind {
        "edge" => Box::new(edge::Edge::new(cfg, edge::Route::EdgeThenBing)?),
        "bing" => Box::new(edge::Edge::new(cfg, edge::Route::BingOnly)?),
        "google_free" => Box::new(google_free::GoogleFree::new(cfg)?),
        "microsoft" => Box::new(microsoft::Microsoft::new(cfg)?),
        "deepl" => Box::new(deepl::DeepL::new(cfg)?),
        "deeplx" => Box::new(deeplx::DeepLX::new(cfg)?),
        "google" => Box::new(google::Google::new(cfg)?),
        "openai" => Box::new(openai::OpenAICompatible::new_with_id(id, cfg)?),
        "anthropic" => Box::new(anthropic::Anthropic::new_with_id(id, cfg)?),
        "gemini" => Box::new(gemini::Gemini::new_with_id(id, cfg)?),
        // 新建的自定义接口还没选协议（类型留空时 kind 回退成实例 id，AI 实例 id 都以 ai_ 开头）
        other if is_unset_ai(other) => return Err(TranslateError::Config(t!("err.engine.no_protocol"))),
        other => {
            let ids: Vec<_> = CATALOG.iter().map(|e| e.id).collect();
            return Err(TranslateError::Config(t!(
                "err.engine.unsupported",
                kind = other,
                ids = ids.join(" / ")
            )));
        }
    })
}

/// AI 供应商实例尚未选择协议
fn is_unset_ai(kind: &str) -> bool {
    kind.starts_with("ai_")
}

/// 按引擎类型创建 Provider
pub fn create(kind: &str, cfg: &ProviderConfig) -> Result<Box<dyn Translator>, TranslateError> {
    create_with_id(kind, kind, cfg)
}

/// 引擎的显示名（按当前界面语言）；不在清单里的原样返回
fn engine_name(kind: &str) -> String {
    CATALOG.iter().find(|e| e.id == kind).map_or_else(|| kind.to_string(), |e| t!(e.name))
}

/// 引擎实例的显示名：优先使用用户配置的名称，留空时回退到类型名
pub fn provider_display_name(id: &str, provider: &ProviderConfig) -> String {
    let custom = provider.name.trim();
    if !custom.is_empty() {
        return custom.to_string();
    }
    engine_name(&provider.kind_or(id))
}

/// 本地缓存全部命中时返回的"引擎名"（按当前界面语言）
pub fn cache_engine() -> String {
    t!("engine.cache")
}

/// 术语表：内置界面常用词（`glossary.json`，按目标语言）+ 配置里的 `glossary`（后者优先）。
/// 按整条原文精确匹配（忽略首尾空白），命中的不查库也不请求引擎。
/// 传统翻译引擎翻单个界面词没有上下文，容易出"New → 新款""Auto → 汽车"这类错，靠它兜住。
pub fn glossary(cfg: &Config) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if cfg.builtin_glossary {
        map.extend(builtin_glossary(&cfg.target_lang));
    }
    map.extend(cfg.glossary.iter().map(|(k, v)| (k.trim().to_string(), v.clone())));
    map
}

/// 某个目标语言的内置术语表（设置界面展示用）
pub fn builtin_glossary(target_lang: &str) -> std::collections::BTreeMap<String, String> {
    static BUILTIN: OnceLock<HashMap<String, std::collections::BTreeMap<String, String>>> = OnceLock::new();
    let builtin = BUILTIN.get_or_init(|| serde_json::from_str(include_str!("glossary.json")).unwrap_or_default());
    let key = match primary(target_lang).as_str() {
        "zh" if is_traditional(target_lang) => "zh-TW".to_string(),
        "zh" => "zh-CN".to_string(),
        p => p.to_string(),
    };
    builtin.get(&key).cloned().unwrap_or_default()
}

/// 批量翻译兼容入口（不带上下文）
#[allow(dead_code)]
pub async fn translate_batch_with_fallback(
    cfg: &Config,
    texts: &[String],
    who: &str,
) -> Result<(Vec<String>, String), String> {
    let items: Vec<TranslateItem> = texts
        .iter()
        .map(|t| TranslateItem { text: t.clone(), role: None, group: None })
        .collect();
    let ctx = BatchContext::default();
    let (res, engine) = translate_items_with_fallback(cfg, &items, &ctx, who).await?;
    Ok((res.into_iter().map(|r| r.text).collect(), engine))
}

type InflightKey = (String, String, String);
type InflightMap = HashMap<InflightKey, watch::Receiver<Option<Result<TranslatedItem, String>>>>;
static INFLIGHT: OnceLock<Mutex<InflightMap>> = OnceLock::new();

fn inflight_map() -> &'static Mutex<InflightMap> {
    INFLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

struct InflightGuard {
    senders: HashMap<InflightKey, watch::Sender<Option<Result<TranslatedItem, String>>>>,
}

impl InflightGuard {
    fn complete(&mut self, results: &[(InflightKey, Result<TranslatedItem, String>)]) {
        if !self.senders.is_empty() {
            let mut map = inflight_map().lock().unwrap();
            for (k, _) in results {
                map.remove(k);
            }
        }
        for (k, res) in results {
            if let Some(tx) = self.senders.remove(k) {
                let _ = tx.send(Some(res.clone()));
            }
        }
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        if self.senders.is_empty() {
            return;
        }
        let mut map = inflight_map().lock().unwrap();
        for (k, tx) in self.senders.drain() {
            map.remove(&k);
            let _ = tx.send(Some(Err("请求已取消".into())));
        }
    }
}

fn scope_key(text: &str, app_id: &str) -> String {
    let norm = normalize_app_id(app_id);
    if !norm.is_empty() && crate::store::is_app_scoped(text, false) {
        norm
    } else {
        String::new()
    }
}

/// 批量翻译（带过滤、上下文、在途去重、缓存隔离）：
/// 本地过滤 → 术语表 → 译文库 → 在途合并 & 翻译引擎 → 写回译文库
pub async fn translate_items_with_fallback(
    cfg: &Config,
    items: &[TranslateItem],
    ctx: &BatchContext,
    who: &str,
) -> Result<(Vec<TranslatedItem>, String), String> {
    use crate::timing::secs;
    if items.is_empty() {
        return Ok((Vec::new(), cache_engine()));
    }
    let start = Instant::now();
    let lang = cfg.target_lang.as_str();
    let app_id = normalize_app_id(&ctx.app_id);
    let fmode = match ctx.mode {
        SourceMode::Dom => FilterMode::Immersive,
        SourceMode::Ocr => FilterMode::Overlay,
    };
    let fctx = FilterContext {
        target_lang: lang,
        app_id: &app_id,
        app_name: &ctx.app_name,
        mode: fmode,
    };

    let mut out: Vec<Option<TranslatedItem>> = vec![None; items.len()];
    let mut forced = vec![false; items.len()];
    let mut filter_kept = 0usize;

    // 1. 统一本地过滤
    for (i, it) in items.iter().enumerate() {
        match filter::classify(cfg, &it.text, &fctx) {
            Decision::Keep(reason) => {
                filter::record_event(&it.text, "keep", reason, &fctx);
                out[i] = Some(TranslatedItem { text: it.text.clone(), keep: true });
                filter_kept += 1;
            }
            Decision::ForceTranslate => {
                filter::record_event(&it.text, "translate", Reason::ForceTranslate, &fctx);
                forced[i] = true;
            }
            Decision::Translate => {}
        }
    }

    // 2. 术语表（整条精确匹配；用户强制翻译且术语原样保留时不拦截）
    let terms = glossary(cfg);
    let mut glossary_hits = 0usize;
    for (i, it) in items.iter().enumerate() {
        if out[i].is_some() {
            continue;
        }
        if let Some(g) = terms.get(it.text.trim()) {
            let keep = g.trim() == it.text.trim();
            if forced[i] && keep {
                continue;
            }
            out[i] = Some(TranslatedItem {
                text: if keep { it.text.clone() } else { g.clone() },
                keep,
            });
            glossary_hits += 1;
        }
    }

    // 3. 本地译文库查询（内联拆碎的片段跳过译文库，保留完整句上下文）
    let t = Instant::now();
    let mut store_hits = 0usize;
    let store_sources: Vec<String> = items
        .iter()
        .enumerate()
        .filter(|(i, it)| out[*i].is_none() && it.group.is_none())
        .map(|(_, it)| it.text.clone())
        .collect();
    if cfg.persist_cache && !store_sources.is_empty() {
        let hit_map = crate::store::get_many_for_app(lang, &app_id, &ctx.app_name, &store_sources);
        for (i, it) in items.iter().enumerate() {
            if out[i].is_some() || it.group.is_some() {
                continue;
            }
            if let Some((tr, keep)) = hit_map.get(&it.text) {
                if forced[i] && *keep {
                    continue;
                }
                out[i] = Some(TranslatedItem {
                    text: if *keep { it.text.clone() } else { tr.clone() },
                    keep: *keep,
                });
                store_hits += 1;
            }
        }
    }
    let store_time = t.elapsed();

    // 4. 收集未命中项，非内联片段走全局在途去重
    let mut waiting: Vec<(usize, watch::Receiver<Option<Result<TranslatedItem, String>>>)> = Vec::new();
    let mut engine_indices: Vec<Vec<usize>> = Vec::new();
    let mut engine_items: Vec<TranslateItem> = Vec::new();
    let mut engine_keys: Vec<Option<InflightKey>> = Vec::new();
    let mut owned_map: HashMap<InflightKey, usize> = HashMap::new();
    let mut guard = InflightGuard { senders: HashMap::new() };

    {
        let mut inflight = inflight_map().lock().unwrap();
        for (i, it) in items.iter().enumerate() {
            if out[i].is_some() {
                continue;
            }
            if it.group.is_some() {
                engine_indices.push(vec![i]);
                engine_items.push(it.clone());
                engine_keys.push(None);
                continue;
            }
            let key = (lang.to_string(), scope_key(&it.text, &app_id), it.text.clone());
            if let Some(&pos) = owned_map.get(&key) {
                engine_indices[pos].push(i);
            } else if let Some(rx) = inflight.get(&key) {
                waiting.push((i, rx.clone()));
            } else {
                let (tx, rx) = watch::channel(None);
                inflight.insert(key.clone(), rx);
                guard.senders.insert(key.clone(), tx);
                let pos = engine_items.len();
                owned_map.insert(key.clone(), pos);
                engine_indices.push(vec![i]);
                engine_items.push(it.clone());
                engine_keys.push(Some(key));
            }
        }
    }

    let engine = if engine_items.is_empty() && waiting.is_empty() {
        tlog!(
            "{who} 翻译 {} 条：过滤 {filter_kept} · 术语表 {glossary_hits} · 译文库 {} 命中 {store_hits} · 共 {}",
            items.len(),
            secs(store_time),
            secs(start.elapsed())
        );
        cache_engine()
    } else {
        let mut used_name = cache_engine();
        if !engine_items.is_empty() {
            let t = Instant::now();
            let rule_set = filter::active_keep_terms(cfg, &app_id);
            let keep_terms: Vec<String> = rule_set
                .keep_terms
                .into_iter()
                .filter(|term| {
                    let t_trim = term.trim();
                    !t_trim.is_empty()
                        && engine_items
                            .iter()
                            .any(|it| it.text.to_lowercase().contains(&t_trim.to_lowercase()))
                })
                .take(30)
                .collect();

            let result = translate_items_by_engines(cfg, &engine_items, ctx, &keep_terms).await;
            let api_time = t.elapsed();
            let (translated, id, display_name, is_ai) = match result {
                Ok(v) => v,
                Err(e) => {
                    let err_pairs: Vec<(InflightKey, Result<TranslatedItem, String>)> = engine_keys
                        .into_iter()
                        .flatten()
                        .map(|k| (k, Err(e.clone())))
                        .collect();
                    guard.complete(&err_pairs);
                    tlog!("{who} 翻译 {} 条：引擎全部失败，耗时 {}", engine_items.len(), secs(api_time));
                    return Err(e);
                }
            };

            let mut to_store: Vec<(String, String, bool)> = Vec::new();
            let mut done_pairs: Vec<(InflightKey, Result<TranslatedItem, String>)> = Vec::new();
            let mut ok_reason_pairs = Vec::new();

            for ((it, tr), (idxs, key_opt)) in engine_items
                .iter()
                .zip(translated)
                .zip(engine_indices.into_iter().zip(engine_keys.into_iter()))
            {
                if tr.keep {
                    ok_reason_pairs.push((
                        it.text.clone(),
                        if is_ai { Reason::AiKeep } else { Reason::EngineUnchanged },
                    ));
                }
                if it.group.is_none() {
                    to_store.push((it.text.clone(), tr.text.clone(), tr.keep));
                }
                if let Some(key) = key_opt {
                    done_pairs.push((key, Ok(tr.clone())));
                }
                for idx in idxs {
                    out[idx] = Some(tr.clone());
                }
            }

            let t_save = Instant::now();
            if cfg.persist_cache && !to_store.is_empty() {
                crate::store::put_many_for_app(lang, &app_id, &ctx.app_name, &to_store, &id);
            }
            let save_time = t_save.elapsed();
            for (src, reason) in ok_reason_pairs {
                filter::record_event(&src, "keep", reason, &fctx);
            }
            guard.complete(&done_pairs);
            used_name = display_name;
            tlog!(
                "{who} 翻译 {} 条：过滤 {filter_kept} · 术语表 {glossary_hits} · 译文库 {} 命中 {store_hits} · 引擎【{used_name}】{} 条 {} · 写库 {} · 共 {}",
                items.len(),
                secs(store_time),
                engine_items.len(),
                secs(api_time),
                secs(save_time),
                secs(start.elapsed())
            );
        }

        // 等待已在途的其他请求完成
        for (idx, mut rx) in waiting {
            loop {
                if let Some(res) = rx.borrow().clone() {
                    out[idx] = Some(res?);
                    break;
                }
                if rx.changed().await.is_err() {
                    return Err("并发请求意外中止".into());
                }
            }
        }
        used_name
    };

    Ok((
        out.into_iter()
            .enumerate()
            .map(|(i, v)| v.unwrap_or_else(|| TranslatedItem { text: items[i].text.clone(), keep: true }))
            .collect(),
        engine,
    ))
}

// ───────────────────────── 单项失败冷却 ─────────────────────────

const COOLDOWN_DURATION: Duration = Duration::from_secs(180);

static COOLDOWNS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

fn cooldown_map() -> &'static Mutex<HashMap<String, Instant>> {
    COOLDOWNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_cooling_at(map: &mut HashMap<String, Instant>, id: &str, now: Instant) -> bool {
    let Some(&until) = map.get(id) else {
        return false;
    };
    if now < until {
        true
    } else {
        map.remove(id);
        false
    }
}

pub fn is_cooling(id: &str) -> bool {
    let mut map = cooldown_map().lock().unwrap();
    is_cooling_at(&mut map, id, Instant::now())
}

pub fn mark_cooldown(id: &str) {
    let mut map = cooldown_map().lock().unwrap();
    map.insert(id.to_string(), Instant::now() + COOLDOWN_DURATION);
}

pub fn clear_cooldown(id: &str) {
    cooldown_map().lock().unwrap().remove(id);
}

pub fn clear_all_cooldowns() {
    cooldown_map().lock().unwrap().clear();
}

pub fn cooling_ids() -> Vec<String> {
    let now = Instant::now();
    let mut map = cooldown_map().lock().unwrap();
    map.retain(|_, &mut until| now < until);
    let mut ids: Vec<String> = map.keys().cloned().collect();
    ids.sort();
    ids
}

fn is_ai_kind(kind: &str) -> bool {
    matches!(kind, "openai" | "anthropic" | "gemini")
}

/// 按配置的主引擎 + 回退顺序依次尝试批量翻译（不查缓存），返回（译文，引擎 id，显示名，是否 AI）
async fn translate_items_by_engines(
    cfg: &Config,
    items: &[TranslateItem],
    ctx: &BatchContext,
    keep_terms: &[String],
) -> Result<(Vec<TranslatedItem>, String, String, bool), String> {
    let chain = cfg.engine_chain();
    let active: Vec<(String, ProviderConfig)> = chain.iter().filter(|(id, _)| !is_cooling(id)).cloned().collect();
    // 全部处于冷却时仍按完整顺序尝试，避免可用节点全部被跳过
    let candidates = if active.is_empty() { chain } else { active };

    let mut errors = Vec::new();
    for (id, provider) in candidates {
        let kind = provider.kind_or(&id);
        let ai = is_ai_kind(&kind);
        let conc = provider.effective_concurrency(&kind);
        let display_name = provider_display_name(&id, &provider);
        let start = Instant::now();
        let (engine, result) = match create_with_id(&id, &kind, &provider) {
            Ok(t) => {
                let r = if ai {
                    // AI 引擎内部每次 chat() 已走 limiter::run_limited
                    t.translate_items(items, ctx, keep_terms, &cfg.target_lang).await
                } else {
                    limiter::run_limited(&id, conc, || t.translate_items(items, ctx, keep_terms, &cfg.target_lang)).await
                };
                (Some(t), r)
            }
            Err(e) => (None, Err(e)),
        };
        if !matches!(&result, Ok(t) if t.len() == items.len()) {
            tlog!("引擎【{display_name}】失败，改用下一个，已耗时 {}", crate::timing::secs(start.elapsed()));
        }
        match result {
            Ok(mut out) if out.len() == items.len() => {
                limiter::gate(&id, conc).on_success();
                clear_cooldown(&id);
                if !ai {
                    if let Some(e) = &engine {
                        let texts: Vec<String> = items.iter().map(|it| it.text.clone()).collect();
                        let mut raw: Vec<String> = out.iter().map(|r| r.text.clone()).collect();
                        retry_misdetected(e.as_ref(), &texts, &mut raw, &cfg.target_lang).await;
                        for (res, (orig, tr)) in out.iter_mut().zip(texts.iter().zip(raw)) {
                            let keep = tr.trim() == orig.trim();
                            res.text = if keep { orig.clone() } else { tr };
                            res.keep = keep;
                        }
                    }
                }
                return Ok((out, id, display_name, ai));
            }
            Ok(_) => {
                if limiter::gate(&id, conc).on_failure(false) {
                    mark_cooldown(&id);
                }
                errors.push(t!("err.engine.count_mismatch", name = display_name));
            }
            Err(e) => {
                let is_rl = matches!(e, TranslateError::RateLimited { .. });
                if limiter::gate(&id, conc).on_failure(is_rl) {
                    mark_cooldown(&id);
                }
                errors.push(t!("err.engine.prefixed", name = display_name, e = e));
            }
        }
    }
    Err(errors.join("\n"))
}

/// 按文字系统推断原文语言：只看目标语言用不到的文字（假名 → 日文，谚文 → 韩文，汉字 → 中文）。
/// 原文含有这类文字、引擎却原样返回，多半是自动识别错了（"GitHub 提示: NOTE" 拉丁字母多，被判成英文，
/// 目标又是英文就不翻），此时返回该语言供重试；原文本来就是目标语言的文字时返回 `None`
fn misdetected_source(text: &str, target_lang: &str) -> Option<&'static str> {
    let has = |f: fn(char) -> bool| text.chars().any(f);
    let kana = |c: char| matches!(c, '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}');
    let hangul = |c: char| matches!(c, '\u{AC00}'..='\u{D7AF}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}');
    let han = |c: char| matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}');
    let target = primary(target_lang);
    if has(kana) {
        return (target != "ja").then_some("ja");
    }
    if has(hangul) {
        return (target != "ko").then_some("ko");
    }
    // 日文也用汉字，目标为日文时汉字不能说明认错了
    if has(han) {
        return (target != "zh" && target != "ja").then_some("zh");
    }
    None
}

/// 引擎原样返回、但原文含有目标语言用不到的文字的条目，指定原文语言重试一次（同一引擎）。
/// 引擎不支持指定原文语言或重试失败时保留第一次的结果
async fn retry_misdetected(engine: &dyn Translator, texts: &[String], out: &mut [String], target_lang: &str) {
    let mut groups: HashMap<&'static str, Vec<usize>> = HashMap::new();
    for (i, (src, tr)) in texts.iter().zip(out.iter()).enumerate() {
        if src.trim() == tr.trim() {
            if let Some(from) = misdetected_source(src, target_lang) {
                groups.entry(from).or_default().push(i);
            }
        }
    }
    for (from, idx) in groups {
        let batch: Vec<String> = idx.iter().map(|&i| texts[i].clone()).collect();
        match engine.translate_batch_from(&batch, from, target_lang).await {
            Ok(Some(v)) if v.len() == idx.len() => {
                for (i, tr) in idx.into_iter().zip(v) {
                    out[i] = tr;
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("指定原文语言（{from}）重试失败，保留原结果：{e}"),
        }
    }
}

/// 大模型接口可用的模型 id（已排序去重）
pub async fn list_models(kind: &str, cfg: &ProviderConfig) -> Result<Vec<String>, String> {
    if is_unset_ai(kind) {
        return Err(t!("err.engine.no_protocol"));
    }
    let res = match kind {
        "anthropic" => anthropic::list_models(cfg).await,
        "gemini" => gemini::list_models(cfg).await,
        _ => openai::list_models(cfg).await,
    };
    res.map_err(|e| e.to_string())
}

/// 引擎测试结果（设置窗口"测试"按钮）
#[derive(Debug, Serialize)]
pub struct EngineTest {
    pub engine: String,
    /// 测试用原文与译文
    pub source: String,
    pub text: String,
    /// 耗时（毫秒），含建立连接
    pub ms: u64,
}

const TEST_SAMPLE: &str = "Hello! Screen text, translated anywhere.";

/// 用一句固定的英文测试某个已配置的引擎（不查缓存、不写译文库、不回退）
pub async fn test_engine(cfg: &Config, id: &str) -> Result<EngineTest, String> {
    let provider = cfg.provider(id);
    let kind = provider.kind_or(id);
    let name = provider_display_name(id, &provider);
    let start = Instant::now();
    let text = create(&kind, &provider)
        .map_err(|e| e.to_string())?
        .translate(TEST_SAMPLE, &cfg.target_lang)
        .await
        .map_err(|e| e.to_string())?;
    clear_cooldown(id);
    Ok(EngineTest {
        engine: name,
        source: TEST_SAMPLE.into(),
        text,
        ms: start.elapsed().as_millis() as u64,
    })
}

/// 首页链路测速单步结果（随探测逐个发出，也作为完整测速返回值）
#[derive(Debug, Clone, Serialize)]
pub struct ChainTestStep {
    pub run_id: String,
    pub id: String,
    pub name: String,
    /// `testing` | `ok` | `error`
    pub status: String,
    pub ms: Option<u64>,
    pub source: Option<String>,
    pub text: Option<String>,
    pub error: Option<String>,
}

/// 按当前生效调用链逐节点测速（忽略冷却、不查译文库、不写库；首个节点成功即停止）
pub async fn test_chain<F: Fn(&ChainTestStep)>(cfg: &Config, run_id: &str, on_step: F) -> Vec<ChainTestStep> {
    let mut steps = Vec::new();
    for (id, provider) in cfg.engine_chain() {
        let kind = provider.kind_or(&id);
        let name = provider_display_name(&id, &provider);
        let testing = ChainTestStep {
            run_id: run_id.to_string(),
            id: id.clone(),
            name: name.clone(),
            status: "testing".into(),
            ms: None,
            source: None,
            text: None,
            error: None,
        };
        on_step(&testing);

        let start = Instant::now();
        let res = match create(&kind, &provider) {
            Ok(t) => t.translate(TEST_SAMPLE, &cfg.target_lang).await.map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        match res {
            Ok(text) => {
                clear_cooldown(&id);
                let step = ChainTestStep {
                    run_id: run_id.to_string(),
                    id,
                    name,
                    status: "ok".into(),
                    ms: Some(elapsed_ms),
                    source: Some(TEST_SAMPLE.into()),
                    text: Some(text),
                    error: None,
                };
                on_step(&step);
                steps.push(step);
                break;
            }
            Err(err) => {
                let step = ChainTestStep {
                    run_id: run_id.to_string(),
                    id,
                    name,
                    status: "error".into(),
                    ms: Some(elapsed_ms),
                    source: None,
                    text: None,
                    error: Some(err),
                };
                on_step(&step);
                steps.push(step);
            }
        }
    }
    steps
}

// ───────────────────────── 公共工具 ─────────────────────────

/// 免 key 接口伪装成浏览器时使用的 UA
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0";

/// 按配置构造 HTTP 客户端
///
/// `cfg.proxy` 应是已按全局解析过的值（见 `Config::provider`）：留空或 `direct` = 直连
/// （本机环境变量里的代理曾导致 HTTPS 握手失败，所以不默认跟随环境变量）；
/// `system` = 跟随系统环境变量；其他 = 代理地址
fn build_client(cfg: &ProviderConfig) -> Result<reqwest::Client, TranslateError> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60));
    builder = match cfg.proxy.as_deref().map(str::trim) {
        Some("system") => builder,
        Some("direct") => builder.no_proxy(),
        Some(p) if !p.is_empty() => builder.proxy(
            reqwest::Proxy::all(p).map_err(|e| TranslateError::Config(t!("err.engine.bad_proxy", e = e)))?,
        ),
        _ => builder.no_proxy(),
    };
    builder
        .build()
        .map_err(|e| TranslateError::Config(e.to_string()))
}

/// 只按代理设置构造 HTTP 客户端（检查更新等非翻译请求用）；`proxy` 取值同 [`build_client`]
pub fn proxy_client(proxy: &str, timeout: Duration) -> Result<reqwest::Client, TranslateError> {
    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(timeout);
    builder = match proxy.trim() {
        "system" => builder,
        "" | "direct" => builder.no_proxy(),
        p => builder.proxy(reqwest::Proxy::all(p).map_err(|e| TranslateError::Config(t!("err.engine.bad_proxy", e = e)))?),
    };
    builder.build().map_err(|e| TranslateError::Config(e.to_string()))
}

fn require_key(cfg: &ProviderConfig, hint: &str) -> Result<String, TranslateError> {
    let key = cfg.api_key.trim();
    if key.is_empty() || key == "sk-xxx" || key.contains("在此填写") {
        return Err(TranslateError::Config(t!("err.engine.no_key", hint = hint)));
    }
    Ok(key.to_string())
}

fn network_err(e: reqwest::Error) -> TranslateError {
    let host = e.url().and_then(|u| u.host_str()).unwrap_or("").to_string();
    let mut msg = if e.is_timeout() { t!("err.engine.timeout") } else { e.to_string() };
    // 带上底层原因，方便排查（TLS、DNS 等）
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        msg.push_str(&format!("：{s}"));
        src = s.source();
    }
    if !host.is_empty() {
        msg.push_str(&format!("（{host}）"));
    }
    TranslateError::Network(msg)
}

/// 发送请求并解析 JSON；非 2xx 时用 `err_pointer` 指向的字段作为错误信息（429 返回 RateLimited）
async fn send_json(req: reqwest::RequestBuilder, err_pointer: &str) -> Result<Value, TranslateError> {
    let resp = req.send().await.map_err(network_err)?;
    let status = resp.status();
    let retry_after = limiter::parse_retry_after(
        resp.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()),
    );
    let raw = resp.text().await.map_err(network_err)?;
    let value: Option<Value> = serde_json::from_str(&raw).ok();
    if !status.is_success() {
        let msg = value
            .as_ref()
            .and_then(|v| v.pointer(err_pointer))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| raw.chars().take(300).collect());
        let formatted = format!("HTTP {status}：{msg}");
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(TranslateError::RateLimited { message: formatted, retry_after });
        }
        return Err(TranslateError::Api(formatted));
    }
    value.ok_or_else(|| TranslateError::Api(t!("err.engine.not_json", body = raw.chars().take(300).collect::<String>())))
}

fn parse_err(v: &Value) -> TranslateError {
    TranslateError::Api(t!("err.engine.bad_response", body = v.to_string().chars().take(300).collect::<String>()))
}

/// 表单编码（application/x-www-form-urlencoded）
fn form_encode(pairs: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse("http://localhost/").unwrap();
    url.query_pairs_mut().extend_pairs(pairs);
    url.query().unwrap_or("").to_string()
}

/// 把 OCR 的硬换行合并成自然段，传统翻译引擎逐行翻译会把句子拆碎
///
/// 规则：上一行以句末标点结尾才保留换行；英文行尾连字符断词时去掉连字符直接拼接
pub fn merge_lines(text: &str) -> String {
    const ENDS: &[char] = &['.', '!', '?', ':', ';', '。', '！', '？', '：', '；', '…', '"', '”', ')', '）'];
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if out.is_empty() {
            out.push_str(line);
            continue;
        }
        let last = out.chars().last().unwrap();
        let first = line.chars().next().unwrap();
        if ENDS.contains(&last) {
            out.push('\n');
        } else if last == '-' && first.is_lowercase() {
            out.pop();
        } else if last.is_ascii() || first.is_ascii() {
            out.push(' ');
        }
        // 两边都是 CJK 字符时直接拼接
        out.push_str(line);
    }
    out
}

// ───────────────────────── 语言代码映射 ─────────────────────────

/// 语言标签的主子标签（小写），如 `zh-CN` → `zh`
fn primary(lang: &str) -> String {
    lang.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase()
}

/// 是否为繁体中文标签
fn is_traditional(lang: &str) -> bool {
    let l = lang.to_ascii_lowercase();
    l.contains("tw") || l.contains("hk") || l.contains("mo") || l.contains("hant")
}

/// 微软系：zh-CN → zh-Hans，zh-TW → zh-Hant，其余取主子标签
fn ms_lang(lang: &str) -> String {
    match primary(lang).as_str() {
        "zh" if is_traditional(lang) => "zh-Hant".into(),
        "zh" => "zh-Hans".into(),
        p => p.into(),
    }
}

/// Google 系：中文用 zh-CN / zh-TW，其余取主子标签
fn google_lang(lang: &str) -> String {
    match primary(lang).as_str() {
        "zh" if is_traditional(lang) => "zh-TW".into(),
        "zh" => "zh-CN".into(),
        p => p.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 合并英文断行() {
        let s = "This is a sentence that\nwas broken by OCR. And a hyph-\nenated word.\nNew para";
        assert_eq!(
            merge_lines(s),
            "This is a sentence that was broken by OCR. And a hyphenated word.\nNew para"
        );
    }

    #[test]
    fn 合并中文断行() {
        assert_eq!(merge_lines("这是一句被\n打断的话。\n下一段"), "这是一句被打断的话。\n下一段");
    }

    #[test]
    fn 语言映射() {
        assert_eq!(ms_lang("zh-CN"), "zh-Hans");
        assert_eq!(ms_lang("zh-TW"), "zh-Hant");
        assert_eq!(google_lang("zh-Hans"), "zh-CN");
        assert_eq!(ms_lang("ja-JP"), "ja");
    }

    #[test]
    fn 术语表() {
        let mut cfg = Config::default();
        let g = glossary(&cfg);
        assert_eq!(g["New"], "新建");
        cfg.glossary.insert(" New ".into(), "新的".into());
        assert_eq!(glossary(&cfg)["New"], "新的", "用户术语优先");
        cfg.builtin_glossary = false;
        cfg.glossary.clear();
        assert!(glossary(&cfg).is_empty());
        cfg.builtin_glossary = true;
        cfg.target_lang = "ja".into();
        assert!(!glossary(&cfg).contains_key("New"), "其他目标语言不用中文术语");
    }

    #[test]
    fn 清单覆盖所有引擎() {
        let cfg = ProviderConfig::default();
        let ids: Vec<_> = CATALOG.iter().map(|e| e.id).collect();
        for e in CATALOG {
            // 只要不是"不支持的类型"即可（缺 key 之类的配置错误是预期的）
            if let Err(TranslateError::Config(m)) = create(e.id, &cfg) {
                let unsupported = t!("err.engine.unsupported", kind = e.id, ids = ids.join(" / "));
                assert_ne!(m, unsupported, "{}", e.id);
            }
        }
    }

    /// 联网实测：OpenRouter 的模型列表不需要密钥
    #[tokio::test]
    #[ignore]
    async fn 联网实测模型列表() {
        let cfg = ProviderConfig {
            base_url: "https://openrouter.ai/api/v1".into(),
            proxy: Some("http://127.0.0.1:7897".into()),
            ..Default::default()
        };
        let ids = list_models("openai", &cfg).await.unwrap();
        println!("{} 个模型，如 {:?}", ids.len(), &ids[..3.min(ids.len())]);
        assert!(!ids.is_empty());
    }

    #[test]
    fn 供应商名称优先自定义() {
        let mut p = ProviderConfig::default();
        p.kind = "anthropic".into();
        assert_eq!(provider_display_name("ai_1", &p), engine_name("anthropic"));
        p.name = "  Claude 主力  ".into();
        assert_eq!(provider_display_name("ai_1", &p), "Claude 主力");
    }

    #[test]
    fn 单项冷却判定与清理() {
        let mut map = HashMap::new();
        let now = Instant::now();
        map.insert("a".to_string(), now + Duration::from_secs(60));
        map.insert("expired".to_string(), now - Duration::from_secs(1));
        assert!(is_cooling_at(&mut map, "a", now));
        assert!(!is_cooling_at(&mut map, "expired", now));
        assert!(!map.contains_key("expired"));
    }

    #[test]
    fn 推断被认错的原文语言() {
        assert_eq!(misdetected_source("GitHub 提示: NOTE", "en"), Some("zh"));
        assert_eq!(misdetected_source("上传 .pdf, .txt 等文件", "en-US"), Some("zh"));
        assert_eq!(misdetected_source("Open ファイル", "en"), Some("ja"));
        assert_eq!(misdetected_source("Save 저장", "zh-CN"), Some("ko"));
        // 原文就是目标语言用的文字：引擎原样返回是正常的
        assert_eq!(misdetected_source("GitHub 提示", "zh-CN"), None);
        assert_eq!(misdetected_source("GitHub 設定", "ja"), None);
        assert_eq!(misdetected_source("Claude", "en"), None);
    }

    /// 联网实测：中英混排的文字指定原文语言后能译出（自动识别时 Edge 会原样返回）
    #[tokio::test]
    #[ignore]
    async fn 联网实测混排重试() {
        let texts: Vec<String> = ["GitHub 提示: NOTE", "上传 .pdf, .txt, .md, .json 等文件", "Chinese (Simplified) 简体中文"]
            .map(String::from)
            .to_vec();
        for id in ["edge", "bing"] {
            let e = create(id, &ProviderConfig::default()).unwrap();
            let mut out = e.translate_batch(&texts, "en").await.unwrap();
            println!("{id} 自动识别：{out:?}");
            retry_misdetected(e.as_ref(), &texts, &mut out, "en").await;
            println!("{id} 重试后：{out:?}");
            for (s, t) in texts.iter().zip(&out) {
                assert!(misdetected_source(t, "en").is_none() || s != t, "{id} 仍原样返回：{s}");
            }
        }
    }

    /// 联网实测：cargo test -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn 联网实测免费引擎() {
        let text = "Hello world, this is a\ntest of the translation filter.";
        for (id, proxy) in [("edge", None), ("bing", None), ("google_free", Some("http://127.0.0.1:7897"))] {
            let cfg = ProviderConfig {
                proxy: proxy.map(str::to_string),
                ..Default::default()
            };
            let r = create(id, &cfg).unwrap().translate(text, "zh-CN").await;
            println!("{id}: {r:?}");
            assert!(r.is_ok(), "{id} 失败");
        }
    }
}
