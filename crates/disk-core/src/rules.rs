//! 清理候选规则：从已扫描的目录树里挑出「可以人工确认后清理」的项。
//!
//! 这里只做筛选和打标签，不做任何删除判断。每一条候选都必须由用户在界面上
//! 看过路径与大小后确认，规则本身不保证内容可以安全删除。

use std::path::Path;

use crate::delete::is_protected;
use crate::model::{NodeKind, Tree, NO_PARENT};

/// 临时/中间产物扩展名。
const TEMPORARY_EXTENSIONS: &[&str] = &[
    "tmp",
    "temp",
    "log",
    "bak",
    "old",
    "dmp",
    "chk",
    "crdownload",
    "part",
    "partial",
];

/// 安装包扩展名（无论文件名如何都算）。
const PACKAGE_EXTENSIONS: &[&str] = &["msi", "iso"];

/// 需要结合文件名判断的安装包关键字。
const INSTALLER_KEYWORDS: &[&str] = &["setup", "install", "installer", "update", "patch"];

#[derive(Debug, Clone)]
pub struct RuleOptions {
    /// 临时文件、日志、备份、下载残留等。
    pub temporary: bool,
    /// 安装包（msi / iso / 名字带 setup、install 的 exe）。
    pub installers: bool,
    /// 不含任何文件的空目录。
    pub empty_dirs: bool,
    /// 超过阈值的大文件。
    pub large_files: bool,
    pub large_threshold: u64,
}

impl Default for RuleOptions {
    fn default() -> Self {
        Self {
            temporary: true,
            installers: true,
            empty_dirs: true,
            // 大文件不是垃圾，默认不勾选，避免误删素材和工程文件。
            large_files: false,
            large_threshold: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: u32,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    /// 命中原因，直接展示给用户。
    pub reason: &'static str,
    /// 命中安全保护时的原因，非空表示界面上必须禁止勾选。
    pub protected: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct CandidateSet {
    pub items: Vec<Candidate>,
    pub total_size: u64,
    /// 因落在系统保护范围而被排除的项数。
    pub protected_count: usize,
    /// 因超出上限而未纳入的项数。
    pub truncated: usize,
}

/// 按规则收集候选。结果按体积从大到小排序，最多返回 `limit` 项。
pub fn collect(tree: &Tree, options: &RuleOptions, limit: usize) -> CandidateSet {
    let mut items: Vec<Candidate> = Vec::new();
    let mut protected_count = 0usize;

    for id in 1..tree.nodes.len() as u32 {
        let node = tree.node(id);
        if node.parent == NO_PARENT {
            continue;
        }
        let Some(reason) = match_reason(node.kind, &node.name, node.size, node.files, options)
        else {
            continue;
        };
        let path = tree.path_of(id);
        let protected = is_protected(Path::new(&path));
        if protected.is_some() {
            protected_count += 1;
        }
        items.push(Candidate {
            id,
            path,
            size: node.size,
            is_dir: node.kind == NodeKind::Dir,
            reason,
            protected,
        });
    }

    items.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));

    let truncated = items.len().saturating_sub(limit);
    items.truncate(limit);
    let total_size = items.iter().map(|item| item.size).sum();
    CandidateSet {
        items,
        total_size,
        protected_count,
        truncated,
    }
}

fn match_reason(
    kind: NodeKind,
    name: &str,
    size: u64,
    files: u64,
    options: &RuleOptions,
) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    let extension = lower.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");

    if options.empty_dirs && kind == NodeKind::Dir && files == 0 && size == 0 {
        return Some("空目录");
    }
    if kind != NodeKind::File {
        return None;
    }
    if options.temporary && (lower.starts_with("~$") || TEMPORARY_EXTENSIONS.contains(&extension)) {
        return Some("临时/备份类文件");
    }
    if options.installers && is_installer(&lower, extension) {
        return Some("安装包");
    }
    if options.large_files && size >= options.large_threshold {
        return Some("超过阈值的大文件");
    }
    None
}

