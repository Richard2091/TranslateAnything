//! 版本号与更新：查询 GitHub Releases 的最新正式版，有新版本时提示；
//! 一键下载并就地更新、自动重启：安装版覆盖安装（不先卸载旧版），便携版替换程序文件。
//!
//! 不引入 tauri-plugin-updater（它要求发布流程签名），检查和下载都走 GitHub Releases：
//! 官方接口 + 发布时附带的 `latest.json` 清单；清单走固定地址，可经 GitHub 加速镜像转发，
//! 方便访问 GitHub 困难的地区。默认自动模式：同时试官方和各镜像，选最快的；下载后按 SHA-256 校验。
//! 启动后稍等片刻检查一次，之后常驻期间每天检查一次；可在设置里关闭自动检查。

use crate::config::{self, Config};
use serde::Serialize;
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// 发布更新的仓库
pub const REPO: &str = "Richard2091/TranslateAnything";
/// 启动后多久做第一次自动检查（不和启动抢资源）
const FIRST_CHECK: Duration = Duration::from_secs(20);
/// 常驻期间的自动检查间隔
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
/// 安装包大小上限，防止下载到意外的大文件
const MAX_ASSET: u64 = 200 * 1024 * 1024;
/// 内置的 GitHub 加速镜像（前缀式：前缀 + 原 GitHub 链接），2026-11 实测可用。
/// 镜像随时可能失效：自动模式下失效的会被自然淘汰，用户也可以在设置里填自己的镜像
pub const BUILTIN_MIRRORS: &[&str] = &[
    "https://gh-proxy.com/",
    "https://ghfast.top/",
    "https://cdn.gh-proxy.com/",
    "https://edgeone.gh-proxy.com/",
    "https://gh-proxy.org/",
];
/// 每个 Release 附带的清单（版本、各安装包大小与 SHA-256），镜像通道靠它检查更新；
/// 旁边的 `latest.json.sig` 是它的签名
const MANIFEST: &str = "latest.json";
/// 验证清单签名的 Ed25519 公钥（十六进制）。私钥只在发布者手里和 GitHub Secrets `UPDATE_SIGNING_KEY` 里。
/// 换密钥时先把新公钥加进来发一版，之后再用新私钥签名、去掉旧公钥
const UPDATE_PUBKEYS: &[&str] = &["bb0a477ce84f16e640926edb730d9df0c1cae9888e7b12d82876f9ec05b032a3"];
/// 清单 / 接口响应的大小上限
const MAX_JSON: usize = 1024 * 1024;
/// 检查更新的单次请求超时
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// 第一个检查结果到达后，再等其他通道这么久，取其中最新的版本
const CHECK_GRACE: Duration = Duration::from_millis(1500);
/// 下载测速时长：各通道同时开始下载，连上后这么久内收到数据最多（按总耗时折算）的胜出
const PROBE: Duration = Duration::from_secs(3);
/// 下载时连接超时
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 下载整体超时（慢速镜像也要能下完）
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// 当前程序的版本信息
#[derive(Serialize, Clone)]
pub struct AppVersion {
    pub version: &'static str,
    /// `x64` / `arm64`
    pub arch: &'static str,
    pub portable: bool,
    pub repo: &'static str,
    /// 内置镜像（设置页的通道下拉框用）
    pub mirrors: &'static [&'static str],
}

/// 最新正式版
#[derive(Serialize, Clone, Debug)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    /// 比当前版本新
    pub newer: bool,
    /// Release 页面
    pub url: String,
    pub published_at: String,
    /// 更新说明（Release 正文，Markdown，上中文下英文）；镜像清单里没有时为空
    pub notes: String,
    /// 本机可用的安装包（安装版为 NSIS 安装程序，便携版为 zip）
    pub asset_name: Option<String>,
    /// 安装包的 GitHub 原始下载地址（走镜像时在前面加镜像前缀）
    #[serde(skip)]
    asset_url: Option<String>,
    pub asset_size: u64,
    /// 安装包 SHA-256（小写十六进制），下载后校验
    #[serde(skip)]
    sha256: Option<String>,
    /// 检查结果来自哪个通道：`github.com` 或镜像域名
    pub via: String,
}

