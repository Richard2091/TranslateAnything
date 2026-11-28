//! 译文本地持久缓存：`~/.translate-anything/translations.db`（SQLite）
//!
//! 翻译过的文字存进本地库，下次遇到相同原文（同一目标语言）直接查库，不再请求翻译引擎。
//! 悬浮窗模式和沉浸模式共用这一份缓存。
//!
//! - 主键为 `(lang, scope, source)`：长句 `scope = ""` 全局复用，短词与"保留原文"条目按应用 `scope = app_id` 隔离防串味；
//! - `apps` 与 `translation_apps` 记录软件名称及共享长句的软件关联，支持按软件筛选与清空；
//! - `manual` 标记用户手工修改过的译文，自动翻译不会覆盖它；
//! - 打不开数据库时（磁盘只读、文件损坏等）静默降级为"不缓存"，不影响翻译本身。

use crate::config::normalize_app_id;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// 单条原文超过这个长度就不缓存（多半是整段正文，复用率低）
const MAX_SOURCE_CHARS: usize = 4000;

/// 数据库文件名
const DB_FILE: &str = "translations.db";

/// 连接状态：第一次用到时才打开；打不开就记成失败，本次运行不再反复尝试（迁移位置后会重置）
enum Slot {
    Unopened,
    Open(Connection),
    Failed,
}

static STORE: Mutex<Slot> = Mutex::new(Slot::Unopened);

/// 判断某条原文是否应按应用隔离存储（短词或模型/引擎判定保留原文的条目按应用隔离，长句全局共享）
pub fn is_app_scoped(source: &str, keep: bool) -> bool {
    if keep {
        return true;
    }
    let s = source.trim();
    if s.is_empty() || s.ends_with(['.', '!', '?', '。', '！', '？']) {
        return false;
    }
    if s.chars().any(crate::filter::is_cjk_char) {
        return s.chars().count() <= 6;
    }
    s.split_whitespace().count() <= 3 && s.chars().count() <= 24
}

/// 译文库所在文件夹：配置了 `store_dir` 用它，否则用数据目录
pub fn db_dir() -> Option<PathBuf> {
    let custom = crate::config::load().map(|c| c.store_dir).unwrap_or_default();
    let custom = custom.trim();
    if custom.is_empty() { crate::config::data_dir() } else { Some(PathBuf::from(custom)) }
}

pub fn db_path() -> Option<PathBuf> {
    db_dir().map(|d| d.join(DB_FILE))
}

/// 数据库文件及其预写日志的总大小（字节）
pub fn db_size() -> u64 {
    let Some(p) = db_path() else { return 0 };
    ["", "-wal"]
        .iter()
        .filter_map(|ext| std::fs::metadata(format!("{}{ext}", p.display())).ok())
        .map(|m| m.len())
        .sum()
}

fn open(path: &PathBuf) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    init(&conn)?;
    Ok(conn)
}

fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;

    let has_table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'translations')",
        [],
        |r| r.get(0),
    )?;
    if has_table {
        let mut stmt = conn.prepare("PRAGMA table_info(translations)")?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        if !cols.iter().any(|c| c == "scope") {
            conn.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE translations RENAME TO translations_old;
                 CREATE TABLE translations (
                     lang       TEXT    NOT NULL,
                     scope      TEXT    NOT NULL DEFAULT '',
                     source     TEXT    NOT NULL,
                     target     TEXT    NOT NULL,
                     engine     TEXT    NOT NULL DEFAULT '',
                     manual     INTEGER NOT NULL DEFAULT 0,
                     keep       INTEGER NOT NULL DEFAULT 0,
                     hits       INTEGER NOT NULL DEFAULT 0,
                     created_at INTEGER NOT NULL,
                     used_at    INTEGER NOT NULL,
                     PRIMARY KEY (lang, scope, source)
                 ) WITHOUT ROWID;
                 INSERT INTO translations (lang, scope, source, target, engine, manual, keep, hits, created_at, used_at)
                 SELECT lang, '', source, target, engine, manual,
                        CASE WHEN trim(target) = trim(source) THEN 1 ELSE 0 END,
                        hits, created_at, used_at
                 FROM translations_old;
                 DROP TABLE translations_old;
                 COMMIT;",
            )?;
        }
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS translations (
             lang       TEXT    NOT NULL,
             scope      TEXT    NOT NULL DEFAULT '',
             source     TEXT    NOT NULL,
             target     TEXT    NOT NULL,
             engine     TEXT    NOT NULL DEFAULT '',
             manual     INTEGER NOT NULL DEFAULT 0,
             keep       INTEGER NOT NULL DEFAULT 0,
             hits       INTEGER NOT NULL DEFAULT 0,
             created_at INTEGER NOT NULL,
             used_at    INTEGER NOT NULL,
             PRIMARY KEY (lang, scope, source)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS idx_translations_used ON translations (lang, used_at DESC);
         CREATE TABLE IF NOT EXISTS apps (
             app_id     TEXT    PRIMARY KEY,
             name       TEXT    NOT NULL,
             updated_at INTEGER NOT NULL
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS translation_apps (
             lang    TEXT    NOT NULL,
             scope   TEXT    NOT NULL,
             source  TEXT    NOT NULL,
             app_id  TEXT    NOT NULL,
             used_at INTEGER NOT NULL,
             PRIMARY KEY (lang, scope, source, app_id)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS idx_translation_apps_app ON translation_apps (lang, app_id, used_at DESC);",
    )
}

/// 在译文库连接上执行 f；库打不开时返回 None
fn with_db<T>(f: impl FnOnce(&mut Connection) -> T) -> Option<T> {
    let mut slot = STORE.lock().unwrap();
    if matches!(*slot, Slot::Unopened) {
        *slot = match db_path() {
            Some(path) => match open(&path) {
                Ok(c) => Slot::Open(c),
                Err(e) => {
                    eprintln!("打开译文缓存失败，本次运行不做持久缓存：{}\n{e}", path.display());
                    Slot::Failed
                }
            },
            None => Slot::Failed,
        };
    }
    match &mut *slot {
        Slot::Open(c) => Some(f(c)),
        _ => None,
    }
}

/// 把译文库搬到新文件夹（空字符串 = 搬回默认的数据目录），成功后写入配置。
pub fn relocate(new_dir: &str) -> Result<PathBuf, String> {
    let mut cfg = crate::config::load()?;
    let new_dir = new_dir.trim();
    let target_dir = if new_dir.is_empty() {
        crate::config::data_dir().ok_or_else(|| t!("err.config.no_home"))?
    } else {
        PathBuf::from(new_dir)
    };
    let old = db_path().ok_or_else(|| t!("err.store.no_path"))?;
    let new = target_dir.join(DB_FILE);
    if same_file(&old, &new) {
        return Ok(new);
    }
    if new.exists() {
        return Err(t!("err.store.exists", path = new.display()));
    }
    std::fs::create_dir_all(&target_dir).map_err(|e| t!("err.store.mkdir", e = e))?;

    let mut slot = STORE.lock().unwrap();
    if let Slot::Open(c) = &*slot {
        let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
    *slot = Slot::Unopened;
    if old.exists() {
        move_file(&old, &new)?;
        for ext in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", old.display()));
        }
    }
    cfg.store_dir = if new_dir.is_empty() { String::new() } else { target_dir.display().to_string() };
    if let Err(e) = crate::config::save(&cfg) {
        let _ = move_file(&new, &old);
        return Err(e);
    }
    Ok(new)
}

fn same_file(a: &PathBuf, b: &PathBuf) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn move_file(from: &PathBuf, to: &PathBuf) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| t!("err.store.move", e = e))?;
    std::fs::remove_file(from).map_err(|e| t!("err.store.remove_old", e = e))
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn upsert_app(c: &Connection, app_id: &str, app_name: &str, t: i64) -> rusqlite::Result<()> {
    let display = if app_name.trim().is_empty() { app_id } else { app_name.trim() };
    c.execute(
        "INSERT INTO apps (app_id, name, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (app_id) DO UPDATE SET
             name = CASE WHEN excluded.name != excluded.app_id OR apps.name = '' THEN excluded.name ELSE apps.name END,
             updated_at = excluded.updated_at",
        params![app_id, display, t],
    )?;
    Ok(())
}

/// 批量查询（按应用先查专属再回退全局），返回 `原文 -> (译文, 是否保留原文)`
pub fn get_many_for_app(
    lang: &str,
    app_id: &str,
    app_name: &str,
    sources: &[String],
) -> HashMap<String, (String, bool)> {
    with_db(|c| {
        get_many_in(c, lang, app_id, app_name, sources).unwrap_or_else(|e| {
            eprintln!("查询译文缓存失败：{e}");
            HashMap::new()
        })
    })
    .unwrap_or_default()
}