fn is_installer(lower: &str, extension: &str) -> bool {
    if PACKAGE_EXTENSIONS.contains(&extension) {
        return true;
    }
    extension == "exe" && INSTALLER_KEYWORDS.iter().any(|word| lower.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeKind;

    fn dir(tree: &mut Tree, parent: u32, name: &str) -> u32 {
        tree.add_node(name.into(), parent, NodeKind::Dir)
    }

    fn file(tree: &mut Tree, parent: u32, name: &str, size: u64) -> u32 {
        let id = tree.add_node(name.into(), parent, NodeKind::File);
        tree.nodes[id as usize].size = size;
        tree.nodes[id as usize].files = 1;
        id
    }

    /// 建一棵小树并手工把子节点大小累加到祖先。
    fn sample(root_path: &str) -> Tree {
        let mut tree = Tree::new(root_path.to_string());
        let root = tree.add_node(root_path.into(), NO_PARENT, NodeKind::Dir);
        let downloads = dir(&mut tree, root, "downloads");
        file(&mut tree, downloads, "setup-chrome.exe", 90);
        file(&mut tree, downloads, "movie.mkv", 5000);
        file(&mut tree, downloads, "note.txt", 10);
        let cache = dir(&mut tree, root, "cache");
        file(&mut tree, cache, "data.tmp", 40);
        file(&mut tree, cache, "app.log", 20);
        dir(&mut tree, root, "empty-folder");
        for i in (1..tree.nodes.len()).rev() {
            let (parent, size, files) = {
                let n = &tree.nodes[i];
                (n.parent, n.size, n.files)
            };
            if parent != NO_PARENT {
                let p = &mut tree.nodes[parent as usize];
                p.size += size;
                p.files += files;
            }
        }
        tree
    }

    fn leaf_names(set: &CandidateSet) -> Vec<String> {
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
    fn matches_installers_and_temporary_files() {
        let tree = sample("C:\\root");
        let set = collect(&tree, &RuleOptions::default(), 100);
        let names = leaf_names(&set);
        assert!(names.contains(&"setup-chrome.exe".to_string()));
        assert!(names.contains(&"data.tmp".to_string()));
        assert!(names.contains(&"app.log".to_string()));
        assert!(!names.contains(&"note.txt".to_string()));
        assert!(!names.contains(&"movie.mkv".to_string()));
    }

    #[test]
    fn matches_empty_directories_only() {
        let tree = sample("C:\\root");
        let set = collect(&tree, &RuleOptions::default(), 100);
        let empties: Vec<&Candidate> = set.items.iter().filter(|item| item.is_dir).collect();
        assert_eq!(empties.len(), 1);
        assert!(empties[0].path.ends_with("empty-folder"));
        assert_eq!(empties[0].reason, "空目录");
    }

    #[test]
    fn respects_rule_switches() {
        let tree = sample("C:\\root");
        let options = RuleOptions {
            temporary: false,
            installers: false,
            empty_dirs: false,
            large_files: false,
            ..RuleOptions::default()
        };
        let set = collect(&tree, &options, 100);
        assert!(set.items.is_empty());
    }

    #[test]
    fn large_file_rule_uses_threshold() {
        let tree = sample("C:\\root");
        let options = RuleOptions {
            temporary: false,
            installers: false,
            empty_dirs: false,
            large_files: true,
            large_threshold: 1000,
        };
        let set = collect(&tree, &options, 100);
        assert_eq!(set.items.len(), 1);
        assert!(set.items[0].path.ends_with("movie.mkv"));
        assert_eq!(set.total_size, 5000);
    }

    #[test]
    fn sorts_by_size_and_truncates() {
        let tree = sample("C:\\root");
        let set = collect(&tree, &RuleOptions::default(), 2);
        assert_eq!(set.items.len(), 2);
        assert!(set.items[0].size >= set.items[1].size);
        assert_eq!(set.truncated, 2);
    }

    #[test]
    fn marks_protected_entries() {
        let tree = sample("C:\\Windows");
        let set = collect(&tree, &RuleOptions::default(), 100);
        assert_eq!(set.items.len(), 4);
        assert_eq!(set.protected_count, 4);
        assert!(set.items.iter().all(|item| item.protected.is_some()));
    }
}
