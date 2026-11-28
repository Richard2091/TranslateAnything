//! 按引擎实例的全局并发限流与 429 退避控制。
//!
//! 解决多页面、多浮窗和单批分片并发叠加时打爆 AI 供应商速率限制的问题：
//! - 每个引擎实例共享同一把信号量（AI 默认 2，传统引擎默认 4，可配置）；
//! - 遇到 HTTP 429 时解析 `Retry-After` 并退避重试，同时临时把该引擎并发压到 1（30 秒）；
//! - 仅在连续多次失败或硬错误时才标记冷却，避免偶发限流直接踢掉主引擎。

use super::TranslateError;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

const THROTTLE_DURATION: Duration = Duration::from_secs(30);
const RATE_LIMIT_RETRIES: usize = 2;

pub struct EngineGate {
    limit: usize,
    sem: Arc<Semaphore>,
    serial: Arc<tokio::sync::Mutex<()>>,
    throttle_until: Mutex<Option<Instant>>,
    consecutive_failures: AtomicU32,
}

pub struct PermitGuard {
    _sem: OwnedSemaphorePermit,
    _serial: Option<OwnedMutexGuard<()>>,
}

impl EngineGate {
    fn new(limit: usize) -> Self {
        let limit = limit.clamp(1, 16);
        Self {
            limit,
            sem: Arc::new(Semaphore::new(limit)),
            serial: Arc::new(tokio::sync::Mutex::new(())),
            throttle_until: Mutex::new(None),
            consecutive_failures: AtomicU32::new(0),
        }
    }

    /// 获取该引擎的一个并发名额；若正处于 429 降速窗口则先串行化
    pub async fn acquire(&self) -> PermitGuard {
        let throttled = {
            let mut guard = self.throttle_until.lock().unwrap();
            match *guard {
                Some(until) if Instant::now() < until => true,
                Some(_) => {
                    *guard = None;
                    false
                }
                None => false,
            }
        };
        let _serial = if throttled {
            Some(self.serial.clone().lock_owned().await)
        } else {
            None
        };
        let _sem = self.sem.clone().acquire_owned().await.expect("引擎信号量不应关闭");
        PermitGuard { _sem, _serial }
    }

    pub fn on_rate_limited(&self) {
        *self.throttle_until.lock().unwrap() = Some(Instant::now() + THROTTLE_DURATION);
    }

    pub fn on_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    /// 记录一次失败；返回是否应当触发引擎级冷却
    pub fn on_failure(&self, is_rate_limit: bool) -> bool {
        let n = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        !is_rate_limit || n >= 3
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_throttled(&self) -> bool {
        self.throttle_until
            .lock()
            .unwrap()
            .is_some_and(|until| Instant::now() < until)
    }
}

static GATES: OnceLock<Mutex<HashMap<String, Arc<EngineGate>>>> = OnceLock::new();

pub fn gate(id: &str, limit: usize) -> Arc<EngineGate> {
    let limit = limit.clamp(1, 16);
    let mut map = GATES.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if let Some(existing) = map.get(id) {
        if existing.limit == limit {
            return existing.clone();
        }
    }
    let g = Arc::new(EngineGate::new(limit));
    map.insert(id.to_string(), g.clone());
    g
}

/// 解析 HTTP `Retry-After`（整数秒），限制在 1..=15 秒避免卡死交互
pub fn parse_retry_after(header: Option<&str>) -> Option<Duration> {
    let secs = header?.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(secs.clamp(1, 15)))
}

/// 受并发限流与 429 退避保护地执行一次请求
pub async fn run_limited<T, F, Fut>(id: &str, limit: usize, mut op: F) -> Result<T, TranslateError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, TranslateError>>,
{
    let g = gate(id, limit);
    for attempt in 0..=RATE_LIMIT_RETRIES {
        let res = {
            let _permit = g.acquire().await;
            op().await
        };
        match res {
            Ok(v) => {
                g.on_success();
                return Ok(v);
            }
            Err(TranslateError::RateLimited { message, retry_after }) => {
                g.on_rate_limited();
                if attempt < RATE_LIMIT_RETRIES {
                    let wait = retry_after.unwrap_or_else(|| Duration::from_secs(1 << attempt));
                    tokio::time::sleep(wait).await;
                    continue;
                }
                return Err(TranslateError::RateLimited { message, retry_after });
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn 信号量限制最大并发与429降速() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..6 {
            let active = active.clone();
            let peak = peak.clone();
            handles.push(tokio::spawn(async move {
                run_limited("test-sem-cap", 2, || {
                    let active = active.clone();
                    let peak = peak.clone();
                    async move {
                        let cur = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(cur, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok::<_, TranslateError>(())
                    }
                })
                .await
                .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= 2, "并发峰值超出上限：{}", peak.load(Ordering::SeqCst));

        // 模拟一次 429 后退避重试成功并进入临时降速
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_c = attempts.clone();
        let res = run_limited("test-429-retry", 3, move || {
            let attempts = attempts_c.clone();
            async move {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(TranslateError::RateLimited {
                        message: "429".into(),
                        retry_after: Some(Duration::from_millis(10)),
                    })
                } else {
                    Ok("ok")
                }
            }
        })
        .await;
        assert_eq!(res.unwrap(), "ok");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert!(gate("test-429-retry", 3).is_throttled());
        assert_eq!(parse_retry_after(Some("100")), Some(Duration::from_secs(15)));
        assert_eq!(parse_retry_after(Some("0")), Some(Duration::from_secs(1)));
        assert_eq!(parse_retry_after(Some("bad")), None);
    }
}
