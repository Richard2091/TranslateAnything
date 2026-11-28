//! 翻译过滤：统一判断一段文本该不该翻译（悬浮窗与沉浸模式共用）。
//!
//! 按分层优先级判断：
//! 1. 用户规则（强制翻译 > 不翻译词表 > 不翻译正则，全局 + 按应用）
//! 2. 内置确定性模式（长度、无字母、URL/邮箱、路径/文件名、版本号/日期、快捷键、代码标识符、型号+版本）
//! 2.5 代码块与结构化文本（JSON/XML/SQL/Diff、命令行、日志与报错堆栈、符号密度/代码关键词打分）
//! 3. 语言识别（短词只看字符集，长文本且高置信度再用 `whatlang`）
//! 4. 拿不准时默认放行翻译，后续由译文库记住引擎/大模型判定的"保留原文"。

use crate::config::{Config, FilterRuleSet, normalize_app_id};
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_TEXT_CHARS: usize = 5000;
const EVENT_CAP: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Overlay,
    Immersive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    ForceTranslate,
    UserKeepTerm,
    UserKeepPattern,
    TooShort,
    TooLong,
    NoLetters,
    UrlOrEmail,
    FilePath,
    VersionOrDate,
    Shortcut,
    Identifier,
    ModelVersion,
    CodeOrStructured,
    AlreadyTargetLang,
    EngineUnchanged,
    AiKeep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Translate,
    ForceTranslate,
    Keep(Reason),
}

pub struct FilterContext<'a> {
    pub target_lang: &'a str,
    pub app_id: &'a str,
    pub app_name: &'a str,
    pub mode: Mode,
}

#[derive(Debug, Clone, Serialize)]
pub struct FilterEvent {
    pub text: String,
    pub decision: String,
    pub reason: Reason,
    pub app_id: String,
    pub app_name: String,
    pub mode: Mode,
    pub at: i64,
}

static EVENTS: OnceLock<Mutex<VecDeque<FilterEvent>>> = OnceLock::new();
static REGEX_CACHE: OnceLock<Mutex<HashMap<String, Option<Regex>>>> = OnceLock::new();

fn events_buf() -> &'static Mutex<VecDeque<FilterEvent>> {
    EVENTS.get_or_init(|| Mutex::new(VecDeque::with_capacity(EVENT_CAP)))
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn preview(text: &str) -> String {
    let mut s: String = text.trim().chars().take(300).collect();
    if text.trim().chars().count() > 300 {
        s.push('…');
    }
    s
}

/// 记录一条过滤判定（最近 50 条内相同文本/应用/原因只更新时间戳，避免刷屏）
pub fn record_event(text: &str, decision: &str, reason: Reason, ctx: &FilterContext<'_>) {
    let text_prev = preview(text);
    if text_prev.is_empty() {
        return;
    }
    let app_id = normalize_app_id(ctx.app_id);
    let at = now_secs();
    let mut buf = events_buf().lock().unwrap();
    if let Some(pos) = buf
        .iter()
        .rev()
        .take(50)
        .position(|e| e.text == text_prev && e.app_id == app_id && e.reason == reason && e.decision == decision)
    {
        let idx = buf.len() - 1 - pos;
        if let Some(item) = buf.remove(idx) {
            let mut item = item;
            item.at = at;
            buf.push_back(item);
            return;
        }
    }
    if buf.len() >= EVENT_CAP {
        buf.pop_front();
    }
    buf.push_back(FilterEvent {
        text: text_prev,
        decision: decision.to_string(),
        reason,
        app_id,
        app_name: ctx.app_name.to_string(),
        mode: ctx.mode,
        at,
    });
}

