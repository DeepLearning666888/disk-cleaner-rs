//! 安全删除：只走系统回收站，绝不直接抹除，并拒绝系统关键目录。
//!
//! 主要防线：
//! 1. 磁盘根目录、系统目录、Program Files、ProgramData、用户配置根目录一律拒绝；
//! 2. `$Recycle.Bin`、`System Volume Information` 等特殊目录一律拒绝；
//! 3. 符号链接 / 目录联接拒绝删除，避免顺着链接误删目标；
//! 4. 删除统一交给 `SHFileOperationW(FO_DELETE | FOF_ALLOWUNDO)`，
//!    失败时返回中文原因，绝不降级为永久删除。

use std::path::Path;

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
/// `SHFileOperationW` 只可靠支持 MAX_PATH 以内的路径。
const MAX_PATH_CHARS: usize = 259;

#[derive(Debug, Clone)]
pub enum DeleteError {
    /// 被安全规则拦截。
    Protected(String),
    /// 路径本身不合法或不存在。
    Invalid(String),
    /// 系统调用失败。
    Failed(String),
}

impl std::fmt::Display for DeleteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeleteError::Protected(reason) => write!(f, "{}", reason),
            DeleteError::Invalid(reason) => write!(f, "{}", reason),
            DeleteError::Failed(reason) => write!(f, "{}", reason),
        }
    }
}

impl std::error::Error for DeleteError {}

/// 判断路径是否被安全规则保护；返回 `None` 表示可以删除。
pub fn is_protected(path: &Path) -> Option<String> {
    let normalized = normalize(path);
    if normalized.is_empty() {
        return Some("路径为空".to_string());
    }
    if normalized.len() == 2 && normalized.ends_with(':') {
        return Some("磁盘根目录不能删除".to_string());
    }
    if is_users_root(&normalized) {
        return Some("用户目录总根不能删除".to_string());
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        if normalized == normalize(Path::new(&profile)) {
            return Some("当前用户配置目录不能删除".to_string());
        }
    }

    for (name, label) in [
        ("SystemRoot", "Windows 系统目录"),
        ("ProgramFiles", "Program Files 目录"),
        ("ProgramFiles(x86)", "Program Files 目录"),
        ("ProgramData", "ProgramData 目录"),
    ] {
        if let Ok(value) = std::env::var(name) {
            let prefix = normalize(Path::new(&value));
            if !prefix.is_empty() && (normalized == prefix || under(&normalized, &prefix)) {
                return Some(format!("{}受系统保护，不能删除", label));
            }
        }
    }

    for part in normalized.split('\\') {
        if part == "$recycle.bin" || part == "system volume information" {
            return Some("系统特殊目录不能删除".to_string());
        }
    }
    None
}

/// 把路径移入回收站。成功返回时目标已不在原位置。
pub fn recycle(path: &Path) -> Result<(), DeleteError> {
    if let Some(reason) = is_protected(path) {
        return Err(DeleteError::Protected(reason));
    }
    let meta = std::fs::symlink_metadata(path)
        .map_err(|error| DeleteError::Invalid(format!("无法访问该路径：{}", error)))?;
    if is_reparse_point(&meta) {
        return Err(DeleteError::Protected(
            "符号链接或目录联接不能删除".to_string(),
        ));
    }
    let absolute = std::path::absolute(path)
        .map_err(|error| DeleteError::Invalid(format!("无法解析路径：{}", error)))?;

    let wide: Vec<u16> = {
        let mut units = wide_chars(&absolute);
        if units.len() > MAX_PATH_CHARS {
            return Err(DeleteError::Invalid(
                "路径超过 259 个字符，暂不支持移入回收站".to_string(),
            ));
        }
        units.push(0);
        units.push(0);
        units
    };

    let result = shell_delete(&wide);
    match result {
        Ok(true) => {
            if std::fs::symlink_metadata(path).is_ok() {
                return Err(DeleteError::Failed(
                    "系统未移除该项目，可能被其他程序占用".to_string(),
                ));
            }
            Ok(())
        }
        Ok(false) => Err(DeleteError::Failed("操作被取消".to_string())),
        Err(code) => Err(DeleteError::Failed(format!(
            "系统返回错误码 0x{:04X}",
            code
        ))),
    }
}

fn normalize(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('/', "\\");
    let trimmed = raw.trim_end_matches('\\');
    trimmed.to_ascii_lowercase()
}

fn under(path: &str, prefix: &str) -> bool {
    path.len() > prefix.len() && path.starts_with(prefix) && path.as_bytes()[prefix.len()] == b'\\'
}

