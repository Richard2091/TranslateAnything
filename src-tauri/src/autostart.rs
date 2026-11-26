//! Windows 当前用户的登录自启动。状态直接从系统读取。

use serde::Serialize;
use std::io;
use std::path::{Path, PathBuf};
use windows_registry::{CURRENT_USER, Type};

const ENTRY: &str = "com.richard.translateanything";
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Disabled,
    Enabled,
    Blocked,
    Stale,
    Portable,
}

/// 注册表库把 Win32 状态包装成 HRESULT，缺键或缺值都视为未设置。
fn optional<T>(result: windows_registry::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if matches!(error.code().0 as u32, 0x8007_0002 | 0x8007_0003) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// 读取与当前程序对应的启动项，再检查 Windows 是否停用了它。
fn read_status(run: &str, approved: &str, command: &str) -> io::Result<Status> {
    let Some(value) = optional(CURRENT_USER.open(run).and_then(|key| key.get_value(ENTRY)))? else {
        return Ok(Status::Disabled);
    };
    if value.ty() != Type::String || !String::try_from(value).is_ok_and(|value| value == command) {
        return Ok(Status::Stale);
    }
    if is_blocked(approved)? {
        Ok(Status::Blocked)
    } else {
        Ok(Status::Enabled)
    }
}

fn is_blocked(approved: &str) -> io::Result<bool> {
    let approval = optional(
        CURRENT_USER
            .open(approved)
            .and_then(|key| key.get_value(ENTRY)),
    )?;
    // Windows 启动应用中常见的停用标志是 03 / 07。
    Ok(approval
        .is_some_and(|value| value.ty() == Type::Bytes && matches!(value.first(), Some(3 | 7))))
}

/// 只删除本应用的值，重复删除仍视为成功。
fn remove_entry(path: &str) -> io::Result<()> {
    optional(
        CURRENT_USER
            .options()
            .write()
            .open(path)
            .and_then(|key| key.remove_value(ENTRY)),
    )?;
    Ok(())
}

/// 用引号保护带空格的路径，不附加打开设置窗口的参数。
fn startup_command(exe: &Path) -> io::Result<String> {
    let exe = exe.to_str().ok_or(io::ErrorKind::InvalidData)?;
    Ok(format!("\"{exe}\""))
}

/// 更新本应用的登录启动项，再回读 Windows 状态。
fn update_entry(run: &str, approved: &str, command: &str, enabled: bool) -> io::Result<Status> {
    if enabled {
        // Windows Run 值的命令行最长为 260 个字符。
        if command.encode_utf16().count() > 260 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let blocked = is_blocked(approved)?;
        CURRENT_USER.create(run)?.set_string(ENTRY, command)?;
        // 只有 Windows 停用了这一项时才需要修改覆盖记录。
        if blocked {
            remove_entry(approved)?;
        }
        read_status(run, approved, command)
    } else {
        remove_entry(run)?;
        // Run 已移除，清理系统留下的显示状态失败也不影响关闭结果。
        if let Err(error) = remove_entry(approved) {
            eprintln!("清理 Windows 自启动状态失败：{error}");
        }
        Ok(Status::Disabled)
    }
}

/// 便携标记存在时禁止操作启动项，即使数据目录已因只读而回退。
fn installed_exe() -> io::Result<Option<PathBuf>> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or(io::ErrorKind::InvalidData)?;
    Ok((!crate::config::has_portable_marker(dir)?).then_some(exe))
}

/// 查询当前程序的自启动状态。
pub fn status() -> io::Result<Status> {
    let Some(exe) = installed_exe()? else {
        return Ok(Status::Portable);
    };
    read_status(RUN, APPROVED, &startup_command(&exe)?)
}

/// 修改当前用户的自启动状态。
pub fn set(enabled: bool) -> io::Result<Status> {
    let Some(exe) = installed_exe()? else {
        return Ok(Status::Portable);
    };
    update_entry(RUN, APPROVED, &startup_command(&exe)?, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 启动命令给含空格的路径加引号() {
        let exe = r"C:\Program Files\随译\Translate Anything.exe";
        assert_eq!(
            startup_command(Path::new(exe)).unwrap(),
            format!("\"{exe}\"")
        );
    }

    #[test]
    fn 在测试键验证登录启动项的写入回读与清理() {
        let path = format!(
            r"Software\com.richard.translateanything.autostart-test-{}",
            std::process::id()
        );
        struct Cleanup<'a>(&'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = CURRENT_USER.remove_tree(self.0);
            }
        }
        let _cleanup = Cleanup(&path);
        let run = format!(r"{path}\Run");
        let approved = format!(r"{path}\Approval");
        let command = r#""C:\Program Files\随译\Translate Anything.exe""#;
        assert_eq!(
            read_status(&run, &approved, command).unwrap(),
            Status::Disabled
        );
        assert_eq!(
            update_entry(&run, &approved, &"a".repeat(261), true)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            update_entry(&run, &approved, command, true).unwrap(),
            Status::Enabled
        );
        let key = CURRENT_USER.open(&run).unwrap();
        assert_eq!(key.get_string(ENTRY).unwrap(), command);
        assert_eq!(
            read_status(&run, &approved, r#""D:\随译.exe""#).unwrap(),
            Status::Stale
        );
        CURRENT_USER
            .create(&run)
            .unwrap()
            .set_u32(ENTRY, 1)
            .unwrap();
        assert_eq!(
            read_status(&run, &approved, command).unwrap(),
            Status::Stale
        );
        assert_eq!(
            update_entry(&run, &approved, command, true).unwrap(),
            Status::Enabled
        );
        let approval = CURRENT_USER.create(&approved).unwrap();
        for flag in [3, 7] {
            approval
                .set_bytes(ENTRY, Type::Bytes, &[flag, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8])
                .unwrap();
            assert_eq!(
                read_status(&run, &approved, command).unwrap(),
                Status::Blocked
            );
            assert_eq!(
                update_entry(&run, &approved, command, true).unwrap(),
                Status::Enabled
            );
        }
        assert_eq!(
            update_entry(&run, &approved, command, false).unwrap(),
            Status::Disabled
        );
        assert_eq!(
            update_entry(&run, &approved, command, false).unwrap(),
            Status::Disabled
        );
        assert!(optional(key.get_type(ENTRY)).unwrap().is_none());
        assert!(optional(approval.get_type(ENTRY)).unwrap().is_none());
    }
}