#[allow(dead_code)]
pub fn get_many(lang: &str, sources: &[String]) -> HashMap<String, String> {
    get_many_for_app(lang, "", "", sources)
        .into_iter()
        .map(|(k, (v, _))| (k, v))
        .collect()
}

fn get_many_in(
    c: &Connection,
    lang: &str,
    app_id: &str,
    app_name: &str,
    sources: &[String],
) -> rusqlite::Result<HashMap<String, (String, bool)>> {
    let mut out = HashMap::new();
    let norm = normalize_app_id(app_id);
    let t = now();
    if !norm.is_empty() {
        upsert_app(c, &norm, app_name, t)?;
    }
    let mut get = c.prepare_cached("SELECT target, keep FROM translations WHERE lang = ?1 AND scope = ?2 AND source = ?3")?;
    let mut touch =
        c.prepare_cached("UPDATE translations SET hits = hits + 1, used_at = ?4 WHERE lang = ?1 AND scope = ?2 AND source = ?3")?;
    let mut link = c.prepare_cached(
        "INSERT INTO translation_apps (lang, scope, source, app_id, used_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (lang, scope, source, app_id) DO UPDATE SET used_at = excluded.used_at",
    )?;
    for s in sources {
        if out.contains_key(s) {
            continue;
        }
        let mut hit: Option<(&str, String, bool)> = None;
        if !norm.is_empty() {
            if let Some((tr, k)) = get
                .query_row(params![lang, &norm, s], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)))
                .optional()?
            {
                hit = Some((norm.as_str(), tr, k));
            }
        }
        if hit.is_none() {
            if let Some((tr, k)) = get
                .query_row(params![lang, "", s], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)))
                .optional()?
            {
                hit = Some(("", tr, k));
            }
        }
        if let Some((matched_scope, target, keep)) = hit {
            touch.execute(params![lang, matched_scope, s, t])?;
            if !norm.is_empty() {
                link.execute(params![lang, matched_scope, s, &norm, t])?;
            }
            out.insert(s.clone(), (target, keep));
        }
    }
    Ok(out)
}

/// 批量写入自动翻译结果（短词与 keep 条目按应用隔离，长句全局共享并登记关联软件）
pub fn put_many_for_app(lang: &str, app_id: &str, app_name: &str, items: &[(String, String, bool)], engine: &str) {
    with_db(|c| {
        if let Err(e) = put_many_in(c, lang, app_id, app_name, items, engine) {
            eprintln!("写入译文缓存失败：{e}");
        }
    });
}

#[allow(dead_code)]
pub fn put_many(lang: &str, pairs: &[(String, String)], engine: &str) {
    let items: Vec<(String, String, bool)> = pairs
        .iter()
        .map(|(s, tr)| (s.clone(), tr.clone(), s.trim() == tr.trim()))
        .collect();
    put_many_for_app(lang, "", "", &items, engine);
}

fn put_many_in(
    c: &mut Connection,
    lang: &str,
    app_id: &str,
    app_name: &str,
    items: &[(String, String, bool)],
    engine: &str,
) -> rusqlite::Result<()> {
    let norm = normalize_app_id(app_id);
    let tx = c.transaction()?;
    let t = now();
    if !norm.is_empty() {
        upsert_app(&tx, &norm, app_name, t)?;
    }
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO translations (lang, scope, source, target, engine, keep, created_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT (lang, scope, source) DO UPDATE
                 SET target = excluded.target, engine = excluded.engine, keep = excluded.keep, used_at = excluded.used_at
                 WHERE manual = 0",
        )?;
        let mut link = tx.prepare_cached(
            "INSERT INTO translation_apps (lang, scope, source, app_id, used_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (lang, scope, source, app_id) DO UPDATE SET used_at = excluded.used_at",
        )?;
        for (s, tr, keep) in items {
            if s.chars().count() > MAX_SOURCE_CHARS || tr.trim().is_empty() {
                continue;
            }
            let scope = if !norm.is_empty() && is_app_scoped(s, *keep) {
                norm.as_str()
            } else {
                ""
            };
            stmt.execute(params![lang, scope, s, tr, engine, *keep as i64, t])?;
            if !norm.is_empty() {
                link.execute(params![lang, scope, s, &norm, t])?;
            }
        }
    }
    tx.commit()
}

/// 最近常用的若干条译文，供沉浸模式注入页面时预加载（同原文应用专属优先于全局，保留原文条目不放入译文预加载）
pub fn recent_for_app(lang: &str, app_id: &str, limit: usize) -> Vec<(String, String)> {
    let norm = normalize_app_id(app_id);
    with_db(|c| {
        recent_in(c, lang, &norm, limit).unwrap_or_default()
    })
    .unwrap_or_default()
}

