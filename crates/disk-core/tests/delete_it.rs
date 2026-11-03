//! 删除路径的集成测试：只覆盖安全拦截与错误路径，不触发真实回收站操作。

use std::path::Path;

use disk_core::{is_protected, recycle, DeleteError};

#[test]
fn rejects_missing_target_as_invalid() {
    let missing = std::env::temp_dir().join(format!(
        "disk-core-it-missing-delete-{}",
        std::process::id()
    ));
    let error = recycle(&missing).expect_err("不存在的路径应失败");
    assert!(
        matches!(error, DeleteError::Invalid(_)),
        "实际错误：{error:?}"
    );
}

#[test]
fn protects_disk_root() {
    assert!(is_protected(Path::new("C:\\")).is_some());
    let error = recycle(Path::new("C:\\")).expect_err("磁盘根目录必须被拦截");
    assert!(
        matches!(error, DeleteError::Protected(_)),
        "实际错误：{error:?}"
    );
}

#[test]
fn protects_windows_directory_when_env_present() {
    let Ok(system_root) = std::env::var("SystemRoot") else {
        return;
    };
    assert!(
        is_protected(Path::new(&system_root)).is_some(),
        "SystemRoot 应受保护：{system_root}"
    );
    let child = Path::new(&system_root).join("Temp");
    assert!(
        is_protected(&child).is_some(),
        "SystemRoot 子目录应受保护：{}",
        child.display()
    );
}

#[test]
fn allows_regular_temp_path() {
    let allowed = std::env::temp_dir().join("disk-core-it-allowed");
    assert!(
        is_protected(&allowed).is_none(),
        "普通临时路径不应被保护：{}",
        allowed.display()
    );
}

#[test]
fn protects_recycle_bin_paths() {
    assert!(is_protected(Path::new("C:\\$Recycle.Bin")).is_some());
    assert!(is_protected(Path::new("C:\\System Volume Information")).is_some());
}
