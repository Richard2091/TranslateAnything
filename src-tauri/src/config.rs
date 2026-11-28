//! 配置读写：`~/.translate-anything/config.json`
//!
//! 每次翻译时重新读取，修改配置文件后无需重启即可生效（快捷键除外）。
//!
//! 翻译引擎可同时配置多个（`engines`），用 `engine` 指定主用、`fallback` 指定失败后的回退顺序，
//! 以后的设置界面只需要读写这几个字段。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 全局快捷键，如 `Alt+T`（启动时读取，修改后需重启）
    pub hotkey: String,
    /// 目标语言，如 `zh-CN`；首次启动时取系统语言
    pub target_lang: String,
    /// 界面语言，如 `zh-CN`、`en`；留空 = 跟随系统（见 `i18n`）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ui_lang: String,
    /// 翻译模式：`ocr`（第一版只支持这个）/ `vision`（预留）
    pub mode: String,
    /// OCR 语言标签，如 `en-US`；`auto` 表示自动挑选
    pub ocr_lang: String,
    /// 新翻译框是否默认鼠标穿透；框内工具条只改变单个实例
    pub frame_click_through: bool,
    /// 所有翻译框的显示圆角（CSS 像素），0 表示直角
    pub frame_corner_radius: u8,
    /// 主用引擎 id（`engines` 里的键）
    pub engine: String,
    /// 主引擎失败后依次尝试的引擎 id
    pub fallback: Vec<String>,
    /// 已启用的 AI 供应商调用顺序（切回机器翻译时保留该顺序与启用状态，不丢失）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ai_chain: Vec<String>,
    /// 各引擎参数，键为引擎 id；未出现的引擎按默认参数使用
    pub engines: BTreeMap<String, ProviderConfig>,
    /// 全局代理，所有引擎默认都走它：留空 = 直连；`system` = 跟随系统环境变量；其他 = 代理地址。
    /// 单个引擎可在自己的 `proxy` 里另行指定（见 `ProviderConfig::proxy`）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub proxy: String,
    /// 译文写入本地库（`~/.translate-anything/translations.db`），相同原文下次直接查库不再请求引擎
    pub persist_cache: bool,
    /// 译文库所在文件夹；留空 = 数据目录。请在设置窗口里修改（会连同文件一起搬过去）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub store_dir: String,
    /// 使用内置的界面常用词术语表（如 New → 新建），避免引擎翻单个词时没有上下文而翻错
    pub builtin_glossary: bool,
    /// 自定义术语表（全局）：原文 → 译文，译文与原文相同 = 保留原文不翻。
    /// 整条命中时直接采用（优先于过滤、译文库和引擎）；句中出现时提示给 AI 翻译
    pub glossary: BTreeMap<String, String>,
    /// 按软件的自定义术语，键为应用标识（程序文件名小写，见 `normalize_app_id`），覆盖同名的全局术语
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub glossary_apps: BTreeMap<String, BTreeMap<String, String>>,
    /// 翻译过滤规则（全局 + 按应用）
    pub filter: FilterConfig,
    /// 沉浸模式：直接改写目标应用界面里的文字（目前支持 Electron/Chromium 内核应用）
    pub immersive: ImmersiveConfig,
    /// 检查更新
    pub update: UpdateConfig,
    /// 旧版单引擎写法，仅为兼容；有 `engines` 时忽略
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    /// 启动后及常驻期间每天自动检查一次新版本（只提示，不自动安装）
    pub auto_check: bool,
    /// 用户选择跳过的版本号（如 `0.3.0`），该版本不再提示
    #[serde(skip_serializing_if = "String::is_empty")]
    pub skip_version: String,
    /// 更新通道：空或 `auto` 同时试官方和内置镜像、用最快的；`github` 只用官方；
    /// 其他为镜像前缀（如 `https://gh-proxy.com/`），只用该镜像。见 `update::BUILTIN_MIRRORS`
    #[serde(skip_serializing_if = "String::is_empty")]
    pub channel: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self { auto_check: true, skip_version: String::new(), channel: String::new() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// 显示名称（AI 翻译多供应商实例用；留空时按类型回退默认名）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// 引擎类型（见 translate::CATALOG）；留空时取 `engines` 里的键名。
    /// 同一类型配置多份时用得上，例如两个不同的 openai 兼容服务。
    #[serde(rename = "type", skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// 接口地址；多数引擎可留空用默认地址
    pub base_url: String,
    pub api_key: String,
    /// 微软翻译官方 API 的资源区域，如 `eastasia`
    pub region: String,
    /// 仅 openai 使用
    pub model: String,
    /// 本引擎的代理：留空 / 不填 = 跟随全局 `Config::proxy`；`direct` = 直连；
    /// `system` = 跟随系统环境变量；其他 = 代理地址。
    /// 旧版里留空表示直连，而全局代理默认也是直连，所以旧配置行为不变、无需迁移
    pub proxy: Option<String>,
    /// 最大并发请求数（留空按引擎类型默认：AI 引擎 2，传统引擎 4）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u8>,
    /// 设置界面显示的图标（AI 翻译供应商用）：留空 = 自动（按接口地址匹配内置 logo，匹配不到取网站图标）；
    /// `preset:<id>` = 指定内置 logo；`none` = 通用图标；`data:image/...` = 用户上传的图片（已缩到 64px）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
}