/// 最近一次检查到的结果
static LATEST: Mutex<Option<UpdateInfo>> = Mutex::new(None);
/// 正在下载安装包
static INSTALLING: Mutex<bool> = Mutex::new(false);

pub fn current() -> AppVersion {
    AppVersion {
        version: env!("CARGO_PKG_VERSION"),
        arch: arch(),
        portable: config::portable_dir().is_some(),
        repo: REPO,
        mirrors: BUILTIN_MIRRORS,
    }
}

fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" }
}

/// 最近一次检查到、且用户没有选择跳过的新版本（托盘菜单用）
pub fn pending() -> Option<UpdateInfo> {
    let skip = config::load().map(|c| c.update.skip_version).unwrap_or_default();
    LATEST.lock().unwrap().clone().filter(|u| u.newer && u.latest != skip)
}

pub fn last_checked() -> Option<UpdateInfo> {
    LATEST.lock().unwrap().clone()
}

/// `v1.2.3` / `1.2.3-beta` → (1, 2, 3)；解析不了的段按 0 处理
fn parse_version(s: &str) -> (u64, u64, u64) {
    let core = s.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next().unwrap_or("");
    let mut it = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

fn is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest) > parse_version(current)
}

/// 按全局代理设置构造 HTTP 客户端
fn client(cfg: &Config, timeout: Duration) -> Result<reqwest::Client, String> {
    crate::translate::proxy_client(cfg.proxy.trim(), timeout).map_err(|e| e.to_string())
}

// ───────────────────────── 通道 ─────────────────────────

/// 要试的通道（前缀）：空串为官方直连，其余为镜像前缀（前缀 + 原 GitHub 链接）
fn channels(cfg: &Config) -> Vec<String> {
    match cfg.update.channel.trim() {
        "" | "auto" => std::iter::once(String::new()).chain(BUILTIN_MIRRORS.iter().map(|m| m.to_string())).collect(),
        "github" => vec![String::new()],
        m => vec![normalize_mirror(m).unwrap_or_default()],
    }
}

/// 镜像前缀：必须是 https，统一以 `/` 结尾；不合法返回 None（按官方直连处理）
fn normalize_mirror(s: &str) -> Option<String> {
    let s = s.trim();
    let host = s.strip_prefix("https://")?;
    if host.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    Some(if s.ends_with('/') { s.to_string() } else { format!("{s}/") })
}

/// 通道显示名：`github.com` 或镜像域名
fn via(prefix: &str) -> String {
    if prefix.is_empty() {
        return "github.com".into();
    }
    prefix.trim_start_matches("https://").split('/').next().unwrap_or(prefix).to_string()
}

fn release_page(version: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/v{version}")
}

fn asset_link(version: &str, name: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/v{version}/{name}")
}

/// 本机要的安装包：安装版为对应架构的 NSIS 安装程序，便携版为对应架构的 zip
fn wanted(name: &str, arch: &str, portable: bool) -> bool {
    let n = name.to_ascii_lowercase();
    if portable {
        n.ends_with(&format!("_{arch}_portable.zip"))
    } else {
        n.ends_with("-setup.exe") && n.contains(&format!("_{arch}"))
    }
}

// ───────────────────────── 检查 ─────────────────────────

type Attempt = std::pin::Pin<Box<dyn std::future::Future<Output = Result<UpdateInfo, String>> + Send>>;