/// `C:\Users` 或 `D:\Users` 这类用户总根。
fn is_users_root(normalized: &str) -> bool {
    let mut parts = normalized.split('\\');
    let drive = parts.next().unwrap_or("");
    let first = parts.next().unwrap_or("");
    parts.next().is_none() && drive.len() == 2 && drive.ends_with(':') && first == "users"
}

#[cfg(windows)]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || meta.file_type().is_symlink()
}

#[cfg(not(windows))]
fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn wide_chars(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().collect()
}

#[cfg(not(windows))]
fn wide_chars(_path: &Path) -> Vec<u16> {
    Vec::new()
}

#[cfg(windows)]
fn shell_delete(wide: &[u16]) -> Result<bool, i32> {
    use std::ffi::c_void;
    use std::ptr;

    #[repr(C)]
    #[allow(dead_code)] // 字段由 Win32 按布局读取，Rust 侧只写不读。
    struct ShFileOpStructW {
        hwnd: *mut c_void,
        w_func: u32,
        p_from: *const u16,
        p_to: *const u16,
        f_flags: u16,
        f_any_operations_aborted: i32,
        name_mappings: *mut c_void,
        progress_title: *const u16,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(op: *mut ShFileOpStructW) -> i32;
    }

    const FO_DELETE: u32 = 0x0003;
    const FOF_SILENT: u16 = 0x0004;
    const FOF_NOCONFIRMATION: u16 = 0x0010;
    const FOF_ALLOWUNDO: u16 = 0x0040;
    const FOF_NOCONFIRMMKDIR: u16 = 0x0200;
    const FOF_NOERRORUI: u16 = 0x0400;

    let mut op = ShFileOpStructW {
        hwnd: ptr::null_mut(),
        w_func: FO_DELETE,
        p_from: wide.as_ptr(),
        p_to: ptr::null(),
        f_flags: FOF_ALLOWUNDO
            | FOF_NOCONFIRMATION
            | FOF_NOCONFIRMMKDIR
            | FOF_NOERRORUI
            | FOF_SILENT,
        f_any_operations_aborted: 0,
        name_mappings: ptr::null_mut(),
        progress_title: ptr::null(),
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 {
        return Err(code);
    }
    Ok(op.f_any_operations_aborted == 0)
}

#[cfg(not(windows))]
fn shell_delete(_wide: &[u16]) -> Result<bool, i32> {
    Err(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rejects_drive_roots_and_user_roots() {
        assert!(is_protected(&PathBuf::from("C:\\")).is_some());
        assert!(is_protected(&PathBuf::from("D:/")).is_some());
        assert!(is_protected(&PathBuf::from("C:\\Users")).is_some());
        assert!(is_protected(&PathBuf::from("c:\\users\\")).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn rejects_system_directories_but_allows_normal_children() {
        assert!(is_protected(&PathBuf::from("C:\\Windows")).is_some());
        assert!(is_protected(&PathBuf::from("C:\\Windows\\Temp\\a.log")).is_some());
        assert!(is_protected(&PathBuf::from("C:\\Program Files\\app\\a.dll")).is_some());
        assert!(is_protected(&PathBuf::from("C:\\ProgramData\\x")).is_some());
        assert!(is_protected(&PathBuf::from("C:\\Users\\me\\Downloads\\a.zip")).is_none());
        assert!(is_protected(&PathBuf::from("D:\\资料\\旧照片")).is_none());
    }

    #[test]
    fn rejects_recycle_bin_and_volume_metadata() {
        assert!(is_protected(&PathBuf::from("C:\\$Recycle.Bin\\S-1-5-21\\$R1.mp4")).is_some());
        assert!(is_protected(&PathBuf::from("D:\\System Volume Information")).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn recycle_refuses_protected_paths() {
        let error = recycle(Path::new("C:\\Windows")).unwrap_err();
        assert!(matches!(error, DeleteError::Protected(_)));
    }

    #[cfg(windows)]
    #[test]
    fn recycle_moves_file_into_recycle_bin() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".testtmp")
            .join(format!(
                "disk_core_recycle_{}_{}",
                std::process::id(),
                nanos
            ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("junk.txt");
        std::fs::write(&file, b"junk").unwrap();

        recycle(&file).unwrap();
        assert!(!file.exists(), "回收站操作后原文件应已消失");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn recycle_rejects_missing_path() {
        let error = recycle(Path::new("C:\\__definitely_missing_dir_9f3a\\x.txt")).unwrap_err();
        assert!(matches!(error, DeleteError::Invalid(_)));
    }
}