/// 查询最近的过滤事件（按时间倒序）
pub fn recent_events(limit: usize, app_id: Option<&str>, only_kept: bool) -> Vec<FilterEvent> {
    let norm = app_id.map(normalize_app_id);
    let buf = events_buf().lock().unwrap();
    buf.iter()
        .rev()
        .filter(|e| !only_kept || e.decision == "keep")
        .filter(|e| match norm.as_deref() {
            None => true,
            Some("") => e.app_id.is_empty(),
            Some(id) => e.app_id == id,
        })
        .take(limit.clamp(1, EVENT_CAP))
        .cloned()
        .collect()
}

#[cfg(test)]
pub fn clear_events() {
    events_buf().lock().unwrap().clear();
}

/// 统一过滤入口
pub fn classify(cfg: &Config, text: &str, ctx: &FilterContext<'_>) -> Decision {
    let core = text.trim();
    if core.is_empty() {
        return Decision::Keep(Reason::TooShort);
    }

    // 第 1 层：用户规则（全局 + 按应用）
    let rules = cfg.effective_filter(Some(ctx.app_id));
    if matches_term(&rules.force_translate, core) {
        return Decision::ForceTranslate;
    }
    if matches_term(&rules.keep_terms, core) {
        return Decision::Keep(Reason::UserKeepTerm);
    }
    if matches_pattern(&rules.keep_patterns, core) {
        return Decision::Keep(Reason::UserKeepPattern);
    }

    // 第 2 层：内置确定性模式
    let char_count = core.chars().count();
    if char_count < 2 {
        return Decision::Keep(Reason::TooShort);
    }
    if char_count > MAX_TEXT_CHARS {
        return Decision::Keep(Reason::TooLong);
    }
    let letters = core.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 2 {
        return Decision::Keep(Reason::NoLetters);
    }
    if is_url_or_email(core) {
        return Decision::Keep(Reason::UrlOrEmail);
    }
    if is_file_path(core) {
        return Decision::Keep(Reason::FilePath);
    }
    if is_version_or_date(core) {
        return Decision::Keep(Reason::VersionOrDate);
    }
    if is_shortcut(core) {
        return Decision::Keep(Reason::Shortcut);
    }
    if is_identifier(core) {
        return Decision::Keep(Reason::Identifier);
    }
    if is_model_version(core) {
        return Decision::Keep(Reason::ModelVersion);
    }

    // 第 2.5 层：代码块与结构化文本
    if is_code_or_structured(core) {
        return Decision::Keep(Reason::CodeOrStructured);
    }

    // 第 3 层：语言识别（先文字系统硬判，再对足够长的文本用 whatlang 统计）
    if is_already_target_lang(core, ctx.target_lang) {
        return Decision::Keep(Reason::AlreadyTargetLang);
    }

    Decision::Translate
}

/// 在有效规则基础上判断术语匹配（忽略首尾空白；纯 ASCII 忽略大小写）
fn matches_term(list: &[String], core: &str) -> bool {
    list.iter().any(|t| {
        let t = t.trim();
        !t.is_empty() && (t == core || (t.is_ascii() && core.is_ascii() && t.eq_ignore_ascii_case(core)))
    })
}

fn matches_pattern(patterns: &[String], core: &str) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let cache = REGEX_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap();
    if map.len() > 1000 {
        map.clear();
    }
    for p in patterns {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        let compiled = map.entry(p.to_string()).or_insert_with(|| Regex::new(p).ok());
        if compiled.as_ref().is_some_and(|re| re.is_match(core)) {
            return true;
        }
    }
    false
}

/// 供上层获取合并后的过滤规则，提取需要传给大模型的保留词
pub fn active_keep_terms(cfg: &Config, app_id: &str) -> FilterRuleSet {
    cfg.effective_filter(Some(app_id))
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(is_cjk_char)
}

pub fn is_cjk_char(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}')
}

fn is_hangul(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7AF}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}')
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}')
}

