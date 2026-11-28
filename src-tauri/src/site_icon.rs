//! 网站图标：给没有内置 logo 的 AI 供应商（自定义接口等）取接口域名所在网站的图标
//!
//! 只直接访问用户自己填的接口域名（及其上级域名），不借助第三方图标服务；走该供应商的代理设置。
//! 结果缓存在 `<数据目录>/icons/<域名>`：取到的保留 30 天，取不到的记一个空文件，3 天内不再重试。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use regex::Regex;
use reqwest::Url;
use serde::Serialize;

use crate::config;

/// 单个图标的大小上限
const MAX_ICON: usize = 256 * 1024;
/// 首页 HTML 只读这么多（`<link rel="icon">` 都在 `<head>` 里）
const MAX_HTML: usize = 512 * 1024;
const HIT_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
const MISS_TTL: Duration = Duration::from_secs(3 * 24 * 3600);

#[derive(Serialize)]
pub struct SiteIcon {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// 按接口地址取网站图标；本机 / 内网地址、取不到时返回 None
pub async fn fetch(base_url: &str, proxy: &str) -> Option<SiteIcon> {
    let url = Url::parse(base_url.trim()).ok()?;
    let host = host_of(base_url)?;
    if !is_public_name(&host) {
        return None;
    }
    let cache = cache_path(&host);
    if let Some(path) = &cache {
        if let Some(hit) = read_cache(path) {
            return hit;
        }
    }
    let client = crate::translate::proxy_client(proxy, Duration::from_secs(8)).ok()?;
    let scheme = if url.scheme() == "http" { "http" } else { "https" };
    let reached = AtomicBool::new(false);
    let mut found = None;
    for origin in origins(scheme, &host) {
        if let Some(icon) = from_origin(&client, &origin, &reached).await {
            found = Some(icon);
            break;
        }
    }
    // 连不上（断网、代理没配好）不记"没有图标"，下次再试
    if let Some(path) = cache.as_ref().filter(|_| found.is_some() || reached.load(Ordering::Relaxed)) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, found.as_ref().map_or(&[][..], |i| &i.bytes[..]));
    }
    found
}

fn host_of(base_url: &str) -> Option<String> {
    Url::parse(base_url.trim()).ok()?.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase())
}

/// 清理缓存：删掉已没有任何引擎使用的域名的图标（删除供应商、改接口地址后调用）
pub fn prune(cfg: &config::Config) {
    let Some(dir) = config::data_dir().map(|d| d.join("icons")) else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let used: std::collections::HashSet<String> = cfg.engines.values().filter_map(|p| host_of(&p.base_url)).collect();
    for e in entries.flatten() {
        if !used.contains(&*e.file_name().to_string_lossy()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// 只给有公网域名的地址取图标：IP、localhost、单段主机名、.local 等内网名一律跳过
fn is_public_name(host: &str) -> bool {
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') || !host.contains('.') {
        return false;
    }
    !["localhost", ".local", ".lan", ".internal", ".home", ".test"].iter().any(|s| host.ends_with(s))
}

/// 依次尝试的站点：接口域名本身，再逐级去掉最左一段（api.foo.example.com → foo.example.com → example.com）
fn origins(scheme: &str, host: &str) -> Vec<String> {
    let labels: Vec<&str> = host.split('.').collect();
    (0..labels.len().saturating_sub(1)).take(3).map(|i| format!("{scheme}://{}", labels[i..].join("."))).collect()
}

async fn from_origin(client: &reqwest::Client, origin: &str, reached: &AtomicBool) -> Option<SiteIcon> {
    let base = Url::parse(&format!("{origin}/")).ok()?;
    // 先看首页声明的图标（往往是清晰的 PNG / SVG），再退到 /favicon.ico
    if let Some(html) = get(client, base.as_str(), MAX_HTML, reached).await {
        let final_base = html.0;
        for href in icon_links(&String::from_utf8_lossy(&html.1)) {
            // 有的站点把图标直接内联在页面里
            if href.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("data:")) {
                if let Some(icon) = data_uri(&href).and_then(as_icon) {
                    return Some(icon);
                }
                continue;
            }
            let Ok(u) = final_base.join(&href) else { continue };
            if !matches!(u.scheme(), "http" | "https") {
                continue;
            }
            if let Some(icon) = get(client, u.as_str(), MAX_ICON, reached).await.and_then(|(_, b)| as_icon(b)) {
                return Some(icon);
            }
        }
    }
    let ico = base.join("/favicon.ico").ok()?;
    get(client, ico.as_str(), MAX_ICON, reached).await.and_then(|(_, b)| as_icon(b))
}

/// GET 并限制读取量；返回跳转后的最终地址和内容。状态码非 2xx、超出上限都算失败
async fn get(client: &reqwest::Client, url: &str, limit: usize, reached: &AtomicBool) -> Option<(Url, Vec<u8>)> {
    let mut resp = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) TranslateAnything")
        .send()
        .await
        .ok()?;
    reached.store(true, Ordering::Relaxed);
    if !resp.status().is_success() {
        return None;
    }
    let final_url = resp.url().clone();
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if buf.len() + chunk.len() > limit {
            return None;
        }
        buf.extend_from_slice(&chunk);
    }
    Some((final_url, buf))
}