/// 查询最新正式版（草稿和预发布不算）。
/// 自动模式同时问官方接口和各镜像，先到的结果再稍等片刻，取其中最新的版本（防镜像缓存了旧清单）
pub async fn check(app: &AppHandle) -> Result<UpdateInfo, String> {
    use futures_util::StreamExt;
    let cfg = config::load().unwrap_or_default();
    let http = client(&cfg, CHECK_TIMEOUT)?;
    let portable = config::portable_dir().is_some();
    let mut attempts: Vec<Attempt> = Vec::new();
    for prefix in channels(&cfg) {
        if prefix.is_empty() {
            attempts.push(Box::pin(check_api(http.clone(), portable)));
        }
        attempts.push(Box::pin(check_manifest(http.clone(), prefix, portable)));
    }
    // 按发起顺序编号，全部失败时报第一个（官方）的错误
    let mut pending: futures_util::stream::FuturesUnordered<_> =
        attempts.into_iter().enumerate().map(|(i, f)| async move { (i, f.await) }).collect();
    let mut best: Option<UpdateInfo> = None;
    let mut first_err: Option<(usize, String)> = None;
    let mut grace: Option<tokio::time::Instant> = None;
    loop {
        let next = match grace {
            Some(deadline) => match tokio::time::timeout_at(deadline, pending.next()).await {
                Ok(n) => n,
                Err(_) => break,
            },
            None => pending.next().await,
        };
        let Some((i, result)) = next else { break };
        match result {
            Ok(mut info) => {
                grace.get_or_insert_with(|| tokio::time::Instant::now() + CHECK_GRACE);
                // 更新说明只有官方接口给：同一版本时补到选中的结果上
                if let Some(b) = &mut best
                    && b.latest == info.latest
                {
                    if b.notes.is_empty() {
                        b.notes = info.notes.clone();
                    } else if info.notes.is_empty() {
                        info.notes = b.notes.clone();
                    }
                }
                if best.as_ref().is_none_or(|b| better(&info, b)) {
                    best = Some(info);
                }
            }
            Err(e) => {
                if first_err.as_ref().is_none_or(|(j, _)| i < *j) {
                    first_err = Some((i, e));
                }
            }
        }
    }
    let info = best.ok_or_else(|| first_err.map(|(_, e)| e).unwrap_or_else(|| t!("err.update.bad_response")))?;
    println!("检查更新：最新 {}，来自 {}", info.latest, info.via);
    *LATEST.lock().unwrap() = Some(info.clone());
    let _ = app.emit("update-state", &info);
    crate::tray::refresh_now(app);
    Ok(info)
}

/// 两个检查结果取哪个：版本更新的优先；同版本时优先带校验值、带匹配安装包的
fn better(a: &UpdateInfo, b: &UpdateInfo) -> bool {
    let key = |u: &UpdateInfo| (parse_version(&u.latest), u.sha256.is_some(), u.asset_name.is_some());
    key(a) > key(b)
}

async fn get_bytes(http: &reqwest::Client, url: &str, api: bool) -> Result<Vec<u8>, String> {
    let mut req = http.get(url).header("User-Agent", "TranslateAnything-updater").timeout(CHECK_TIMEOUT);
    if api {
        req = req.header("Accept", "application/vnd.github+json");
    }
    let resp = req.send().await.map_err(|e| t!("err.update.network", e = e))?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Err(t!("err.update.no_release"));
    }
    if !status.is_success() {
        return Err(t!("err.update.http", status = status));
    }
    let body = resp.bytes().await.map_err(|e| t!("err.update.network", e = e))?;
    if body.len() > MAX_JSON {
        return Err(t!("err.update.bad_response"));
    }
    Ok(body.to_vec())
}

async fn get_json(http: &reqwest::Client, url: &str, api: bool) -> Result<Value, String> {
    // 镜像出错时可能返回 200 的网页，解析失败一律当作无法识别
    serde_json::from_slice(&get_bytes(http, url, api).await?).map_err(|_| t!("err.update.bad_response"))
}