/// 翻译过滤规则集（不翻译正则 / 强制翻译）。要保留原文的词写在术语表里
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterRuleSet {
    /// 旧版"不翻译词表"，读取时迁入术语表（见 `Config::migrate`），不再写回
    #[serde(rename = "keep_terms", skip_serializing)]
    pub legacy_keep_terms: Vec<String>,
    pub keep_patterns: Vec<String>,
    pub force_translate: Vec<String>,
}

impl FilterRuleSet {
    fn extend_from(&mut self, other: &FilterRuleSet) {
        self.keep_patterns.extend(other.keep_patterns.iter().cloned());
        self.force_translate.extend(other.force_translate.iter().cloned());
    }

    pub fn is_empty(&self) -> bool {
        self.legacy_keep_terms.is_empty() && self.keep_patterns.is_empty() && self.force_translate.is_empty()
    }
}

/// 翻译过滤总配置（全局规则 + 按应用覆盖）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterConfig {
    /// 旧版"不翻译词表"，同 `FilterRuleSet::legacy_keep_terms`
    #[serde(rename = "keep_terms", skip_serializing)]
    pub legacy_keep_terms: Vec<String>,
    pub keep_patterns: Vec<String>,
    pub force_translate: Vec<String>,
    pub apps: BTreeMap<String, FilterRuleSet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImmersiveConfig {
    /// 总开关
    pub enabled: bool,
    /// 在"译文 / 原文"之间切换的全局快捷键（启动时读取）
    pub hotkey: String,
    /// 要接管的应用；应用需要带 `--remote-debugging-port=<port>` 启动
    pub targets: Vec<ImmersiveTarget>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImmersiveTarget {
    /// 显示名
    pub name: String,
    /// 是否接管这个应用（单独开关）
    pub enabled: bool,
    /// 程序完整路径。用于身份校验（调试端口背后必须是这个程序才接管）和"以沉浸模式启动"
    pub exe: String,
    /// 启动参数（不含调试端口）。"以沉浸模式启动"时使用；应用在运行时会自动从它的命令行读取并记下
    pub args: String,
    /// 远程调试端口（只监听本机 127.0.0.1），各应用不能重复
    pub port: u16,
    /// 只处理地址以这些前缀开头的页面；留空 = 该应用的所有页面
    pub url_prefixes: Vec<String>,
    /// 额外不翻译的元素（CSS 选择器），如聊天正文、代码区
    pub skip_selectors: Vec<String>,
    /// 用户直接打开应用（没带调试端口）时，自动重启成沉浸模式
    pub auto_relaunch: bool,
    /// 该应用的过滤规则（与全局过滤规则合并生效）
    pub filter: FilterRuleSet,
}

impl Default for ImmersiveTarget {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            exe: String::new(),
            args: String::new(),
            port: 0,
            url_prefixes: Vec::new(),
            skip_selectors: Vec::new(),
            auto_relaunch: true,
            filter: FilterRuleSet::default(),
        }
    }
}

