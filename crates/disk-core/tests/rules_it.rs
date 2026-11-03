//! 扫描 + 规则筛选的端到端集成测试。

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use disk_core::{collect_candidates, scan, RuleOptions, ScanProgress};

static SEQ: AtomicUsize = AtomicUsize::new(0);

struct TempDir(std::path::PathBuf);

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

fn file_names(set: &disk_core::CandidateSet) -> Vec<String> {
    set.items
        .iter()
        .map(|item| {
            Path::new(&item.path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default()
        })
        .collect()
}

#[test]
fn collects_candidates_from_scanned_tree() {
    let temp = TempDir::new("rules");
    fs::write(temp.path().join("app.log"), vec![0u8; 10]).expect("写入日志");
    fs::write(temp.path().join("data.tmp"), vec![0u8; 20]).expect("写入临时文件");
    fs::write(temp.path().join("setup-tool.exe"), vec![0u8; 30]).expect("写入安装包");
    fs::write(temp.path().join("keep.txt"), vec![0u8; 5]).expect("写入普通文件");
    fs::create_dir_all(temp.path().join("empty")).expect("创建空目录");

    let tree = scan(temp.path(), Arc::new(ScanProgress::new())).expect("扫描应成功");
    let set = collect_candidates(&tree, &RuleOptions::default(), 100);
    let names = file_names(&set);

    assert_eq!(set.items.len(), 4, "实际候选：{names:?}");
    assert!(names.contains(&"app.log".to_string()));
    assert!(names.contains(&"data.tmp".to_string()));
    assert!(names.contains(&"setup-tool.exe".to_string()));
    assert!(names.contains(&"empty".to_string()));
    assert!(!names.contains(&"keep.txt".to_string()));

    assert_eq!(set.protected_count, 0);
    assert!(set.items.iter().all(|item| item.protected.is_none()));
    assert_eq!(set.total_size, 60, "30 + 20 + 10 + 0");
}

#[test]
fn disabled_rules_yield_no_candidates() {
    let temp = TempDir::new("rules-off");
    fs::write(temp.path().join("app.log"), vec![0u8; 10]).expect("写入日志");

    let tree = scan(temp.path(), Arc::new(ScanProgress::new())).expect("扫描应成功");
    let options = RuleOptions {
        temporary: false,
        installers: false,
        empty_dirs: false,
        large_files: false,
        ..RuleOptions::default()
    };
    let set = collect_candidates(&tree, &options, 100);
    assert!(set.items.is_empty());
}