/// 校验清单签名：`latest.json.sig` 是对 `latest.json` 原始字节的 Ed25519 签名（十六进制），
/// 由发布流程用私钥生成；任一内置公钥验证通过即可
fn verify_manifest(body: &[u8], sig_hex: &[u8]) -> bool {
    use aws_lc_rs::signature::{UnparsedPublicKey, ED25519};
    let Some(sig) = std::str::from_utf8(sig_hex).ok().and_then(|s| decode_hex(s.trim())) else {
        return false;
    };
    UPDATE_PUBKEYS
        .iter()
        .filter_map(|k| decode_hex(k))
        .any(|k| UnparsedPublicKey::new(&ED25519, k).verify(body, &sig).is_ok())
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// 官方接口：带各安装包的 SHA-256（GitHub 自动计算的 digest）
async fn check_api(http: reqwest::Client, portable: bool) -> Result<UpdateInfo, String> {
    let v = get_json(&http, &format!("https://api.github.com/repos/{REPO}/releases/latest"), true).await?;
    parse_release(&v, env!("CARGO_PKG_VERSION"), arch(), portable).ok_or_else(|| t!("err.update.bad_response"))
}

/// 发布附带的清单 `latest.json`：经 `releases/latest/download/` 固定地址取，镜像也能转发
async fn check_manifest(http: reqwest::Client, prefix: String, portable: bool) -> Result<UpdateInfo, String> {
    let url = format!("{prefix}https://github.com/{REPO}/releases/latest/download/{MANIFEST}");
    let (body, sig) = futures_util::future::join(get_bytes(&http, &url, false), get_bytes(&http, &format!("{url}.sig"), false)).await;
    let body = body?;
    // 镜像是第三方服务：清单必须带有效签名，否则整条结果不用
    if !verify_manifest(&body, &sig?) {
        return Err(t!("err.update.signature"));
    }
    let v: Value = serde_json::from_slice(&body).map_err(|_| t!("err.update.bad_response"))?;
    parse_manifest(&v, env!("CARGO_PKG_VERSION"), arch(), portable, via(&prefix)).ok_or_else(|| t!("err.update.bad_response"))
}

fn parse_release(v: &Value, current: &str, arch: &str, portable: bool) -> Option<UpdateInfo> {
    let tag = v.get("tag_name")?.as_str()?;
    let latest = tag.trim_start_matches(['v', 'V']).to_string();
    let asset = v
        .get("assets")?
        .as_array()?
        .iter()
        .find(|a| a.get("name").and_then(Value::as_str).is_some_and(|n| wanted(n, arch, portable)));
    let name = asset.and_then(|a| a.get("name")?.as_str()).map(str::to_string);
    Some(UpdateInfo {
        newer: is_newer(&latest, current),
        current: current.to_string(),
        url: release_page(&latest),
        published_at: v.get("published_at").and_then(Value::as_str).unwrap_or_default().to_string(),
        notes: v.get("body").and_then(Value::as_str).unwrap_or_default().trim().to_string(),
        asset_url: name.as_deref().map(|n| asset_link(&latest, n)),
        asset_size: asset.and_then(|a| a.get("size")?.as_u64()).unwrap_or(0),
        sha256: asset
            .and_then(|a| a.get("digest")?.as_str()?.strip_prefix("sha256:"))
            .and_then(normalize_sha256),
        asset_name: name,
        via: "github.com".into(),
        latest,
    })
}

/// `latest.json`：`{ "version": "0.3.0", "published_at": "…", "assets": [{ "name", "size", "sha256" }] }`
fn parse_manifest(v: &Value, current: &str, arch: &str, portable: bool, via: String) -> Option<UpdateInfo> {
    let latest = v.get("version")?.as_str()?.trim_start_matches(['v', 'V']).to_string();
    if parse_version(&latest) == (0, 0, 0) {
        return None;
    }
    let asset = v
        .get("assets")?
        .as_array()?
        .iter()
        .find(|a| a.get("name").and_then(Value::as_str).is_some_and(|n| wanted(n, arch, portable)));
    // 文件名只取清单里的纯文件名，链接自己拼，不信任清单里的任何地址
    let name = asset.and_then(|a| a.get("name")?.as_str()).filter(|n| !n.contains(['/', '\\', '?', '#'])).map(str::to_string);
    Some(UpdateInfo {
        newer: is_newer(&latest, current),
        current: current.to_string(),
        url: release_page(&latest),
        published_at: v.get("published_at").and_then(Value::as_str).unwrap_or_default().to_string(),
        notes: v.get("notes").and_then(Value::as_str).unwrap_or_default().trim().to_string(),
        asset_url: name.as_deref().map(|n| asset_link(&latest, n)),
        asset_size: asset.and_then(|a| a.get("size")?.as_u64()).unwrap_or(0),
        sha256: asset.and_then(|a| a.get("sha256")?.as_str()).and_then(normalize_sha256),
        asset_name: name,
        via,
        latest,
    })
}

fn normalize_sha256(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).then_some(s)
}