impl Default for ImmersiveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: "Alt+Shift+T".into(),
            // 默认不预置任何应用，由用户在设置窗口里检测或手动添加
            targets: Vec::new(),
        }
    }
}

impl ProviderConfig {
    pub fn kind_or(&self, id: &str) -> String {
        let k = self.kind.trim();
        if k.is_empty() { id.to_string() } else { k.to_string() }
    }

    /// 实际使用的并发上限（AI 引擎默认 2，传统引擎默认 4）
    pub fn effective_concurrency(&self, kind: &str) -> usize {
        self.concurrency
            .map(|c| c.clamp(1, 16) as usize)
            .unwrap_or(if matches!(kind, "openai" | "anthropic" | "gemini") { 2 } else { 4 })
    }
}

/// 统一规范化应用标识：取路径最后一段（如可执行文件名）并转小写
pub fn normalize_app_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(trimmed)
        .trim()
        .to_lowercase()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Alt+T".into(),
            target_lang: crate::i18n::default_target_lang(),
            ui_lang: String::new(),
            mode: "ocr".into(),
            ocr_lang: "auto".into(),
            frame_click_through: true,
            frame_corner_radius: 0,
            engine: "edge".into(),
            fallback: Vec::new(),
            ai_chain: Vec::new(),
            engines: BTreeMap::new(),
            proxy: String::new(),
            persist_cache: true,
            store_dir: String::new(),
            builtin_glossary: true,
            glossary: BTreeMap::new(),
            glossary_apps: BTreeMap::new(),
            filter: FilterConfig::default(),
            immersive: ImmersiveConfig::default(),
            update: UpdateConfig::default(),
            provider: None,
        }
    }
}

impl Config {
    /// 某个引擎实际使用的参数：`proxy` 已按"引擎自己的 → 全局"解析好，
    /// 结果为空或 `direct` 表示直连
    pub fn provider(&self, id: &str) -> ProviderConfig {
        self.resolve_proxy(self.engines.get(id).cloned().unwrap_or_default())
    }

    fn resolve_proxy(&self, mut p: ProviderConfig) -> ProviderConfig {
        let own = p.proxy.as_deref().map(str::trim).unwrap_or("");
        p.proxy = Some(if own.is_empty() { self.proxy.trim() } else { own }.to_string());
        p
    }

    /// 按尝试顺序返回（引擎 id，参数），已去重；参数里的代理已解析好
    pub fn engine_chain(&self) -> Vec<(String, ProviderConfig)> {
        // 兼容旧版：只写了 provider
        if self.engines.is_empty() {
            if let Some(p) = &self.provider {
                let id = p.kind_or("openai");
                return vec![(id, self.resolve_proxy(p.clone()))];
            }
        }
        let mut ids: Vec<&str> = vec![self.engine.trim()];
        ids.extend(self.fallback.iter().map(|s| s.trim()));
        let mut seen = Vec::new();
        let mut chain = Vec::new();
        for id in ids.into_iter().filter(|s| !s.is_empty()) {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            chain.push((id.to_string(), self.provider(id)));
        }
        if chain.is_empty() {
            chain.push(("edge".into(), self.provider("edge")));
        }
        chain
    }