fn is_url_or_email(s: &str) -> bool {
    if s.contains(char::is_whitespace) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ftp://")
        || lower.starts_with("ws://")
        || lower.starts_with("wss://")
        || lower.starts_with("mailto:")
        || lower.starts_with("www.")
    {
        return true;
    }
    static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
    let re = EMAIL_RE.get_or_init(|| Regex::new(r"^[^\s@]+@[^\s@]+\.[A-Za-z]{2,}$").unwrap());
    re.is_match(s)
}

fn is_file_path(s: &str) -> bool {
    if s.contains(char::is_whitespace) {
        return false;
    }
    static PATH_RE: OnceLock<Regex> = OnceLock::new();
    let re = PATH_RE.get_or_init(|| {
        Regex::new(r"^(?:[A-Za-z]:[\\/]|\\\\[^\\]+\\|~/|\.\.?/|/(?:[^/\s]+/)*[^/\s]+)$").unwrap()
    });
    if re.is_match(s) {
        return true;
    }
    // 带扩展名的单个文件名（含中英文文件名如 "海鸥3.0.md"）
    static EXT_RE: OnceLock<Regex> = OnceLock::new();
    let ext = EXT_RE.get_or_init(|| {
        Regex::new(r"(?i)\.(?:rs|ts|tsx|js|jsx|mjs|cjs|py|go|java|kt|cs|cpp|cc|c|h|hpp|rb|php|swift|sh|ps1|bat|cmd|json|yaml|yml|toml|xml|html|css|scss|sql|md|txt|log|csv|ini|conf|cfg|env|lock|exe|dll|msi|dmg|app|deb|rpm|zip|tar|gz|7z|png|jpe?g|gif|webp|svg|ico|pdf|docx?|xlsx?|pptx?|mp3|mp4|wav|wasm)$").unwrap()
    });
    ext.is_match(s)
}

fn is_version_or_date(s: &str) -> bool {
    if s.contains(char::is_whitespace) {
        return false;
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(concat!(
            r"^(?:",
            r"v?\d+(?:\.\d+){1,3}(?:-[A-Za-z0-9.+-]+)?|", // semver / v1.2.3
            r"(?:0x)?[0-9a-fA-F]{8,}|",                      // hex / hash
            r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}|", // UUID
            r"\d{4}[-/]\d{1,2}[-/]\d{1,2}(?:[T ]\d{2}:\d{2}(?::\d{2})?(?:Z|[+-]\d{2}:?\d{2})?)?|", // 日期时间
            r"\d{1,2}:\d{2}(?::\d{2})?",                     // 时间
            r")$"
        ))
        .unwrap()
    });
    re.is_match(s)
}

fn is_shortcut(s: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:(?:Ctrl|Alt|Shift|Win|Meta|Cmd|Command|Option|Super|⌘|⌥|⌃|⇧)\s*\+\s*)+(?:[A-Z0-9]|F\d{1,2}|Space|Enter|Esc|Tab|Backspace|Delete|Up|Down|Left|Right|Home|End|PgUp|PgDn|[\[\]\\;',./`\-=])$").unwrap()
    });
    if re.is_match(s) {
        return true;
    }
    // macOS 符号组合键如 ⌘K、⇧⌘P
    let chars: Vec<char> = s.chars().collect();
    (2..=4).contains(&chars.len())
        && chars[..chars.len() - 1].iter().all(|c| matches!(c, '⌘' | '⌥' | '⌃' | '⇧'))
        && chars.last().is_some_and(|c| c.is_ascii_alphanumeric())
}

fn is_identifier(s: &str) -> bool {
    if s.contains(char::is_whitespace) {
        return false;
    }
    if has_cjk(s) {
        // 中日文整句没有空格，只拦明显的路径分隔/括号符号
        return s.chars().any(|c| matches!(c, '/' | '\\' | '@' | '#' | '=' | '<' | '>' | '{' | '}' | '[' | ']' | '$' | '|'));
    }
    // 含明显代码/路径符号（不含句末标点）
    if s.chars().any(|c| matches!(c, '/' | '\\' | '_' | '@' | '#' | '=' | '<' | '>' | '{' | '}' | '[' | ']' | '$' | '|')) {
        return true;
    }
    // --cli-flag / foo() / camelCase / foo.bar（排除缩写句号如 "etc."）
    if s.starts_with("--") || s.ends_with("()") {
        return true;
    }
    if s.contains('.') && !s.ends_with('.') {
        return true;
    }
    if s.contains(':') && !s.ends_with(':') {
        return true;
    }
    // camelCase 或含数字的单个技术词（如 "GPT-4o", "utf8", "x64", "v2"）
    let has_camel = s.as_bytes().windows(2).any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase());
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    has_camel || has_digit
}