fn recent_in(c: &Connection, lang: &str, norm_app: &str, limit: usize) -> rusqlite::Result<Vec<(String, String)>> {
    let mut map: HashMap<String, String> = HashMap::new();
    let mut global = c.prepare_cached(
        "SELECT source, target FROM translations WHERE lang = ?1 AND scope = '' AND keep = 0 ORDER BY used_at DESC LIMIT ?2",
    )?;
    let rows = global.query_map(params![lang, limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (s, tr) = row?;
        map.insert(s, tr);
    }
    if !norm_app.is_empty() {
        let mut scoped = c.prepare_cached(
            "SELECT source, target FROM translations WHERE lang = ?1 AND scope = ?2 AND keep = 0 ORDER BY used_at DESC LIMIT ?3",
        )?;
        let rows = scoped.query_map(params![lang, norm_app, limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (s, tr) = row?;
            map.insert(s, tr);
        }
    }
    Ok(map.into_iter().collect())
}

// ───────────────────────── 管理（设置界面用） ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AppSummary {
    /// 空字符串表示“未分类”
    pub id: String,
    pub name: String,
    pub count: i64,
}

/// 译文库里的一条记录
#[derive(Debug, Clone, serde::Serialize)]
pub struct Entry {
    pub lang: String,
    pub scope: String,
    pub app_name: String,
    pub source: String,
    pub target: String,
    pub engine: String,
    /// 用户手工改过（自动翻译不会覆盖）
    pub manual: bool,
    /// 保留原文
    pub keep: bool,
    pub hits: i64,
    pub used_at: i64,
    /// 归属的软件（见 entry_apps）
    pub apps: Vec<AppRef>,
}

/// 译文条目归属的软件
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, serde::Serialize)]
pub struct Page {
    pub total: i64,
    pub rows: Vec<Entry>,
}

fn with_conn<T>(f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    with_db(f).ok_or_else(|| t!("err.store.open"))?.map_err(|e| t!("err.store.op", e = e))
}

/// 列出当前目标语言已记录的软件清单（含数量；若有未分类数据则附一条 `id = ""`）
pub fn list_apps(lang: &str) -> Result<Vec<AppSummary>, String> {
    with_conn(|c| list_apps_in(c, lang))
}