    /// 合并全局与指定应用生效的过滤规则
    pub fn effective_filter(&self, app_id: Option<&str>) -> FilterRuleSet {
        let mut out = FilterRuleSet {
            legacy_keep_terms: Vec::new(),
            keep_patterns: self.filter.keep_patterns.clone(),
            force_translate: self.filter.force_translate.clone(),
        };
        let norm = app_id.map(normalize_app_id).unwrap_or_default();
        if !norm.is_empty() {
            for (k, rules) in &self.filter.apps {
                if normalize_app_id(k) == norm {
                    out.extend_from(rules);
                }
            }
            for target in &self.immersive.targets {
                let exe_id = normalize_app_id(&target.exe);
                let name_id = normalize_app_id(&target.name);
                if (!exe_id.is_empty() && exe_id == norm) || (!name_id.is_empty() && name_id == norm) {
                    out.extend_from(&target.filter);
                }
            }
        }
        out
    }

    /// 某个应用生效的自定义术语：全局 + 该应用的（后者覆盖前者），键已去掉首尾空白
    pub fn user_glossary(&self, app_id: &str) -> BTreeMap<String, String> {
        let norm = normalize_app_id(app_id);
        let mut out = BTreeMap::new();
        let apps = self.glossary_apps.iter().filter(|(k, _)| !norm.is_empty() && normalize_app_id(k) == norm);
        for (k, v) in std::iter::once(&self.glossary).chain(apps.map(|(_, m)| m)).flatten() {
            let k = k.trim();
            if !k.is_empty() && !v.trim().is_empty() {
                out.insert(k.to_string(), v.trim().to_string());
            }
        }
        out
    }

    /// 旧版"不翻译词表"（过滤规则里的 `keep_terms`，全局 / 按应用 / 沉浸模式应用各一份）
    /// 迁入术语表，写成"原文 → 原文"。原来它比术语表先生效，所以同名术语改为保留原文。
    /// 返回是否有改动
    pub fn migrate(&mut self) -> bool {
        let mut changed = false;
        for term in std::mem::take(&mut self.filter.legacy_keep_terms) {
            let term = term.trim().to_string();
            if !term.is_empty() {
                self.glossary.insert(term.clone(), term);
            }
            changed = true;
        }
        let mut moved: Vec<(String, Vec<String>)> = Vec::new();
        for (app, rules) in &mut self.filter.apps {
            moved.push((normalize_app_id(app), std::mem::take(&mut rules.legacy_keep_terms)));
        }
        for t in &mut self.immersive.targets {
            let raw = if t.exe.trim().is_empty() { &t.name } else { &t.exe };
            moved.push((normalize_app_id(raw), std::mem::take(&mut t.filter.legacy_keep_terms)));
        }
        for (app, terms) in moved.into_iter().filter(|(_, terms)| !terms.is_empty()) {
            changed = true;
            let map = if app.is_empty() { &mut self.glossary } else { self.glossary_apps.entry(app).or_default() };
            for term in terms.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
                map.insert(term.to_string(), term.to_string());
            }
        }
        if changed {
            self.filter.apps.retain(|_, r| !r.is_empty());
        }
        changed
    }
}

/// 用户数据目录名（配置、译文库、沉浸模式启动参数备份都放这里）
const DATA_DIR: &str = ".translate-anything";
/// 改名前（translate-filter 时期）的数据目录名，启动时自动迁移
const LEGACY_DATA_DIR: &str = ".translate-filter";

/// 便携模式标记文件名：程序同目录下有其中任一文件即为便携模式（带 `.txt` 是为了照顾在资源管理器里新建文本文件的用户）
const PORTABLE_MARKERS: [&str; 2] = ["portable", "portable.txt"];
/// 便携模式下数据目录相对程序所在目录的名字
const PORTABLE_DATA_DIR: &str = "data";

/// 检查程序旁的便携标记，不依赖数据目录是否可写。
pub fn has_portable_marker(exe_dir: &std::path::Path) -> std::io::Result<bool> {
    for marker in PORTABLE_MARKERS {
        match std::fs::metadata(exe_dir.join(marker)) {
            Ok(metadata) if metadata.is_file() => return Ok(true),
            Ok(_) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

/// 便携模式的数据目录：程序同目录有标记文件时为 `<程序目录>\data`，否则 `None`
///
/// 目录建不出来（如被放进了只读位置）也返回 `None`，回退到用户目录并打日志，免得程序直接用不了。
pub fn portable_dir() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
        if !has_portable_marker(&exe_dir).unwrap_or(false) {
            return None;
        }
        let dir = exe_dir.join(PORTABLE_DATA_DIR);
        match std::fs::create_dir_all(&dir) {
            Ok(()) => Some(dir),
            Err(e) => {
                eprintln!("便携模式：无法创建数据目录 {}，改用用户目录：{e}", dir.display());
                None
            }
        }
    })
    .clone()
}