/// 常驻后台的自动检查：启动后检查一次，之后每天一次；关闭自动检查后只是跳过，不退出循环
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        cleanup_old_exe();
        loop {
            if config::load().map(|c| c.update.auto_check).unwrap_or(true) {
                match check(&app).await {
                    Ok(u) if u.newer => println!("发现新版本 {}（当前 {}）", u.latest, u.current),
                    Ok(_) => {}
                    Err(e) => eprintln!("检查更新失败：{e}"),
                }
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// 打开最新版本的 Release 页面（没有匹配的安装包时）
pub fn open_page() -> Result<(), String> {
    let url = last_checked()
        .map(|u| u.url)
        .filter(|u| u.starts_with("https://github.com/"))
        .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/latest"));
    crate::dialog::open_url(&url)
}

/// 安装程序参数（与 tauri-plugin-updater 一致）：`/UPDATE` 直接覆盖安装、不先卸载旧版；
/// `/P` 被动模式，只显示进度不用点选；`/R` 装完自动启动新版本
const INSTALLER_ARGS: [&str; 3] = ["/P", "/R", "/UPDATE"];

/// 便携版更新后新进程的启动参数：等旧进程退出、释放单实例锁后再启动
pub const AFTER_UPDATE_ARG: &str = "--after-update";

/// 下载新版本并就地更新，随后自动重启：
/// 安装版以覆盖模式运行安装程序；便携版解压出新程序替换自身（数据目录不动）。
/// 没有匹配本机的安装包时打开下载页
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let info = last_checked().filter(|u| u.newer).ok_or_else(|| t!("err.update.none"))?;
    let (Some(name), Some(url)) = (info.asset_name.clone(), info.asset_url.clone()) else {
        return open_page();
    };
    if !url.starts_with(&format!("https://github.com/{REPO}/releases/download/")) || name.contains(['/', '\\']) {
        return Err(t!("err.update.bad_response"));
    }
    {
        let mut busy = INSTALLING.lock().unwrap();
        if *busy {
            return Err(t!("err.update.busy"));
        }
        *busy = true;
    }
    let result = apply(app, &info, &name, &url).await;
    if result.is_err() {
        *INSTALLING.lock().unwrap() = false;
    }
    result
}

async fn apply(app: &AppHandle, info: &UpdateInfo, name: &str, url: &str) -> Result<(), String> {
    let path = download(app, info, name, url).await?;
    if config::portable_dir().is_some() {
        // 先换好文件（失败则原样保留），再停沉浸模式、启动新程序
        let exe = tauri::async_runtime::spawn_blocking(move || replace_portable(&path))
            .await
            .map_err(|e| t!("err.update.save", e = e))??;
        crate::immersive::shutdown(app).await;
        std::process::Command::new(&exe).arg(AFTER_UPDATE_ARG).spawn().map_err(|e| t!("err.update.run", e = e))?;
        println!("便携版已更新为 {}，重启", info.latest);
    } else {
        // 被动模式的安装程序发现本程序还在运行会直接结束它，所以先让所有页面恢复原文，再启动安装程序
        crate::immersive::shutdown(app).await;
        run_installer(&path)?;
        println!("已启动安装程序 {}，退出本程序", path.display());
    }
    app.exit(0);
    Ok(())
}

/// 旧程序替换后留下的文件名：`TranslateAnything.exe` → `TranslateAnything.exe.old`
fn old_exe_path(exe: &std::path::Path) -> std::path::PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(".old");
    s.into()
}

/// 便携版：解压新版 zip，把正在运行的程序改名让位，新程序放到原位置。返回程序路径
fn replace_portable(zip: &std::path::Path) -> Result<std::path::PathBuf, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let save_err = |e: &dyn std::fmt::Display| t!("err.update.save", e = e);
    let dir = zip.with_extension("");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| save_err(&e))?;
    // 用系统自带的 tar（Windows 10 1803 起自带，支持 zip）解压，不为此引入压缩库
    let tar = std::env::var_os("SystemRoot")
        .map(|r| std::path::PathBuf::from(r).join("System32").join("tar.exe"))
        .unwrap_or_else(|| "tar.exe".into());
    let out = std::process::Command::new(tar)
        .arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(&dir)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| save_err(&e))?;
    if !out.status.success() {
        return Err(save_err(&String::from_utf8_lossy(&out.stderr).trim()));
    }
    let new_exe = find_exe(&dir, 2).ok_or_else(|| t!("err.update.bad_response"))?;
    let exe = std::env::current_exe().map_err(|e| save_err(&e))?;
    let old = old_exe_path(&exe);
    let _ = std::fs::remove_file(&old);
    // 运行中的程序不能覆盖，但可以改名
    std::fs::rename(&exe, &old).map_err(|e| save_err(&e))?;
    if let Err(e) = std::fs::copy(&new_exe, &exe) {
        let _ = std::fs::remove_file(&exe);
        let _ = std::fs::rename(&old, &exe);
        return Err(save_err(&e));
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(zip);
    Ok(exe)
}