/// 从 HTML 里找图标链接，按清晰度排序：SVG → 标了尺寸的大图 → apple-touch-icon → 其他
fn icon_links(html: &str) -> Vec<String> {
    static LINK: OnceLock<Regex> = OnceLock::new();
    static ATTR: OnceLock<Regex> = OnceLock::new();
    let link = LINK.get_or_init(|| Regex::new(r"(?is)<link\b[^>]*>").unwrap());
    let attr = ATTR.get_or_init(|| Regex::new(r#"(?is)([a-z][a-z-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap());
    let mut found: Vec<(i32, String)> = Vec::new();
    for tag in link.find_iter(html) {
        let (mut rel, mut href, mut sizes, mut ty) = (String::new(), String::new(), String::new(), String::new());
        for c in attr.captures_iter(tag.as_str()) {
            let v = c.get(2).or(c.get(3)).or(c.get(4)).map_or("", |m| m.as_str()).trim().to_string();
            match c[1].to_ascii_lowercase().as_str() {
                "rel" => rel = v.to_ascii_lowercase(),
                "href" => href = v.replace("&amp;", "&"),
                "sizes" => sizes = v.to_ascii_lowercase(),
                "type" => ty = v.to_ascii_lowercase(),
                _ => {}
            }
        }
        if href.is_empty() || !rel.split_whitespace().any(|r| r == "icon" || r == "apple-touch-icon") {
            continue;
        }
        let px = sizes.split_whitespace().filter_map(|s| s.split('x').next()?.parse::<i32>().ok()).max().unwrap_or(0);
        let score = if ty.contains("svg") || href.to_ascii_lowercase().ends_with(".svg") {
            1000
        } else if px > 0 {
            px.min(512)
        } else if rel.contains("apple-touch-icon") {
            180
        } else {
            16
        };
        found.push((score, href));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.dedup_by(|a, b| a.1 == b.1);
    found.into_iter().map(|(_, h)| h).take(4).collect()
}

/// 解出 `data:` URI 的内容（base64 或百分号编码）；格式由 [`sniff`] 按内容判断，不信声明的类型
fn data_uri(s: &str) -> Option<Vec<u8>> {
    let (meta, body) = s.get(5..)?.split_once(',')?;
    let out = if meta.to_ascii_lowercase().ends_with(";base64") {
        base64_decode(body)?
    } else {
        percent_decode(body)
    };
    (out.len() <= MAX_ICON).then_some(out)
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// 按文件头识别图片格式；不是图片（如被重定向到的 HTML 页面）返回 None
fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.starts_with(&[0, 0, 1, 0]) {
        Some("image/x-icon")
    } else if b.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if b.starts_with(b"GIF8") {
        Some("image/gif")
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        let head = String::from_utf8_lossy(&b[..b.len().min(512)]).to_ascii_lowercase();
        (head.contains("<svg") && !head.contains("<html") && !head.contains("<!doctype html")).then_some("image/svg+xml")
    }
}

fn as_icon(bytes: Vec<u8>) -> Option<SiteIcon> {
    let mime = sniff(&bytes)?;
    Some(SiteIcon { mime, bytes })
}

fn cache_path(host: &str) -> Option<PathBuf> {
    // 域名只含字母、数字、点和连字符，直接当文件名
    if !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return None;
    }
    Some(config::data_dir()?.join("icons").join(host))
}

/// 缓存命中：Some(Some(图标)) / Some(None) = 近期取过但没有；None = 未缓存或已过期
fn read_cache(path: &PathBuf) -> Option<Option<SiteIcon>> {
    let meta = std::fs::metadata(path).ok()?;
    let age = SystemTime::now().duration_since(meta.modified().ok()?).unwrap_or_default();
    if meta.len() == 0 {
        return (age < MISS_TTL).then_some(None);
    }
    if age >= HIT_TTL {
        return None;
    }
    Some(as_icon(std::fs::read(path).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 候选站点逐级上溯() {
        assert_eq!(origins("https", "api.foo.example.com"), ["https://api.foo.example.com", "https://foo.example.com", "https://example.com"]);
        assert_eq!(origins("https", "example.com"), ["https://example.com"]);
    }

    #[test]
    fn 跳过内网地址() {
        assert!(!is_public_name("localhost"));
        assert!(!is_public_name("127.0.0.1"));
        assert!(!is_public_name("nas"));
        assert!(!is_public_name("box.local"));
        assert!(is_public_name("api.example.com"));
    }

    #[test]
    fn 图标链接按清晰度排序() {
        let html = r#"<head><link rel="icon" href="/a.ico"><link rel='apple-touch-icon' href='/t.png'>
            <link href="/b.png" rel="icon" sizes="32x32"><link rel="icon" type="image/svg+xml" href="/c.svg?v=1&amp;x=2">
            <link rel="stylesheet" href="/s.css"></head>"#;
        assert_eq!(icon_links(html), ["/c.svg?v=1&x=2", "/t.png", "/b.png", "/a.ico"]);
    }

    /// 联网实测：几个常见接口域名都能取到图标
    #[tokio::test]
    #[ignore]
    async fn 联网实测网站图标() {
        for url in ["https://api.deepseek.com/v1", "https://api.groq.com/openai/v1", "https://api.together.xyz/v1"] {
            let host = host_of(url).unwrap();
            let mut found = None;
            let reached = AtomicBool::new(false);
            let client = crate::translate::proxy_client("http://127.0.0.1:7897", Duration::from_secs(8)).unwrap();
            for origin in origins("https", &host) {
                if let Some(i) = from_origin(&client, &origin, &reached).await {
                    found = Some(i);
                    break;
                }
            }
            println!("{url}: {:?}", found.as_ref().map(|i| (i.mime, i.bytes.len())));
            assert!(found.is_some(), "{url} 没取到");
        }
    }

    #[test]
    fn 解析内联图标() {
        assert_eq!(data_uri("data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=").unwrap(), b"<svg></svg>");
        assert_eq!(data_uri("data:image/svg+xml,%3Csvg%3E%3C/svg%3E").unwrap(), b"<svg></svg>");
        assert_eq!(base64_decode("aGVsbG8gd29ybGQ").unwrap(), b"hello world");
        assert!(data_uri("data:image/png;base64,@@@").is_none());
    }

    #[test]
    fn 识别图片格式() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nxxxx"), Some("image/png"));
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><svg xmlns=..."), Some("image/svg+xml"));
        assert_eq!(sniff(b"<!doctype html><html>"), None);
    }
}