/// 窄化的"产品/型号 + 版本号"规则（如 `Opus 5.5`、`Node 22`、`Claude 3.5`），放行 `Step 1`、`2 items`、`Last 7 days` 等
fn is_model_version(s: &str) -> bool {
    if has_cjk(s) {
        return false;
    }
    let words: Vec<&str> = s.split_whitespace().collect();
    if !(2..=3).contains(&words.len()) {
        return false;
    }
    const UI_WORDS: &[&str] = &[
        "step", "page", "item", "items", "part", "line", "tab", "row", "col", "column",
        "day", "days", "week", "weeks", "month", "months", "year", "years", "hour", "hours",
        "min", "mins", "sec", "secs", "level", "stage", "phase", "chapter", "section",
        "version", "last", "next", "prev", "top", "first", "save", "retry", "wait",
        "open", "show", "hide", "add", "remove", "delete", "select", "found", "showing",
        "total", "max", "only", "about", "over", "under", "every", "after", "before",
        "within", "at", "in", "on", "by", "to", "for", "of", "from", "up", "try",
    ];
    let first_lower = words[0].trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_ascii_lowercase();
    if UI_WORDS.contains(&first_lower.as_str()) {
        return false;
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^[A-Z][A-Za-z0-9.+-]{1,18}(?:\s+[A-Z][A-Za-z0-9.+-]{1,12})?\s+v?\d+(?:\.\d+)*(?:[A-Za-z0-9.-]{0,6})?$").unwrap()
    });
    re.is_match(s)
}

