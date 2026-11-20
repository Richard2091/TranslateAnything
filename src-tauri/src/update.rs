//! 版本号与更新：查询 GitHub Releases 的最新正式版，有新版本时提示；
//! 安装版一键下载安装包并启动安装程序，便携版打开下载页。
//!
//! 不引入 tauri-plugin-updater（它要求发布流程签名并另外维护 latest.json），直接读 Releases 接口：
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

/// 当前程序的版本信息
#[derive(Serialize, Clone)]
pub struct AppVersion {
    pub version: &'static str,
    /// `x64` / `arm64`
    pub arch: &'static str,
    pub portable: bool,
    pub repo: &'static str,
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
    /// 本机可用的安装包（安装版为 NSIS 安装程序，便携版为 zip）
    pub asset_name: Option<String>,
    #[serde(skip)]
    asset_url: Option<String>,
    pub asset_size: u64,
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
fn client(cfg: &Config) -> Result<reqwest::Client, String> {
    crate::translate::proxy_client(cfg.proxy.trim(), Duration::from_secs(300)).map_err(|e| e.to_string())
}

/// 查询最新正式版（草稿和预发布不算）
pub async fn check(app: &AppHandle) -> Result<UpdateInfo, String> {
    let cfg = config::load().unwrap_or_default();
    let resp = client(&cfg)?
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("User-Agent", "TranslateAnything-updater")
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| t!("err.update.network", e = e))?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Err(t!("err.update.no_release"));
    }
    if !status.is_success() {
        return Err(t!("err.update.http", status = status));
    }
    let v: Value = resp.json().await.map_err(|e| t!("err.update.network", e = e))?;
    let info = parse_release(&v, env!("CARGO_PKG_VERSION"), arch(), config::portable_dir().is_some())
        .ok_or_else(|| t!("err.update.bad_response"))?;
    *LATEST.lock().unwrap() = Some(info.clone());
    let _ = app.emit("update-state", &info);
    crate::tray::refresh_now(app);
    Ok(info)
}

fn parse_release(v: &Value, current: &str, arch: &str, portable: bool) -> Option<UpdateInfo> {
    let tag = v.get("tag_name")?.as_str()?;
    let latest = tag.trim_start_matches(['v', 'V']).to_string();
    let want = |name: &str| {
        let n = name.to_ascii_lowercase();
        if portable {
            n.ends_with(&format!("_{arch}_portable.zip"))
        } else {
            n.ends_with("-setup.exe") && n.contains(&format!("_{arch}"))
        }
    };
    let asset = v.get("assets")?.as_array()?.iter().find(|a| a.get("name").and_then(Value::as_str).is_some_and(want));
    Some(UpdateInfo {
        newer: is_newer(&latest, current),
        current: current.to_string(),
        latest,
        url: v.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        published_at: v.get("published_at").and_then(Value::as_str).unwrap_or_default().to_string(),
        asset_name: asset.and_then(|a| a.get("name")?.as_str()).map(str::to_string),
        asset_url: asset.and_then(|a| a.get("browser_download_url")?.as_str()).map(str::to_string),
        asset_size: asset.and_then(|a| a.get("size")?.as_u64()).unwrap_or(0),
    })
}

/// 常驻后台的自动检查：启动后检查一次，之后每天一次；关闭自动检查后只是跳过，不退出循环
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
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

/// 打开最新版本的 Release 页面（便携版、或没有匹配的安装包时）
pub fn open_page() -> Result<(), String> {
    let url = last_checked()
        .map(|u| u.url)
        .filter(|u| u.starts_with("https://github.com/"))
        .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/latest"));
    crate::dialog::open_url(&url)
}

/// 下载安装包并启动安装程序，随后退出本程序让安装程序替换文件。
/// 便携版不自动替换，打开下载页
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let info = last_checked().filter(|u| u.newer).ok_or_else(|| t!("err.update.none"))?;
    if config::portable_dir().is_some() {
        return open_page();
    }
    let (Some(name), Some(url)) = (info.asset_name.clone(), info.asset_url.clone()) else {
        return open_page();
    };
    if !url.starts_with("https://github.com/") || name.contains(['/', '\\']) {
        return Err(t!("err.update.bad_response"));
    }
    {
        let mut busy = INSTALLING.lock().unwrap();
        if *busy {
            return Err(t!("err.update.busy"));
        }
        *busy = true;
    }
    let result = download_and_run(app, &info, &name, &url).await;
    *INSTALLING.lock().unwrap() = false;
    let path = result?;
    println!("已启动安装程序 {}，退出本程序", path.display());
    // 先让所有页面恢复原文再退出，安装程序才能覆盖文件
    crate::immersive::shutdown(app).await;
    app.exit(0);
    Ok(())
}