/// 在解压目录里找主程序（zip 内是 `TranslateAnything/TranslateAnything.exe`）
fn find_exe(dir: &std::path::Path, depth: u32) -> Option<std::path::PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                if let Some(f) = find_exe(&p, depth - 1) {
                    return Some(f);
                }
            }
        } else if p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("TranslateAnything.exe")) {
            return Some(p);
        }
    }
    None
}

/// 删掉上次便携版更新留下的旧程序（启动一会儿后旧进程早已退出）
fn cleanup_old_exe() {
    if let Ok(exe) = std::env::current_exe() {
        let old = old_exe_path(&exe);
        if old.exists() {
            let _ = std::fs::remove_file(&old);
        }
    }
}

/// 一个通道的测速结果：已收到的数据、没读完的响应（读完了为 None）、折算速度
struct Probe {
    prefix: String,
    buf: Vec<u8>,
    resp: Option<reqwest::Response>,
    speed: f64,
}

/// 连上通道并读 `window` 这么久（从连上算起），用于比较速度；window 为 0 时只连接
async fn probe(http: reqwest::Client, prefix: String, url: String, size: u64, window: Duration) -> Result<Probe, String> {
    let start = tokio::time::Instant::now();
    let send = http.get(format!("{prefix}{url}")).header("User-Agent", "TranslateAnything-updater").send();
    let mut resp = tokio::time::timeout(CONNECT_TIMEOUT, send)
        .await
        .map_err(|_| t!("err.update.network", e = "timeout"))?
        .map_err(|e| t!("err.update.network", e = e))?;
    if !resp.status().is_success() {
        return Err(t!("err.update.http", status = resp.status()));
    }
    // 镜像出错时可能返回 200 的网页：长度对不上就不用它
    let len = resp.content_length();
    if len.is_some_and(|l| l > MAX_ASSET || (size > 0 && l != size)) {
        return Err(t!("err.update.bad_response"));
    }
    let until = tokio::time::Instant::now() + window;
    let mut buf = Vec::new();
    let mut finished = false;
    while !window.is_zero() {
        match tokio::time::timeout_at(until, resp.chunk()).await {
            Err(_) => break,
            Ok(Ok(Some(c))) => {
                buf.extend_from_slice(&c);
                if buf.len() as u64 > MAX_ASSET {
                    return Err(t!("err.update.bad_response"));
                }
            }
            Ok(Ok(None)) => {
                finished = true;
                break;
            }
            Ok(Err(e)) => return Err(t!("err.update.network", e = e)),
        }
    }
    let speed = buf.len() as f64 / start.elapsed().as_secs_f64().max(0.001);
    Ok(Probe { prefix, buf, resp: (!finished).then_some(resp), speed })
}