/// 识别代码块、结构化数据、命令行、日志与报错堆栈
fn is_code_or_structured(s: &str) -> bool {
    // 1. JSON 对象或数组
    if ((s.starts_with('{') && s.ends_with('}')) || (s.starts_with('[') && s.ends_with(']')))
        && (s.contains("\":") || s.contains("':") || s.contains('{'))
    {
        return true;
    }
    // 2. XML / HTML 标签块
    static XML_RE: OnceLock<Regex> = OnceLock::new();
    let xml = XML_RE.get_or_init(|| {
        Regex::new(r"^<[a-zA-Z][\w:-]*(?:\s[^>]*)?(?:/>|>[\s\S]*</[a-zA-Z][\w:-]*>)$").unwrap()
    });
    if xml.is_match(s) {
        return true;
    }
    // 3. Diff 块
    if s.starts_with("diff --git ") || s.starts_with("@@ ") || (s.contains("\n+++ ") && s.contains("\n--- ")) {
        return true;
    }
    // 4. SQL 语句
    static SQL_RE: OnceLock<Regex> = OnceLock::new();
    let sql = SQL_RE.get_or_init(|| {
        Regex::new(r"(?is)^(?:SELECT\s+.+\s+FROM|INSERT\s+INTO|UPDATE\s+\S+\s+SET|DELETE\s+FROM|CREATE\s+TABLE|ALTER\s+TABLE|DROP\s+TABLE)\b").unwrap()
    });
    if sql.is_match(s) {
        return true;
    }
    // 5. 命令行
    if is_cli_command(s) {
        return true;
    }
    // 6. 日志行 / 报错堆栈
    if is_log_or_stacktrace(s) {
        return true;
    }

    // 7. 代码密度打分（避免误伤带括号/冒号的正常散文）
    let mut syntax_score = 0u32;
    for tok in ["=>", "->", "::", "==", "!=", "<=", ">=", "&&", "||", ":=", "+=", "/*", "*/", "//", "();", "{}", "[]"] {
        if s.contains(tok) {
            syntax_score += 2;
        }
    }
    let mut kw_hits = 0u32;
    for line in s.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let first_word = line.split_whitespace().next().unwrap_or("");
        if matches!(
            first_word,
            "fn" | "pub" | "let" | "const" | "var" | "function" | "def" | "import" | "export"
                | "return" | "struct" | "impl" | "enum" | "interface" | "package" | "#include" | "async"
        ) && (line.contains('(') || line.contains('{') || line.contains(';') || line.contains('=') || line.contains(':'))
        {
            kw_hits += 1;
        }
    }
    if kw_hits >= 1 && (syntax_score >= 2 || s.lines().count() >= 2) {
        return true;
    }
    if syntax_score >= 4 {
        return true;
    }

    let lines: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() >= 2 {
        let code_lines = lines
            .iter()
            .filter(|l| {
                l.ends_with([';', '{', '}'])
                    || is_log_or_stacktrace(l)
                    || is_cli_command(l)
                    || ["=>", "->", "::", ":=", "=="].iter().any(|t| l.contains(t))
            })
            .count();
        if code_lines * 2 >= lines.len() {
            return true;
        }
    }

    let code_punct = s
        .chars()
        .filter(|c| matches!(c, '{' | '}' | '[' | ']' | ';' | '=' | '<' | '>' | '|' | '\\' | '^' | '~' | '`'))
        .count();
    let non_ws = s.chars().filter(|c| !c.is_whitespace()).count().max(1);
    code_punct >= 3 && (code_punct as f32 / non_ws as f32) >= 0.18
}

fn is_cli_command(s: &str) -> bool {
    let first = s.lines().next().unwrap_or("").trim();
    if first.starts_with("$ ") || first.starts_with("PS ") || first.starts_with("C:\\>") {
        return true;
    }
    let head = first.split_whitespace().next().unwrap_or("");
    let is_tool = matches!(
        head,
        "git" | "npm" | "pnpm" | "yarn" | "npx" | "cargo" | "rustc" | "docker" | "kubectl"
            | "curl" | "wget" | "ssh" | "scp" | "pip" | "pip3" | "python" | "python3" | "node" | "sudo"
    );
    is_tool && (first.contains(" -") || first.contains(" --") || first.contains('|') || first.contains("&&") || first.contains('/'))
}

fn is_log_or_stacktrace(s: &str) -> bool {
    static LOG_RE: OnceLock<Regex> = OnceLock::new();
    let re = LOG_RE.get_or_init(|| {
        Regex::new(r"(?m)(?:^\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}|^\[(?:INFO|WARN|WARNING|ERROR|DEBUG|TRACE)\]|^\s*at\s+\S+\s*\(.*:\d+|Traceback \(most recent call last\):|^\s*File\s+\x22.*\x22,\s+line\s+\d+|panicked at\s)").unwrap()
    });
    re.is_match(s)
}