async fn download_and_run(app: &AppHandle, info: &UpdateInfo, name: &str, url: &str) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    let cfg = config::load().unwrap_or_default();
    let dir = std::env::temp_dir().join("TranslateAnything-update");
    std::fs::create_dir_all(&dir).map_err(|e| t!("err.update.save", e = e))?;
    let path = dir.join(name);
    let mut resp = client(&cfg)?
        .get(url)
        .header("User-Agent", "TranslateAnything-updater")
        .send()
        .await
        .map_err(|e| t!("err.update.network", e = e))?;
    if !resp.status().is_success() {
        return Err(t!("err.update.http", status = resp.status()));
    }
    let total = resp.content_length().unwrap_or(info.asset_size);
    if total > MAX_ASSET {
        return Err(t!("err.update.bad_response"));
    }
    let part = path.with_extension("part");
    let mut file = std::fs::File::create(&part).map_err(|e| t!("err.update.save", e = e))?;
    let (mut done, mut last_pct) = (0u64, u64::MAX);
    while let Some(chunk) = resp.chunk().await.map_err(|e| t!("err.update.network", e = e))? {
        done += chunk.len() as u64;
        if done > MAX_ASSET {
            return Err(t!("err.update.bad_response"));
        }
        file.write_all(&chunk).map_err(|e| t!("err.update.save", e = e))?;
        let pct = if total > 0 { done * 100 / total } else { 0 };
        if pct != last_pct {
            last_pct = pct;
            let _ = app.emit("update-progress", pct);
        }
    }
    drop(file);
    if info.asset_size > 0 && done != info.asset_size {
        let _ = std::fs::remove_file(&part);
        return Err(t!("err.update.incomplete"));
    }
    std::fs::rename(&part, &path).map_err(|e| t!("err.update.save", e = e))?;
    match std::process::Command::new(&path).spawn() {
        Ok(_) => {}
        // 需要管理员权限（ERROR_ELEVATION_REQUIRED）：交给外壳启动，由系统弹出授权提示
        Err(e) if e.raw_os_error() == Some(740) => crate::dialog::open_url(&path.display().to_string())?,
        Err(e) => return Err(t!("err.update.run", e = e)),
    }
    Ok(path)
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
        let v = json!({
            "tag_name": "v0.3.0",
            "html_url": "https://github.com/Richard2091/TranslateAnything/releases/tag/v0.3.0",
            "published_at": "2026-11-20T00:00:00Z",
            "assets": [
                { "name": "Translate.Anything_0.3.0_x64_en-US.msi", "browser_download_url": "https://github.com/a/msi", "size": 1 },
                { "name": "Translate.Anything_0.3.0_x64-setup.exe", "browser_download_url": "https://github.com/a/x64", "size": 2 },
                { "name": "Translate.Anything_0.3.0_arm64-setup.exe", "browser_download_url": "https://github.com/a/arm", "size": 3 },
                { "name": "TranslateAnything_0.3.0_x64_portable.zip", "browser_download_url": "https://github.com/a/zip", "size": 4 },
            ]
        });
        let u = parse_release(&v, "0.2.0", "x64", false).unwrap();
        assert!(u.newer);
        assert_eq!(u.latest, "0.3.0");
        assert_eq!(u.asset_url.as_deref(), Some("https://github.com/a/x64"));
        let u = parse_release(&v, "0.2.0", "arm64", false).unwrap();
        assert_eq!(u.asset_size, 3);
        let u = parse_release(&v, "0.3.0", "x64", true).unwrap();
        assert!(!u.newer);
        assert_eq!(u.asset_url.as_deref(), Some("https://github.com/a/zip"));
    }
}