/// 打开下载：单个通道直接连；多个通道同时测速，留下最快的继续下载，其余断开
async fn open_fastest(http: &reqwest::Client, prefixes: Vec<String>, url: &str, size: u64) -> Result<Probe, String> {
    if prefixes.len() == 1 {
        let prefix = prefixes.into_iter().next().unwrap_or_default();
        return probe(http.clone(), prefix, url.to_string(), size, Duration::ZERO).await;
    }
    let results = futures_util::future::join_all(
        prefixes.into_iter().map(|p| probe(http.clone(), p, url.to_string(), size, PROBE)),
    )
    .await;
    let mut first_err = None;
    let mut best: Option<Probe> = None;
    for r in results {
        match r {
            Ok(p) => {
                println!("更新测速：{} {:.0} KB/s", via(&p.prefix), p.speed / 1024.0);
                if best.as_ref().is_none_or(|b| p.speed > b.speed) {
                    best = Some(p);
                }
            }
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    best.ok_or_else(|| first_err.unwrap_or_else(|| t!("err.update.bad_response")))
}

async fn download(app: &AppHandle, info: &UpdateInfo, name: &str, url: &str) -> Result<std::path::PathBuf, String> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let cfg = config::load().unwrap_or_default();
    let dir = std::env::temp_dir().join("TranslateAnything-update");
    std::fs::create_dir_all(&dir).map_err(|e| t!("err.update.save", e = e))?;
    let path = dir.join(name);
    let http = client(&cfg, DOWNLOAD_TIMEOUT)?;
    // 没有可信的校验值时只从官方下载，不经第三方镜像
    let mut prefixes = channels(&cfg);
    if info.sha256.is_none() {
        prefixes = vec![String::new()];
    }
    let Probe { prefix, buf, resp, .. } = open_fastest(&http, prefixes, url, info.asset_size).await?;
    println!("下载更新：经 {}", via(&prefix));
    let _ = app.emit("update-via", via(&prefix));
    let total = resp.as_ref().and_then(|r| r.content_length()).unwrap_or(info.asset_size);
    let part = path.with_extension("part");
    let mut file = std::fs::File::create(&part).map_err(|e| t!("err.update.save", e = e))?;
    let mut hasher = Sha256::new();
    let (mut done, mut last_pct) = (0u64, u64::MAX);
    let mut write = |chunk: &[u8]| -> Result<(), String> {
        done += chunk.len() as u64;
        if done > MAX_ASSET {
            return Err(t!("err.update.bad_response"));
        }
        hasher.update(chunk);
        file.write_all(chunk).map_err(|e| t!("err.update.save", e = e))?;
        let pct = if total > 0 { (done * 100 / total).min(100) } else { 0 };
        if pct != last_pct {
            last_pct = pct;
            let _ = app.emit("update-progress", pct);
        }
        Ok(())
    };
    let mut result = write(&buf);
    if let Some(mut resp) = resp.filter(|_| result.is_ok()) {
        loop {
            match resp.chunk().await {
                Ok(Some(c)) => {
                    if let Err(e) = write(&c) {
                        result = Err(e);
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    result = Err(t!("err.update.network", e = e));
                    break;
                }
            }
        }
    }
    drop(write);
    drop(file);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    if info.asset_size > 0 && done != info.asset_size {
        let _ = std::fs::remove_file(&part);
        return Err(t!("err.update.incomplete"));
    }
    // 校验 SHA-256：镜像是第三方服务，防止拿到被改过或损坏的安装包
    let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if info.sha256.as_deref().is_some_and(|want| want != digest) {
        let _ = std::fs::remove_file(&part);
        return Err(t!("err.update.digest"));
    }
    std::fs::rename(&part, &path).map_err(|e| t!("err.update.save", e = e))?;
    Ok(path)
}

/// 以覆盖安装模式启动安装程序
fn run_installer(path: &std::path::Path) -> Result<(), String> {
    match std::process::Command::new(path).args(INSTALLER_ARGS).spawn() {
        Ok(_) => Ok(()),
        // 需要管理员权限（ERROR_ELEVATION_REQUIRED，如装在 Program Files 下）：由系统弹出授权提示
        Err(e) if e.raw_os_error() == Some(740) => run_elevated(path),
        Err(e) => Err(t!("err.update.run", e = e)),
    }
}

/// 以管理员身份启动安装程序（带同样的参数）
fn run_elevated(path: &std::path::Path) -> Result<(), String> {
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let file = HSTRING::from(path.as_os_str());
    let params = HSTRING::from(INSTALLER_ARGS.join(" "));
    // 返回值大于 32 表示成功
    let r = unsafe { ShellExecuteW(None, w!("runas"), &file, &params, None, SW_SHOWNORMAL) };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(t!("err.update.run", e = std::io::Error::last_os_error()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn 比较版本() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("garbage", "0.2.0"));
    }

    #[test]
    fn 挑选安装包() {
        let sha = "a".repeat(64);
        let v = json!({
            "tag_name": "v0.3.0",
            "html_url": "https://github.com/Richard2091/TranslateAnything/releases/tag/v0.3.0",
            "published_at": "2026-11-20T00:00:00Z",
            "assets": [
                { "name": "Translate.Anything_0.3.0_x64_en-US.msi", "size": 1 },
                { "name": "Translate.Anything_0.3.0_x64-setup.exe", "size": 2, "digest": format!("sha256:{sha}") },
                { "name": "Translate.Anything_0.3.0_arm64-setup.exe", "size": 3 },
                { "name": "TranslateAnything_0.3.0_x64_portable.zip", "size": 4 },
            ]
        });
        let u = parse_release(&v, "0.2.0", "x64", false).unwrap();
        assert!(u.newer);
        assert_eq!(u.latest, "0.3.0");
        assert_eq!(
            u.asset_url.as_deref(),
            Some("https://github.com/Richard2091/TranslateAnything/releases/download/v0.3.0/Translate.Anything_0.3.0_x64-setup.exe")
        );
        assert_eq!(u.sha256.as_deref(), Some(sha.as_str()));
        let u = parse_release(&v, "0.2.0", "arm64", false).unwrap();
        assert_eq!(u.asset_size, 3);
        assert!(u.sha256.is_none());
        let u = parse_release(&v, "0.3.0", "x64", true).unwrap();
        assert!(!u.newer);
        assert_eq!(u.asset_name.as_deref(), Some("TranslateAnything_0.3.0_x64_portable.zip"));
    }

    #[test]
    fn 解析清单() {
        let sha = "B".repeat(64);
        let v = json!({
            "version": "0.3.0",
            "assets": [
                { "name": "Translate.Anything_0.3.0_x64-setup.exe", "size": 2, "sha256": sha },
                { "name": "../evil_x64_portable.zip", "size": 4, "sha256": sha },
            ]
        });
        let u = parse_manifest(&v, "0.2.0", "x64", false, "gh-proxy.com".into()).unwrap();
        assert!(u.newer);
        assert_eq!(u.via, "gh-proxy.com");
        assert_eq!(u.sha256.as_deref(), Some("b".repeat(64).as_str()));
        // 带路径的文件名不认
        let u = parse_manifest(&v, "0.2.0", "x64", true, "x".into()).unwrap();
        assert!(u.asset_name.is_none());
        assert!(parse_manifest(&json!({ "version": "garbage", "assets": [] }), "0.2.0", "x64", false, "x".into()).is_none());
    }

    #[test]
    fn 镜像前缀() {
        assert_eq!(normalize_mirror(" https://gh-proxy.com ").as_deref(), Some("https://gh-proxy.com/"));
        assert_eq!(normalize_mirror("https://a.b/x/").as_deref(), Some("https://a.b/x/"));
        assert!(normalize_mirror("http://gh-proxy.com/").is_none());
        assert!(normalize_mirror("https://").is_none());
        assert_eq!(via("https://ghfast.top/"), "ghfast.top");
        assert_eq!(via(""), "github.com");
        let mut cfg = Config::default();
        assert_eq!(channels(&cfg).len(), 1 + BUILTIN_MIRRORS.len());
        cfg.update.channel = "github".into();
        assert_eq!(channels(&cfg), vec![String::new()]);
        cfg.update.channel = "https://my.mirror".into();
        assert_eq!(channels(&cfg), vec!["https://my.mirror/".to_string()]);
    }

    #[test]
    fn 取最新的检查结果() {
        let v = |ver: &str| json!({ "version": ver, "assets": [] });
        let old = parse_manifest(&v("0.3.0"), "0.2.0", "x64", false, "a".into()).unwrap();
        let new = parse_manifest(&v("0.3.1"), "0.2.0", "x64", false, "b".into()).unwrap();
        assert!(better(&new, &old));
        assert!(!better(&old, &new));
    }

    #[test]
    fn 清单签名() {
        // 用发布私钥按 release.yml 同样的方式（openssl pkeyutl -sign -rawin）签出的样例
        let body = br#"{"version":"9.9.9","assets":[]}"#;
        let sig = b"4ed57eec71ebe885bd47ff40aaa3174b9207235461d9a76e5abae5850b7a57facecca81595c7f923f3803e6f2312e0c6306a94444e228faa14c99ce99bd9030b
";
        assert!(verify_manifest(body, sig));
        assert!(!verify_manifest(br#"{"version":"9.9.8","assets":[]}"#, sig));
        assert!(!verify_manifest(body, b"00"));
        assert!(!verify_manifest(body, b"not hex"));
    }
}
