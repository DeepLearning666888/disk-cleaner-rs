//! 递归扫描：构建目录树并聚合大小。
//!
//! 只读实现，不跟随重解析点（符号链接 / 目录联接），避免循环。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::model::{NodeKind, Tree, NO_PARENT};

const MAX_STORED_ERRORS: usize = 100;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[derive(Debug, Clone)]
pub enum ScanError {
    NotADirectory(String),
    Cancelled,
    Io(String),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::NotADirectory(path) => write!(f, "不是文件夹：{}", path),
            ScanError::Cancelled => write!(f, "扫描已取消"),
            ScanError::Io(message) => write!(f, "{}", message),
        }
    }
}

impl std::error::Error for ScanError {}

/// 扫描进度，供界面线程无锁读取。
#[derive(Debug)]
pub struct ScanProgress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub skipped: AtomicU64,
    pub cancel: AtomicBool,
    pub current: Mutex<String>,
}

impl ScanProgress {
    pub fn new() -> Self {
        Self {
            files: AtomicU64::new(0),
            dirs: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            current: Mutex::new(String::new()),
        }
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            files: self.files.load(Ordering::Relaxed),
            dirs: self.dirs.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
            current: self
                .current
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|e| e.into_inner().clone()),
        }
    }
}

impl Default for ScanProgress {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProgressSnapshot {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub errors: u64,
    pub skipped: u64,
    pub current: String,
}

/// 扫描指定目录。取消时返回 [`ScanError::Cancelled`]。
pub fn scan(root: &Path, progress: Arc<ScanProgress>) -> Result<Tree, ScanError> {
    let started = Instant::now();
    let meta = fs::metadata(root)
        .map_err(|e| ScanError::Io(format!("无法访问 {}：{}", root.display(), e)))?;
    if !meta.is_dir() {
        return Err(ScanError::NotADirectory(root.display().to_string()));
    }

    let root_display = normalize_root(root);
    let mut tree = Tree::new(root_display.clone());
    let root_id = tree.add_node(root_display.into_boxed_str(), NO_PARENT, NodeKind::Dir);
    progress.dirs.fetch_add(1, Ordering::Relaxed);

    let mut stack: Vec<(PathBuf, u32)> = vec![(root.to_path_buf(), root_id)];

    while let Some((dir_path, dir_id)) = stack.pop() {
        if progress.is_cancelled() {
            return Err(ScanError::Cancelled);
        }
        if let Ok(mut current) = progress.current.lock() {
            current.clear();
            current.push_str(&dir_path.display().to_string());
        }

        let entries = match fs::read_dir(&dir_path) {
            Ok(entries) => entries,
            Err(error) => {
                tree.nodes[dir_id as usize].error = Some(error.to_string().into_boxed_str());
                record_error(&mut tree, &format!("{}：{}", dir_path.display(), error));
                progress.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        for entry in entries {
            if progress.is_cancelled() {
                return Err(ScanError::Cancelled);
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    record_error(&mut tree, &format!("{}：{}", dir_path.display(), error));
                    progress.errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };
            let path = entry.path();
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(error) => {
                    record_error(&mut tree, &format!("{}：{}", path.display(), error));
                    progress.errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };
            if is_reparse_point(&meta) || meta.file_type().is_symlink() {
                progress.skipped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if meta.is_dir() {
                let id = tree.add_node(name.into_boxed_str(), dir_id, NodeKind::Dir);
                progress.dirs.fetch_add(1, Ordering::Relaxed);
                stack.push((path, id));
            } else if meta.is_file() {
                let size = meta.len();
                let id = tree.add_node(name.into_boxed_str(), dir_id, NodeKind::File);
                tree.nodes[id as usize].size = size;
                progress.files.fetch_add(1, Ordering::Relaxed);
                progress.bytes.fetch_add(size, Ordering::Relaxed);
            } else {
                // 设备、管道等特殊条目不计入。
                progress.skipped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    // 子节点编号一定大于父节点，倒序累加即可完成后序遍历。
    for index in (1..tree.nodes.len()).rev() {
        let (parent, size, files) = {
            let node = &tree.nodes[index];
            (node.parent, node.size, node.files)
        };
        let parent_node = &mut tree.nodes[parent as usize];
        parent_node.size = parent_node.size.saturating_add(size);
        parent_node.files = parent_node.files.saturating_add(files);
    }

    tree.skipped_links = progress.skipped.load(Ordering::Relaxed);
    tree.read_error_count = progress.errors.load(Ordering::Relaxed);
    tree.elapsed = started.elapsed();
    Ok(tree)
}

fn record_error(tree: &mut Tree, message: &str) {
    if tree.read_errors.len() < MAX_STORED_ERRORS {
        tree.read_errors.push(message.to_string());
    }
}

/// 去掉末尾多余分隔符，但保留 `C:\` 这类根路径。
fn normalize_root(path: &Path) -> String {
    let raw = path.display().to_string();
    let trimmed = raw.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() || trimmed.ends_with(':') {
        format!("{}\\", trimmed)
    } else {
        trimmed.to_string()
    }
}

#[cfg(windows)]
fn is_reparse_point(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_meta: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_GUARD: Mutex<()> = Mutex::new(());

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".testtmp");
        let path = base.join(format!(
            "disk_core_{}_{}_{}",
            tag,
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn scans_and_aggregates() {
        let _guard = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let root = temp_dir("scan");
        fs::create_dir_all(root.join("alpha").join("nested")).unwrap();
        fs::write(root.join("alpha").join("one.bin"), vec![0u8; 10]).unwrap();
        fs::write(
            root.join("alpha").join("nested").join("two.bin"),
            vec![0u8; 20],
        )
        .unwrap();
        fs::write(root.join("root.txt"), vec![0u8; 5]).unwrap();

        let tree = scan(&root, Arc::new(ScanProgress::new())).unwrap();
        assert_eq!(tree.total_size(), 35);
        assert_eq!(tree.total_files(), 3);
        assert_eq!(tree.dir_count(), 3);
        let alpha = tree.nodes.iter().position(|n| &*n.name == "alpha").unwrap() as u32;
        assert_eq!(tree.node(alpha).size, 30);
        assert_eq!(tree.node(alpha).files, 2);
        assert!(tree.read_error_count == 0);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_files_and_missing_paths() {
        let _guard = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let root = temp_dir("reject");
        let file = root.join("a.txt");
        fs::write(&file, b"x").unwrap();
        assert!(matches!(
            scan(&file, Arc::new(ScanProgress::new())),
            Err(ScanError::NotADirectory(_))
        ));
        let missing = root.join("missing");
        assert!(matches!(
            scan(&missing, Arc::new(ScanProgress::new())),
            Err(ScanError::Io(_))
        ));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cancel_before_start() {
        let _guard = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let root = temp_dir("cancel");
        let progress = Arc::new(ScanProgress::new());
        progress.request_cancel();
        assert!(matches!(scan(&root, progress), Err(ScanError::Cancelled)));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn normalizes_drive_root_display() {
        assert_eq!(normalize_root(Path::new("C:\\")), "C:\\");
        assert_eq!(normalize_root(Path::new("C:\\Users\\a\\")), "C:\\Users\\a");
    }
}