/// 内嵌浏览器（WebView2）缓存目录：便携模式下放进数据目录，否则 `None`（用 Tauri 默认的 `%LOCALAPPDATA%\<identifier>`）
pub fn webview_data_dir() -> Option<PathBuf> {
    portable_dir().map(|d| d.join("webview"))
}

/// 给建窗口的 builder 用：便携模式下把 WebView2 缓存指到数据目录，所有窗口都要调，保证同一进程用同一个缓存目录
pub trait PortableWebview {
    fn portable_data(self) -> Self;
}

impl<'a, R: tauri::Runtime, M: tauri::Manager<R>> PortableWebview for tauri::WebviewWindowBuilder<'a, R, M> {
    fn portable_data(self) -> Self {
        match webview_data_dir() {
            Some(d) => self.data_directory(d),
            None => self,
        }
    }
}

/// 用户数据目录：便携模式为 `<程序目录>\data`（见 [`portable_dir`]），否则为 `~/.translate-anything`
///
/// 非便携模式首次调用时：新目录不存在而旧目录 `~/.translate-filter` 存在，就把旧目录整个改名过来
/// （配置、译文库、术语表等原样保留）。改名失败（如旧版程序还开着、文件被占用）时继续用旧目录，
/// 下次启动再试。
pub fn data_dir() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        if let Some(dir) = portable_dir() {
            return Some(dir);
        }
        let home = dirs::home_dir()?;
        let dir = home.join(DATA_DIR);
        let legacy = home.join(LEGACY_DATA_DIR);
        if !dir.exists() && legacy.is_dir() {
            match std::fs::rename(&legacy, &dir) {
                Ok(()) => println!("已将旧数据目录 {} 迁移到 {}", legacy.display(), dir.display()),
                Err(e) => {
                    eprintln!("迁移旧数据目录 {} 失败，本次继续使用旧目录：{e}", legacy.display());
                    return Some(legacy);
                }
            }
        }
        Some(dir)
    })
    .clone()
}

pub fn config_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("config.json"))
}

/// 读取配置；文件不存在时返回默认配置（默认引擎免 key，开箱即用）
pub fn load() -> Result<Config, String> {
    let path = config_path().ok_or_else(|| t!("err.config.no_home"))?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(t!("err.config.read", path = path.display(), e = e)),
    };
    let mut cfg: Config = serde_json::from_str(&text).map_err(|e| t!("err.config.parse", path = path.display(), e = e))?;
    if cfg.migrate() {
        let _ = save(&cfg);
    }
    Ok(cfg)
}

/// 检查配置是否可以保存（端口重复等）
pub fn validate(cfg: &Config) -> Result<(), String> {
    let mut ports = std::collections::HashSet::new();
    for t in &cfg.immersive.targets {
        if t.name.trim().is_empty() {
            return Err(t!("err.config.app_name_empty"));
        }
        if t.port < 1024 {
            return Err(t!("err.config.port_range", name = t.name));
        }
        if !ports.insert(t.port) {
            return Err(t!("err.config.port_dup", port = t.port));
        }
    }
    if cfg.target_lang.trim().is_empty() {
        return Err(t!("err.config.target_lang_empty"));
    }
    let check_patterns = |patterns: &[String]| -> Result<(), String> {
        for p in patterns {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            if let Err(e) = regex::Regex::new(p) {
                return Err(t!("err.config.bad_regex", pattern = p, e = e));
            }
        }
        Ok(())
    };
    check_patterns(&cfg.filter.keep_patterns)?;
    for rules in cfg.filter.apps.values() {
        check_patterns(&rules.keep_patterns)?;
    }
    for t in &cfg.immersive.targets {
        check_patterns(&t.filter.keep_patterns)?;
    }
    Ok(())
}