/// 第 3 层：判断是否已是目标语言（短文本只看字符集，长文本且高置信度用 whatlang）
fn is_already_target_lang(s: &str, target_lang: &str) -> bool {
    let target = target_lang.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    let mut total = 0usize;
    let mut han = 0usize;
    let mut kana = 0usize;
    let mut hangul = 0usize;
    let mut cyrillic = 0usize;
    let mut arabic = 0usize;
    let mut thai = 0usize;
    let mut greek = 0usize;

    for c in s.chars().filter(|c| c.is_alphabetic()) {
        total += 1;
        if is_kana(c) {
            kana += 1;
        } else if is_hangul(c) {
            hangul += 1;
        } else if is_han(c) {
            han += 1;
        } else if matches!(c, '\u{0400}'..='\u{04FF}') {
            cyrillic += 1;
        } else if matches!(c, '\u{0600}'..='\u{06FF}') {
            arabic += 1;
        } else if matches!(c, '\u{0E00}'..='\u{0E7F}') {
            thai += 1;
        } else if matches!(c, '\u{0370}'..='\u{03FF}') {
            greek += 1;
        }
    }
    if total == 0 {
        return false;
    }

    // 独占文字系统硬判断（任意长度准确）
    match target.as_str() {
        "zh" => return kana == 0 && hangul == 0 && han * 2 >= total,
        "ja" => {
            if kana > 0 && (kana + han) * 2 >= total {
                return true;
            }
        }
        "ko" => return hangul > 0 && hangul * 2 >= total,
        "ar" => return arabic * 2 >= total,
        "th" => return thai * 2 >= total,
        "el" => return greek * 2 >= total,
        "ru" | "uk" if cyrillic * 2 >= total => {}
        _ => {}
    }

    // 短词不用统计模型，防止如 "Open"、"Pro"、"File" 被误判成其他欧洲语言
    if total < 20 && han < 8 {
        return false;
    }

    if let Some(info) = whatlang::detect(s) {
        if info.is_reliable() && info.confidence() >= 0.85 {
            if let Some(detected) = whatlang_primary(info.lang()) {
                return detected == target;
            }
        }
    }
    false
}

