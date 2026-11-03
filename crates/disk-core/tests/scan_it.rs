//! 扫描相关的集成测试：真实临时目录 → 扫描 → 校验聚合结果。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use disk_core::{scan, ScanError, ScanProgress};

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// 每个测试独占一个临时目录，断言失败时也不会互相影响。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("disk-core-it-{tag}-{}-{seq}", std::process::id()));
        fs::create_dir_all(&path).expect("创建临时目录");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn scans_nested_directory_and_aggregates_sizes() {
    let temp = TempDir::new("scan");
    fs::write(temp.path().join("a.bin"), vec![0u8; 100]).expect("写入文件");
    let sub = temp.path().join("sub");
    fs::create_dir_all(&sub).expect("创建子目录");
    fs::write(sub.join("b.bin"), vec![0u8; 200]).expect("写入子目录文件");
    fs::create_dir_all(temp.path().join("empty")).expect("创建空目录");

    let progress = Arc::new(ScanProgress::new());
    let tree = scan(temp.path(), progress.clone()).expect("扫描应成功");

    assert_eq!(tree.root().files, 2);
    assert_eq!(tree.root().size, 300);
    assert_eq!(tree.total_size(), 300);
    assert_eq!(tree.nodes.len(), 5, "root + a.bin + sub + b.bin + empty");

    let snapshot = progress.snapshot();
    assert_eq!(snapshot.files, 2);
    assert_eq!(snapshot.bytes, 300);
    assert_eq!(snapshot.errors, 0);
}

#[test]
fn reports_missing_path_as_io_error() {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let missing =
        std::env::temp_dir().join(format!("disk-core-it-missing-{}-{seq}", std::process::id()));
    let progress = Arc::new(ScanProgress::new());
    let error = scan(&missing, progress).expect_err("不存在的目录应返回错误");
    assert!(matches!(error, ScanError::Io(_)), "实际错误：{error:?}");
}

#[test]
fn rejects_regular_file_as_scan_root() {
    let temp = TempDir::new("file-root");
    let file = temp.path().join("single.txt");
    fs::write(&file, b"x").expect("写入文件");

    let progress = Arc::new(ScanProgress::new());
    let error = scan(&file, progress).expect_err("文件不能作为扫描根");
    assert!(
        matches!(error, ScanError::NotADirectory(_)),
        "实际错误：{error:?}"
    );
}

#[test]
fn returns_cancelled_when_requested_before_scan() {
    let temp = TempDir::new("cancel");
    fs::write(temp.path().join("a.bin"), vec![0u8; 10]).expect("写入文件");

    let progress = Arc::new(ScanProgress::new());
    progress.request_cancel();
    let error = scan(temp.path(), progress).expect_err("已取消的扫描应返回错误");
    assert!(matches!(error, ScanError::Cancelled), "实际错误：{error:?}");
}
