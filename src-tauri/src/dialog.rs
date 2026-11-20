//! 系统对话框：选择文件夹 / 程序、在资源管理器中定位文件、用默认浏览器打开网址。直接调 Windows 外壳接口，不额外引入插件。

use std::os::windows::process::CommandExt;
use std::path::Path;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::{HSTRING, PCWSTR};

/// 弹出"选择文件夹"对话框（以 `owner` 为父窗口，模态）。用户取消时返回 `Ok(None)`。
/// 会阻塞当前线程直到对话框关闭，请在后台线程调用。
pub fn pick_folder(owner: isize, title: &str, initial: Option<&Path>) -> Result<Option<String>, String> {
    pick(owner, title, initial, None)
}

/// 弹出"选择程序"对话框，只列出 .exe；`initial` 是初始所在的文件夹。用法同 [`pick_folder`]
pub fn pick_exe(owner: isize, title: &str, initial: Option<&Path>) -> Result<Option<String>, String> {
    pick(owner, title, initial, Some(&t!("dialog.exe_filter")))
}

/// `filter` 为 None 时选文件夹，否则选 .exe 文件（`filter` 是文件类型的显示名）
fn pick(owner: isize, title: &str, initial: Option<&Path>, filter: Option<&str>) -> Result<Option<String>, String> {
    unsafe {
        let inited = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let result = (|| -> windows::core::Result<Option<String>> {
            let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            let mode = if filter.is_some() { FOS_FILEMUSTEXIST } else { FOS_PICKFOLDERS };
            dlg.SetOptions(dlg.GetOptions()? | mode | FOS_FORCEFILESYSTEM)?;
            dlg.SetTitle(&HSTRING::from(title))?;
            // 两个 HSTRING 要活到对话框关闭
            let types = filter.map(|f| (HSTRING::from(f), HSTRING::from("*.exe")));
            if let Some((name, spec)) = &types {
                dlg.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(spec.as_ptr()) }])?;
            }
            if let Some(dir) = initial.filter(|d| d.is_dir()) {
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(dir.as_os_str()), None) {
                    let _ = dlg.SetFolder(&item);
                }
            }
            // 用户点取消时 Show 返回错误码，按"未选择"处理
            if dlg.Show(Some(HWND(owner as _))).is_err() {
                return Ok(None);
            }
            let name = dlg.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string();
            CoTaskMemFree(Some(name.0 as _));
            Ok(Some(path?))
        })();
        if inited {
            CoUninitialize();
        }
        result.map_err(|e| t!("err.dialog.pick_folder", e = e))
    }
}

/// 在资源管理器中打开文件所在文件夹并选中它；文件不存在时打开其所在文件夹
pub fn reveal(path: &Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new("explorer");
    if path.exists() {
        // explorer 自己解析参数，路径要整体加引号
        cmd.raw_arg(format!("/select,\"{}\"", path.display()));
    } else {
        let dir = path.parent().filter(|d| d.is_dir()).ok_or_else(|| t!("err.dialog.not_found"))?;
        cmd.arg(dir);
    }
    cmd.spawn().map(|_| ()).map_err(|e| t!("err.dialog.explorer", e = e))
}

/// 交给系统外壳打开：网址用默认浏览器，程序按需弹出管理员授权
pub fn open_url(target: &str) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|e| t!("err.dialog.explorer", e = e))
}