fn whatlang_primary(lang: whatlang::Lang) -> Option<&'static str> {
    use whatlang::Lang;
    Some(match lang {
        Lang::Eng => "en",
        Lang::Cmn => "zh",
        Lang::Jpn => "ja",
        Lang::Kor => "ko",
        Lang::Fra => "fr",
        Lang::Deu => "de",
        Lang::Spa => "es",
        Lang::Por => "pt",
        Lang::Ita => "it",
        Lang::Rus => "ru",
        Lang::Ukr => "uk",
        Lang::Ara => "ar",
        Lang::Tha => "th",
        Lang::Ell => "el",
        Lang::Vie => "vi",
        Lang::Nld => "nl",
        Lang::Pol => "pl",
        Lang::Tur => "tr",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(lang: &'a str) -> FilterContext<'a> {
        FilterContext { target_lang: lang, app_id: "claude.exe", app_name: "Claude", mode: Mode::Immersive }
    }

    #[test]
    fn 放行常见含数字界面文案并拦截型号版本() {
        let cfg = Config::default();
        for text in ["Step 1", "2 items", "Save 20%", "Last 7 days", "Retry in 5s", "Page 1 of 3", "Open settings"] {
            assert_eq!(classify(&cfg, text, &ctx("zh-CN")), Decision::Translate, "{text}");
        }
        assert_eq!(classify(&cfg, "Opus 5.5", &ctx("zh-CN")), Decision::Keep(Reason::ModelVersion));
        assert_eq!(classify(&cfg, "Node 22", &ctx("zh-CN")), Decision::Keep(Reason::ModelVersion));
        assert_eq!(classify(&cfg, "v1.2.3-beta", &ctx("zh-CN")), Decision::Keep(Reason::VersionOrDate));
    }

    #[test]
    fn 路径网址快捷键与标识符过滤() {
        let cfg = Config::default();
        assert_eq!(classify(&cfg, "https://example.com/docs", &ctx("zh-CN")), Decision::Keep(Reason::UrlOrEmail));
        assert_eq!(classify(&cfg, "user@example.com", &ctx("zh-CN")), Decision::Keep(Reason::UrlOrEmail));
        assert_eq!(classify(&cfg, r"C:\Users\Richard\test.txt", &ctx("zh-CN")), Decision::Keep(Reason::FilePath));
        assert_eq!(classify(&cfg, "海鸥3.0.md", &ctx("en")), Decision::Keep(Reason::FilePath));
        assert_eq!(classify(&cfg, "Ctrl+Shift+P", &ctx("zh-CN")), Decision::Keep(Reason::Shortcut));
        assert_eq!(classify(&cfg, "camelCaseVar", &ctx("zh-CN")), Decision::Keep(Reason::Identifier));
        assert_eq!(classify(&cfg, "snake_case_id", &ctx("zh-CN")), Decision::Keep(Reason::Identifier));
        assert_eq!(classify(&cfg, "Close", &ctx("zh-CN")), Decision::Translate);
    }

    #[test]
    fn 代码块与结构化文本过滤但放行散文() {
        let cfg = Config::default();
        assert_eq!(classify(&cfg, r#"{"name": "test", "count": 3}"#, &ctx("zh-CN")), Decision::Keep(Reason::CodeOrStructured));
        assert_eq!(classify(&cfg, "SELECT id, name FROM users WHERE active = 1", &ctx("zh-CN")), Decision::Keep(Reason::CodeOrStructured));
        assert_eq!(classify(&cfg, "git commit -m 'fix bug'", &ctx("zh-CN")), Decision::Keep(Reason::CodeOrStructured));
        assert_eq!(
            classify(&cfg, "2026-11-28T10:00:00Z [ERROR] connection refused", &ctx("zh-CN")),
            Decision::Keep(Reason::CodeOrStructured)
        );
        assert_eq!(
            classify(&cfg, "fn main() {\n    let x = a::b();\n    return;\n}", &ctx("zh-CN")),
            Decision::Keep(Reason::CodeOrStructured)
        );
        assert_eq!(
            classify(&cfg, "Please select an item (optional); changes apply immediately.", &ctx("zh-CN")),
            Decision::Translate
        );
    }

    #[test]
    fn 语言识别兼顾短词安全与长文准确() {
        let cfg = Config::default();
        assert_eq!(classify(&cfg, "已经是中文了", &ctx("zh-CN")), Decision::Keep(Reason::AlreadyTargetLang));
        assert_eq!(classify(&cfg, "Open", &ctx("en")), Decision::Translate, "短词不交统计模型避免误判");
        assert_eq!(
            classify(&cfg, "This paragraph is clearly written in English and should be skipped when targeting English.", &ctx("en")),
            Decision::Keep(Reason::AlreadyTargetLang)
        );
        assert_eq!(classify(&cfg, "設定を開く", &ctx("ja")), Decision::Keep(Reason::AlreadyTargetLang));
        assert_eq!(classify(&cfg, "設定を開く", &ctx("zh-CN")), Decision::Translate);
        assert_eq!(classify(&cfg, "저장하기", &ctx("ko")), Decision::Keep(Reason::AlreadyTargetLang));
    }

    #[test]
    fn 用户规则优先级与环形日志去重() {
        clear_events();
        let mut cfg = Config::default();
        cfg.filter.keep_terms.push("Artifacts".into());
        cfg.filter.keep_patterns.push("^ERR-\\d+$".into());
        cfg.filter.apps.insert(
            "claude.exe".into(),
            FilterRuleSet {
                keep_terms: vec![],
                keep_patterns: vec![],
                force_translate: vec!["Opus 5.5".into()],
            },
        );
        assert_eq!(classify(&cfg, "Artifacts", &ctx("zh-CN")), Decision::Keep(Reason::UserKeepTerm));
        assert_eq!(classify(&cfg, "ERR-404", &ctx("zh-CN")), Decision::Keep(Reason::UserKeepPattern));
        assert_eq!(classify(&cfg, "Opus 5.5", &ctx("zh-CN")), Decision::ForceTranslate);

        record_event("Opus 5.5", "keep", Reason::ModelVersion, &ctx("zh-CN"));
        record_event("Opus 5.5", "keep", Reason::ModelVersion, &ctx("zh-CN"));
        let evs = recent_events(10, Some("claude.exe"), true);
        assert_eq!(evs.len(), 1, "短时间重复事件应去重合并");
        clear_events();
    }
}