/// 保存配置：先写临时文件再替换，写到一半出错不会损坏原文件
pub fn save(cfg: &Config) -> Result<(), String> {
    validate(cfg)?;
    let path = config_path().ok_or_else(|| t!("err.config.no_home"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| t!("err.config.mkdir", e = e))?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| t!("err.config.write", e = e))?;
    std::fs::rename(&tmp, &path).map_err(|e| t!("err.config.replace", e = e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 引擎链与回退() {
        let cfg: Config = serde_json::from_str(
            r#"{"engine":"microsoft","fallback":["edge","microsoft","google_free"],
                "engines":{"microsoft":{"api_key":"k","region":"eastasia"},"my-llm":{"type":"openai"}}}"#,
        )
        .unwrap();
        let chain: Vec<_> = cfg.engine_chain().into_iter().map(|(id, _)| id).collect();
        assert_eq!(chain, ["microsoft", "edge", "google_free"]);
        assert_eq!(cfg.engines["my-llm"].kind_or("my-llm"), "openai");
    }

    #[test]
    fn 兼容旧版provider写法() {
        let cfg: Config = serde_json::from_str(r#"{"provider":{"type":"openai","model":"m"}}"#).unwrap();
        let chain = cfg.engine_chain();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].0, "openai");
    }

    #[test]
    fn 校验端口重复() {
        let mut cfg = Config::default();
        assert!(cfg.immersive.targets.is_empty(), "默认不预置应用");
        let t = ImmersiveTarget { name: "a".into(), port: 39222, ..Default::default() };
        cfg.immersive.targets.push(t.clone());
        assert!(validate(&cfg).is_ok());
        let mut t = t;
        t.name = "另一个".into();
        cfg.immersive.targets.push(t);
        assert_eq!(validate(&cfg).unwrap_err(), t!("err.config.port_dup", port = 39222));
    }

    #[test]
    fn 旧配置补齐新字段() {
        let cfg: Config = serde_json::from_str(r#"{"immersive":{"targets":[{"name":"a","port":40000}]}}"#).unwrap();
        assert!(cfg.immersive.targets[0].enabled, "缺省为启用");
        assert!(cfg.immersive.enabled);
        assert_eq!(cfg.frame_corner_radius, 0);
    }

    #[test]
    fn 代理跟随全局与单独指定() {
        let cfg: Config = serde_json::from_str(
            r#"{"proxy":"http://127.0.0.1:7897",
                "engines":{"edge":{"proxy":"direct"},"google_free":{"proxy":"http://127.0.0.1:1080"},"bing":{"proxy":""}}}"#,
        )
        .unwrap();
        let proxy = |id: &str| cfg.provider(id).proxy.unwrap();
        assert_eq!(proxy("edge"), "direct");
        assert_eq!(proxy("google_free"), "http://127.0.0.1:1080");
        assert_eq!(proxy("bing"), "http://127.0.0.1:7897", "留空跟随全局");
        assert_eq!(proxy("deepl"), "http://127.0.0.1:7897", "没配置过的引擎也跟随全局");
        // 旧配置没有全局代理：留空仍是直连
        let old: Config = serde_json::from_str(r#"{"engines":{"bing":{"proxy":null}}}"#).unwrap();
        assert_eq!(old.provider("bing").proxy.unwrap(), "");
    }

    #[test]
    fn 空配置默认edge() {
        let chain = Config::default().engine_chain();
        assert_eq!(chain[0].0, "edge");
    }

    #[test]
    fn 供应商显示名与ai链字段兼容() {
        let cfg: Config = serde_json::from_str(
            r#"{"engine":"ai_1","fallback":["edge"],"ai_chain":["ai_1","ai_2"],
                "engines":{"ai_1":{"name":"DeepSeek","type":"openai"},"ai_2":{"name":"  ","type":"anthropic"}}}"#,
        )
        .unwrap();
        assert_eq!(cfg.ai_chain, ["ai_1", "ai_2"]);
        assert_eq!(cfg.engines["ai_1"].name, "DeepSeek");
        assert_eq!(cfg.engines["ai_2"].kind, "anthropic");
    }

    #[test]
    fn 过滤规则合并与正则校验() {
        let cfg: Config = serde_json::from_str(
            r#"{
                "filter": {
                    "keep_terms": ["Claude"],
                    "keep_patterns": ["^v\\d+$"],
                    "force_translate": ["Step 1"],
                    "apps": {
                        "Code.exe": { "keep_terms": ["Terminal"], "keep_patterns": ["^--"] }
                    }
                },
                "immersive": {
                    "targets": [
                        { "name": "VS Code", "exe": "C:\\Program Files\\Code.exe", "port": 39222, "filter": { "force_translate": ["Run"] } }
                    ]
                },
                "engines": { "openai": { "concurrency": 3 }, "edge": {} }
            }"#,
        )
        .unwrap();
        assert!(validate(&cfg).is_ok());
        assert_eq!(normalize_app_id(r"C:\Program Files\Code.exe"), "code.exe");
        let rules = cfg.effective_filter(Some("CODE.EXE"));
        assert_eq!(rules.keep_patterns, ["^v\\d+$", "^--"]);
        assert_eq!(rules.force_translate, ["Step 1", "Run"]);
        assert_eq!(cfg.provider("openai").effective_concurrency("openai"), 3);
        assert_eq!(cfg.provider("default_ai").effective_concurrency("openai"), 2);
        assert_eq!(cfg.provider("default_anth").effective_concurrency("anthropic"), 2);
        assert_eq!(cfg.provider("edge").effective_concurrency("edge"), 4);

        let mut bad = cfg.clone();
        bad.filter.keep_patterns.push("[unclosed".into());
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn 旧版不翻译词表迁入术语表() {
        let mut cfg: Config = serde_json::from_str(
            r#"{
                "glossary": { "Claude": "克劳德", "New": "新建" },
                "filter": {
                    "keep_terms": ["Claude", " "],
                    "apps": {
                        "Code.exe": { "keep_terms": ["Terminal"] },
                        "word.exe": { "keep_terms": ["Ribbon"], "keep_patterns": ["^--"] }
                    }
                },
                "immersive": {
                    "targets": [
                        { "name": "Claude", "exe": "C:\\x\\claude.exe", "port": 39222, "filter": { "keep_terms": ["Opus"], "force_translate": ["Run"] } }
                    ]
                }
            }"#,
        )
        .unwrap();
        assert!(cfg.migrate());
        assert_eq!(cfg.glossary["Claude"], "Claude", "原来不翻译词表先生效，同名术语改为保留原文");
        assert_eq!(cfg.glossary["New"], "新建");
        assert!(!cfg.glossary.contains_key(""));
        assert_eq!(cfg.glossary_apps["code.exe"]["Terminal"], "Terminal");
        assert_eq!(cfg.glossary_apps["claude.exe"]["Opus"], "Opus");
        assert!(!cfg.filter.apps.contains_key("Code.exe"), "迁完变空的应用规则删掉");
        assert_eq!(cfg.filter.apps["word.exe"].keep_patterns, ["^--"]);
        assert_eq!(cfg.immersive.targets[0].filter.force_translate, ["Run"]);
        assert!(!cfg.migrate(), "迁移只做一次");

        let saved = serde_json::to_string(&cfg).unwrap();
        assert!(!saved.contains("keep_terms"), "旧字段不再写回");

        let g = cfg.user_glossary("C:\\Program Files\\Code.exe");
        assert_eq!(g["Terminal"], "Terminal");
        assert_eq!(g["New"], "新建");
        assert!(!cfg.user_glossary("other.exe").contains_key("Terminal"), "按软件的术语只对该软件生效");
    }
}
