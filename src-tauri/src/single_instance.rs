//! 按 Windows 登录会话限制随译只运行一个实例。

use windows::Win32::Foundation::{HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Threading::{
    CreateMutexExW, ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject,
};
use windows::core::{Error, Owned, PCWSTR, Result, w};

// 安装版和便携版共用同一个锁，名称与版本和可执行文件路径无关。
const LOCK_NAME: PCWSTR = w!("Local\\com.richard.translateanything.single-instance");

/// 由获取锁的启动线程持有。Tauri 直接结束进程时，由 Windows 回收锁和句柄。
pub(crate) struct InstanceGuard(Owned<HANDLE>);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // 互斥量由当前线程获取，释放后 Owned 自动关闭句柄。
        let _ = unsafe { ReleaseMutex(*self.0) };
    }
}

/// 获取当前会话的启动锁，已有实例持有时立即返回 `None`。
pub(crate) fn acquire() -> Result<Option<InstanceGuard>> {
    acquire_named(LOCK_NAME)
}

/// 创建或打开命名互斥量，并尝试在当前线程取得所有权。
fn acquire_named(name: PCWSTR) -> Result<Option<InstanceGuard>> {
    // 仅申请等待权限，句柄不会继承给子进程。
    let handle = unsafe {
        Owned::new(CreateMutexExW(
            None,
            name,
            0,
            SYNCHRONIZATION_SYNCHRONIZE.0,
        )?)
    };
    // 零超时等待原子地取得所有权。若原持有者异常退出，也可接管。
    match unsafe { WaitForSingleObject(*handle, 0) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Some(InstanceGuard(handle))),
        WAIT_TIMEOUT => Ok(None),
        _ => Err(Error::from_thread()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证第二个线程立即被拦截，且持有者退出后可以再次获取锁。
    #[test]
    fn 拦截重复启动并允许重新启动() {
        // 使用独立名称，避免和运行中的随译或其他测试进程冲突。
        let name: Vec<u16> = format!(
            "Local\\com.richard.translateanything.single-instance.test.{}",
            std::process::id()
        )
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
        let guard = acquire_named(PCWSTR(name.as_ptr())).unwrap().unwrap();

        // 保留独立句柄，确保释放前后测试的是同一个互斥量。
        let observer = unsafe {
            Owned::new(
                CreateMutexExW(
                    None,
                    PCWSTR(name.as_ptr()),
                    0,
                    SYNCHRONIZATION_SYNCHRONIZE.0,
                )
                .unwrap(),
            )
        };
        // Windows 互斥量允许同一线程重入，前后均在另一个线程尝试获取。
        let try_from_other_thread = || {
            std::thread::scope(|scope| {
                scope
                    .spawn(|| acquire_named(PCWSTR(name.as_ptr())).unwrap().is_some())
                    .join()
                    .unwrap()
            })
        };
        assert!(!try_from_other_thread());
        drop(guard);
        assert!(try_from_other_thread());
        drop(observer);
    }
}