fn list_apps_in(c: &Connection, lang: &str) -> rusqlite::Result<Vec<AppSummary>> {
    let mut stmt = c.prepare(
        "SELECT u.app_id, COALESCE(NULLIF(ap.name, ''), u.app_id) AS name, COUNT(*) AS cnt
         FROM (
             SELECT scope AS app_id, scope, source FROM translations WHERE lang = ?1 AND scope != ''
             UNION
             SELECT app_id, scope, source FROM translation_apps WHERE lang = ?1
         ) u
         JOIN translations t ON t.lang = ?1 AND t.scope = u.scope AND t.source = u.source
         LEFT JOIN apps ap ON ap.app_id = u.app_id
         GROUP BY u.app_id
         ORDER BY cnt DESC, name ASC",
    )?;
    let mut out = stmt
        .query_map(params![lang], |r| {
            Ok(AppSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                count: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let unclassified: i64 = c.query_row(
        "SELECT COUNT(*) FROM translations t
         WHERE t.lang = ?1 AND t.scope = ''
           AND NOT EXISTS (SELECT 1 FROM translation_apps ta WHERE ta.lang = t.lang AND ta.scope = t.scope AND ta.source = t.source)",
        params![lang],
        |r| r.get(0),
    )?;
    if unclassified > 0 {
        out.push(AppSummary {
            id: String::new(),
            name: String::new(),
            count: unclassified,
        });
    }
    Ok(out)
}

/// 分页查询：支持按软件筛选（`None` 全部，`Some("")` 未分类，`Some(app_id)` 指定软件）
/// 译文库列表的筛选条件；`None` 表示不限
#[derive(Debug, Default, Clone, Copy)]
pub struct Filter<'a> {
    pub keyword: &'a str,
    /// `Some(true)` 只看手工译文，`Some(false)` 只看自动翻译
    pub manual: Option<bool>,
    /// `Some(true)` 只看保留原文，`Some(false)` 排除保留原文
    pub keep: Option<bool>,
    /// 按软件筛选；`Some("")` 为未分类
    pub app: Option<&'a str>,
}

/// 三态条件转成 SQL 参数：-1 不限，0 / 1 对应取值
fn tri(v: Option<bool>) -> i64 {
    v.map_or(-1, i64::from)
}

pub fn query(lang: &str, filter: &Filter, offset: i64, limit: i64) -> Result<Page, String> {
    with_conn(|c| query_in(c, lang, filter, offset, limit))
}

fn query_in(c: &Connection, lang: &str, f: &Filter, offset: i64, limit: i64) -> rusqlite::Result<Page> {
    let kw = format!("%{}%", f.keyword.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let norm_app = f.app.map(normalize_app_id);
    let app_clause = match norm_app.as_deref() {
        None => "?4 = ?4",
        Some("") => {
            "?4 = '' AND t.scope = '' AND NOT EXISTS (SELECT 1 FROM translation_apps ta WHERE ta.lang = t.lang AND ta.scope = t.scope AND ta.source = t.source)"
        }
        Some(_) => {
            "(t.scope = ?4 OR EXISTS (SELECT 1 FROM translation_apps ta WHERE ta.lang = t.lang AND ta.scope = t.scope AND ta.source = t.source AND ta.app_id = ?4))"
        }
    };
    let filter = format!(
        "t.lang = ?1 AND (t.source LIKE ?2 ESCAPE '\\' OR t.target LIKE ?2 ESCAPE '\\') AND (?3 = -1 OR t.manual = ?3) AND {app_clause} AND (?5 = -1 OR t.keep = ?5)"
    );
    let app_param = norm_app.unwrap_or_default();
    let (manual, keep) = (tri(f.manual), tri(f.keep));

    let total: i64 = c.query_row(
        &format!("SELECT COUNT(*) FROM translations t WHERE {filter}"),
        params![lang, kw, manual, app_param, keep],
        |r| r.get(0),
    )?;
    // linked：共享长句被哪些软件用过，拼成 "id\x1fname\x1eid\x1fname…"，下面再拆开
    let mut stmt = c.prepare(&format!(
        "SELECT t.lang, t.scope, COALESCE(NULLIF(ap.name, ''), t.scope) AS app_name,
                t.source, t.target, t.engine, t.manual, t.keep, t.hits, t.used_at,
                (SELECT group_concat(ta.app_id || char(31) || COALESCE(NULLIF(a2.name, ''), ta.app_id), char(30))
                 FROM translation_apps ta LEFT JOIN apps a2 ON a2.app_id = ta.app_id
                 WHERE ta.lang = t.lang AND ta.scope = t.scope AND ta.source = t.source) AS linked
         FROM translations t
         LEFT JOIN apps ap ON ap.app_id = t.scope
         WHERE {filter}
         ORDER BY t.used_at DESC, t.source
         LIMIT ?6 OFFSET ?7"
    ))?;
    let rows = stmt
        .query_map(params![lang, kw, manual, app_param, keep, limit, offset], |r| {
            let scope: String = r.get(1)?;
            let app_name: String = r.get(2)?;
            let linked: Option<String> = r.get(10)?;
            Ok(Entry {
                lang: r.get(0)?,
                apps: entry_apps(&scope, &app_name, linked.as_deref()),
                scope,
                app_name,
                source: r.get(3)?,
                target: r.get(4)?,
                engine: r.get(5)?,
                manual: r.get::<_, i64>(6)? != 0,
                keep: r.get::<_, i64>(7)? != 0,
                hits: r.get(8)?,
                used_at: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Page { total, rows })
}

/// 一条译文归属的软件：按应用隔离的就是它所在的那个，共享长句是用过它的所有软件；都没有即“未分类”
fn entry_apps(scope: &str, app_name: &str, linked: Option<&str>) -> Vec<AppRef> {
    if !scope.is_empty() {
        return vec![AppRef { id: scope.to_string(), name: app_name.to_string() }];
    }
    let mut out: Vec<AppRef> = linked
        .unwrap_or_default()
        .split('\u{1e}')
        .filter_map(|s| s.split_once('\u{1f}'))
        .map(|(id, name)| AppRef { id: id.to_string(), name: name.to_string() })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// 某个目标语言的译文库概况（设置窗口概览页用）
#[derive(Debug, Default, serde::Serialize)]
pub struct Stats {
    pub total: i64,
    pub manual: i64,
    pub hits: i64,
}

pub fn stats(lang: &str) -> Result<Stats, String> {
    with_conn(|c| stats_in(c, lang))
}

fn stats_in(c: &Connection, lang: &str) -> rusqlite::Result<Stats> {
    c.query_row(
        "SELECT COUNT(*), COALESCE(SUM(manual), 0), COALESCE(SUM(hits), 0) FROM translations WHERE lang = ?1",
        params![lang],
        |r| Ok(Stats { total: r.get(0)?, manual: r.get(1)?, hits: r.get(2)? }),
    )
}

/// 手工改译（不存在则新增），标记为手工译文
pub fn set_manual(lang: &str, scope: &str, source: &str, target: &str) -> Result<(), String> {
    let scope = normalize_app_id(scope);
    with_conn(|c| set_manual_in(c, lang, &scope, source, target))
}

fn set_manual_in(c: &Connection, lang: &str, scope: &str, source: &str, target: &str) -> rusqlite::Result<()> {
    let t = now();
    let keep = (target.trim() == source.trim()) as i64;
    c.execute(
        "INSERT INTO translations (lang, scope, source, target, engine, manual, keep, created_at, used_at)
         VALUES (?1, ?2, ?3, ?4, '手工', 1, ?5, ?6, ?6)
         ON CONFLICT (lang, scope, source) DO UPDATE SET target = excluded.target, manual = 1, keep = excluded.keep",
        params![lang, scope, source, target, keep, t],
    )?;
    Ok(())
}

/// 按 `(scope, source)` 删除指定条目并清理关联
pub fn delete_scoped(lang: &str, items: &[(String, String)]) -> Result<usize, String> {
    with_conn(|c| {
        let tx = c.transaction()?;
        let mut n = 0;
        {
            let mut del_tr = tx.prepare("DELETE FROM translations WHERE lang = ?1 AND scope = ?2 AND source = ?3")?;
            let mut del_link = tx.prepare("DELETE FROM translation_apps WHERE lang = ?1 AND scope = ?2 AND source = ?3")?;
            for (scope, src) in items {
                let s = normalize_app_id(scope);
                del_link.execute(params![lang, &s, src])?;
                n += del_tr.execute(params![lang, &s, src])?;
            }
        }
        tx.commit()?;
        Ok(n)
    })
}

/// 删除匹配原文的所有作用域条目（用于从过滤日志一键改为"强制翻译"时清旧记录）
pub fn delete_by_source(lang: &str, sources: &[String]) -> Result<usize, String> {
    with_conn(|c| {
        let tx = c.transaction()?;
        let mut n = 0;
        {
            let mut del_tr = tx.prepare("DELETE FROM translations WHERE lang = ?1 AND source = ?2")?;
            let mut del_link = tx.prepare("DELETE FROM translation_apps WHERE lang = ?1 AND source = ?2")?;
            for s in sources {
                del_link.execute(params![lang, s])?;
                n += del_tr.execute(params![lang, s])?;
            }
        }
        tx.commit()?;
        Ok(n)
    })
}

/// 清空某个目标语言的译文；支持按软件筛选（`None` 全部，`Some("")` 未分类，`Some(app_id)` 指定软件）
pub fn clear_for_app(lang: &str, app_filter: Option<&str>, keep_manual: bool) -> Result<usize, String> {
    with_conn(|c| clear_in(c, lang, app_filter, keep_manual))
}

fn clear_in(
    c: &mut Connection,
    lang: &str,
    app_filter: Option<&str>,
    keep_manual: bool,
) -> rusqlite::Result<usize> {
    let tx = c.transaction()?;
    let km = keep_manual as i64;
    let norm_app = app_filter.map(normalize_app_id);
    let affected = match norm_app.as_deref() {
        None => {
            tx.execute(
                "DELETE FROM translation_apps WHERE lang = ?1 AND (?2 = 0 OR EXISTS (
                    SELECT 1 FROM translations t
                    WHERE t.lang = translation_apps.lang AND t.scope = translation_apps.scope
                      AND t.source = translation_apps.source AND t.manual = 0
                ))",
                params![lang, km],
            )?;
            tx.execute(
                "DELETE FROM translations WHERE lang = ?1 AND (?2 = 0 OR manual = 0)",
                params![lang, km],
            )?
        }
        Some("") => tx.execute(
            "DELETE FROM translations
             WHERE lang = ?1 AND scope = '' AND (?2 = 0 OR manual = 0)
               AND NOT EXISTS (
                   SELECT 1 FROM translation_apps ta
                   WHERE ta.lang = translations.lang AND ta.scope = translations.scope AND ta.source = translations.source
               )",
            params![lang, km],
        )?,
        Some(app_id) => {
            // 1. 删除该软件专属条目（短词 / keep）及其关联
            tx.execute(
                "DELETE FROM translation_apps
                 WHERE lang = ?1 AND scope = ?2 AND (?3 = 0 OR EXISTS (
                     SELECT 1 FROM translations t
                     WHERE t.lang = translation_apps.lang AND t.scope = translation_apps.scope
                       AND t.source = translation_apps.source AND t.manual = 0
                 ))",
                params![lang, app_id, km],
            )?;
            let n_scoped = tx.execute(
                "DELETE FROM translations WHERE lang = ?1 AND scope = ?2 AND (?3 = 0 OR manual = 0)",
                params![lang, app_id, km],
            )?;

            // 2. 先查出本次需解绑的共享长句，避免误删原本无关联的“未分类”长句
            let shared_sources: Vec<String> = {
                let mut stmt = tx.prepare(
                    "SELECT ta.source FROM translation_apps ta
                     JOIN translations t ON t.lang = ta.lang AND t.scope = ta.scope AND t.source = ta.source
                     WHERE ta.lang = ?1 AND ta.scope = '' AND ta.app_id = ?2 AND (?3 = 0 OR t.manual = 0)",
                )?;
                let rows = stmt
                    .query_map(params![lang, app_id, km], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };

            let mut n_unlinked = 0usize;
            {
                let mut del_link = tx.prepare(
                    "DELETE FROM translation_apps WHERE lang = ?1 AND scope = '' AND source = ?2 AND app_id = ?3",
                )?;
                let mut count_left = tx.prepare(
                    "SELECT COUNT(*) FROM translation_apps WHERE lang = ?1 AND scope = '' AND source = ?2",
                )?;
                let mut del_tr = tx.prepare(
                    "DELETE FROM translations WHERE lang = ?1 AND scope = '' AND source = ?2 AND (?3 = 0 OR manual = 0)",
                )?;
                for src in shared_sources {
                    n_unlinked += del_link.execute(params![lang, &src, app_id])?;
                    let left: i64 = count_left.query_row(params![lang, &src], |r| r.get(0))?;
                    if left == 0 {
                        del_tr.execute(params![lang, &src, km])?;
                    }
                }
            }
            n_scoped + n_unlinked
        }
    };
    tx.commit()?;
    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_app(app: Option<&str>) -> Filter<'_> {
        Filter { app, ..Default::default() }
    }

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        c
    }

    #[test]
    fn 写入后命中与按应用隔离() {
        let mut c = mem();
        let long_sent = "Please configure your proxy settings before retrying.";
        // Claude 下把 "View" 译成 "对话视图"，VS Code 下译成 "视图"
        put_many_in(
            &mut c,
            "zh-CN",
            "claude.exe",
            "Claude",
            &[
                ("View".into(), "对话视图".into(), false),
                ("Sonnet".into(), "Sonnet".into(), true),
                (long_sent.into(), "重试前请先配置代理设置。".into(), false),
                ("Empty".into(), " ".into(), false),
            ],
            "openai",
        )
        .unwrap();
        put_many_in(
            &mut c,
            "zh-CN",
            "code.exe",
            "VS Code",
            &[("View".into(), "视图".into(), false)],
            "edge",
        )
        .unwrap();

        let claude_hits = get_many_in(&c, "zh-CN", "claude.exe", "Claude", &["View".into(), "Sonnet".into(), long_sent.into()]).unwrap();
        assert_eq!(claude_hits["View"], ("对话视图".into(), false));
        assert_eq!(claude_hits["Sonnet"], ("Sonnet".into(), true));
        assert_eq!(claude_hits[long_sent].0, "重试前请先配置代理设置。");

        // VS Code 查询时短词命自己的，长句复用全局共享
        let code_hits = get_many_in(&c, "zh-CN", "code.exe", "VS Code", &["View".into(), long_sent.into()]).unwrap();
        assert_eq!(code_hits["View"], ("视图".into(), false));
        assert_eq!(code_hits[long_sent].0, "重试前请先配置代理设置。");

        // 预加载同样按应用专属覆盖全局
        let pre = recent_in(&c, "zh-CN", "claude.exe", 100).unwrap();
        let pre_map: HashMap<String, String> = pre.into_iter().collect();
        assert_eq!(pre_map.get("View").map(String::as_str), Some("对话视图"));
        assert!(!pre_map.contains_key("Sonnet"), "keep 条目不进译文预加载");
    }

    #[test]
    fn 按软件筛选与清空不误伤共享长句() {
        let mut c = mem();
        let long_sent = "Open settings to customize your translation preferences.";
        // 历史无归属条目（未分类）
        put_many_in(&mut c, "zh-CN", "", "", &[("Legacy global sentence.".into(), "旧全局句。".into(), false)], "edge").unwrap();
        // Claude 与 VS Code 共用 long_sent，且各自有专属短词
        put_many_in(
            &mut c,
            "zh-CN",
            "claude.exe",
            "Claude",
            &[("New chat".into(), "新对话".into(), false), (long_sent.into(), "打开设置以自定义翻译偏好。".into(), false)],
            "edge",
        )
        .unwrap();
        get_many_in(&c, "zh-CN", "code.exe", "VS Code", &[long_sent.into()]).unwrap();
        put_many_in(&mut c, "zh-CN", "code.exe", "VS Code", &[("Terminal".into(), "终端".into(), false)], "edge").unwrap();

        let apps = list_apps_in(&c, "zh-CN").unwrap();
        assert_eq!(apps.len(), 3);
        assert_eq!(query_in(&c, "zh-CN", &by_app(Some("claude.exe")), 0, 20).unwrap().total, 2);
        assert_eq!(query_in(&c, "zh-CN", &by_app(Some("")), 0, 20).unwrap().total, 1);

        // 清空 Claude：应删掉 Claude 的专属短词，但 long_sent 因 VS Code 也在用必须保留
        let cleared = clear_in(&mut c, "zh-CN", Some("claude.exe"), true).unwrap();
        assert_eq!(cleared, 2);
        assert_eq!(query_in(&c, "zh-CN", &by_app(Some("claude.exe")), 0, 20).unwrap().total, 0);
        assert_eq!(query_in(&c, "zh-CN", &by_app(Some("code.exe")), 0, 20).unwrap().total, 2);
        assert_eq!(query_in(&c, "zh-CN", &by_app(Some("")), 0, 20).unwrap().total, 1, "未分类不受影响");

        // 再清空 VS Code：此时 long_sent 已无其他软件关联，应连同共享长句一起删除
        clear_in(&mut c, "zh-CN", Some("code.exe"), true).unwrap();
        assert_eq!(query_in(&c, "zh-CN", &by_app(None), 0, 20).unwrap().total, 1);
    }

    #[test]
    fn 旧版数据库自动迁移与保留手工译文() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE translations (
                lang TEXT NOT NULL,
                source TEXT NOT NULL,
                target TEXT NOT NULL,
                engine TEXT NOT NULL DEFAULT '',
                manual INTEGER NOT NULL DEFAULT 0,
                hits INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                used_at INTEGER NOT NULL,
                PRIMARY KEY (lang, source)
            ) WITHOUT ROWID;
            INSERT INTO translations VALUES ('zh-CN', 'Run', '执行', '手工', 1, 5, 100, 200);
            INSERT INTO translations VALUES ('zh-CN', 'Claude', 'Claude', 'edge', 0, 2, 100, 150);",
        )
        .unwrap();
        init(&c).unwrap();

        let page = query_in(&c, "zh-CN", &by_app(Some("")), 0, 10).unwrap();
        assert_eq!(page.total, 2);
        let run = page.rows.iter().find(|r| r.source == "Run").unwrap();
        assert!(run.manual);
        assert!(!run.keep);
        let claude = page.rows.iter().find(|r| r.source == "Claude").unwrap();
        assert!(claude.keep);

        // 更多筛选：来源、保留原文
        let total = |manual, keep| query_in(&c, "zh-CN", &Filter { manual, keep, ..Default::default() }, 0, 10).unwrap().total;
        assert_eq!(total(Some(true), None), 1, "只看手工");
        assert_eq!(total(Some(false), None), 1, "只看自动");
        assert_eq!(total(None, Some(true)), 1, "只看保留原文");
        assert_eq!(total(None, Some(false)), 1, "排除保留原文");
        assert_eq!(total(Some(true), Some(true)), 0);

        // 手工译文不被自动写入覆盖，且 clear(keep_manual=true) 保留
        put_many_in(&mut c, "zh-CN", "", "", &[("Run".into(), "跑".into(), false)], "edge").unwrap();
        assert_eq!(get_many_in(&c, "zh-CN", "", "", &["Run".into()]).unwrap()["Run"].0, "执行");
        clear_in(&mut c, "zh-CN", None, true).unwrap();
        assert_eq!(query_in(&c, "zh-CN", &by_app(None), 0, 10).unwrap().total, 1);
    }
}
