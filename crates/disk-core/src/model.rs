//! 目录树数据模型与查询。

use std::time::Duration;

pub const NO_PARENT: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Dir,
    File,
}

impl NodeKind {
    pub fn label(self) -> &'static str {
        match self {
            NodeKind::Dir => "文件夹",
            NodeKind::File => "文件",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: Box<str>,
    pub parent: u32,
    pub kind: NodeKind,
    pub children: Vec<u32>,
    /// 文件为自身逻辑大小；文件夹为子树聚合后的逻辑大小。
    pub size: u64,
    /// 文件为 1；文件夹为子树内文件总数。
    pub files: u64,
    /// 读取该目录失败时的错误信息。
    pub error: Option<Box<str>>,
}

#[derive(Debug, Clone)]
pub struct Tree {
    pub root_path: String,
    pub nodes: Vec<Node>,
    pub skipped_links: u64,
    pub read_error_count: u64,
    pub read_errors: Vec<String>,
    pub elapsed: Duration,
    /// 已从树上摘除的目录数量，用于修正 [`Tree::dir_count`]。
    removed_dirs: u64,
}

impl Tree {
    pub fn new(root_path: String) -> Self {
        Self {
            root_path,
            nodes: Vec::new(),
            skipped_links: 0,
            read_error_count: 0,
            read_errors: Vec::new(),
            elapsed: Duration::ZERO,
            removed_dirs: 0,
        }
    }

    pub fn add_node(&mut self, name: Box<str>, parent: u32, kind: NodeKind) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(Node {
            name,
            parent,
            kind,
            children: Vec::new(),
            size: 0,
            files: if kind == NodeKind::File { 1 } else { 0 },
            error: None,
        });
        if parent != NO_PARENT {
            self.nodes[parent as usize].children.push(id);
        }
        id
    }

    pub fn node(&self, id: u32) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }

    pub fn total_size(&self) -> u64 {
        self.nodes.first().map(|n| n.size).unwrap_or(0)
    }

    pub fn total_files(&self) -> u64 {
        self.nodes.first().map(|n| n.files).unwrap_or(0)
    }

    pub fn dir_count(&self) -> u64 {
        let total = self
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Dir)
            .count() as u64;
        total.saturating_sub(self.removed_dirs)
    }

    /// 把节点连同子树从树上摘除，并把大小/文件数从祖先链扣掉。
    ///
    /// 这里只做内存中的统计修正，真正删除文件由调用方负责。
    pub fn detach(&mut self, id: u32) -> bool {
        if id == 0 || id as usize >= self.nodes.len() {
            return false;
        }
        let (parent, size, files) = {
            let node = &self.nodes[id as usize];
            (node.parent, node.size, node.files)
        };
        if parent == NO_PARENT {
            return false;
        }

        let mut dirs = 0u64;
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            let node = &self.nodes[current as usize];
            if node.kind == NodeKind::Dir {
                dirs += 1;
            }
            stack.extend(node.children.iter().copied());
        }
        self.removed_dirs = self.removed_dirs.saturating_add(dirs);

        if let Some(parent_node) = self.nodes.get_mut(parent as usize) {
            parent_node.children.retain(|child| *child != id);
        }

        let mut cursor = parent;
        loop {
            let node = &mut self.nodes[cursor as usize];
            node.size = node.size.saturating_sub(size);
            node.files = node.files.saturating_sub(files);
            if cursor == 0 {
                break;
            }
            cursor = node.parent;
            if cursor == NO_PARENT {
                break;
            }
        }
        self.nodes[id as usize].parent = NO_PARENT;
        true
    }

    /// 由根到指定节点的路径，用反斜杠拼接。
    pub fn path_of(&self, mut id: u32) -> String {
        if id == 0 {
            return self.root_path.clone();
        }
        let mut parts: Vec<&str> = Vec::new();
        while id != 0 {
            let node = &self.nodes[id as usize];
            parts.push(&node.name);
            id = node.parent;
        }
        let mut out = self.root_path.clone();
        for part in parts.iter().rev() {
            if !out.ends_with('\\') && !out.ends_with('/') {
                out.push('\\');
            }
            out.push_str(part);
        }
        out
    }

    /// 从根到该节点（含该节点）的祖先链。
    pub fn ancestors(&self, mut id: u32) -> Vec<u32> {
        let mut out = vec![id];
        while id != 0 {
            id = self.nodes[id as usize].parent;
            out.push(id);
        }
        out.reverse();
        out
    }

    /// 子节点按大小降序。
    pub fn sorted_children(&self, id: u32) -> Vec<u32> {
        let mut out = self.nodes[id as usize].children.clone();
        out.sort_by(|a, b| {
            self.nodes[*b as usize]
                .size
                .cmp(&self.nodes[*a as usize].size)
                .then_with(|| {
                    self.nodes[*a as usize]
                        .name
                        .cmp(&self.nodes[*b as usize].name)
                })
        });
        out
    }

    /// 子树中最大的文件（含自身为文件的情况）。
    pub fn top_files(&self, id: u32, limit: usize) -> Vec<u32> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            let node = &self.nodes[current as usize];
            match node.kind {
                NodeKind::File => out.push(current),
                NodeKind::Dir => stack.extend(node.children.iter().copied()),
            }
        }
        out.sort_by(|a, b| {
            self.nodes[*b as usize]
                .size
                .cmp(&self.nodes[*a as usize].size)
        });
        out.truncate(limit);
        out
    }

    /// 在当前子树内按名称搜索（不区分大小写），结果按大小降序。
    pub fn search(&self, id: u32, needle: &str, limit: usize) -> Vec<u32> {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            let node = &self.nodes[current as usize];
            if node.name.to_lowercase().contains(&needle) {
                out.push(current);
            }
            if node.kind == NodeKind::Dir {
                stack.extend(node.children.iter().copied());
            }
        }
        out.sort_by(|a, b| {
            self.nodes[*b as usize]
                .size
                .cmp(&self.nodes[*a as usize].size)
        });
        out.truncate(limit);
        out
    }

    /// 当前目录直属子项的 CSV 清单（UTF-8，CRLF，含表头）。
    pub fn children_csv(&self, id: u32) -> String {
        let base = self.nodes[id as usize].size.max(1);
        let mut out =
            String::from("名称,类型,大小(字节),大小(可读),占当前目录比例(%),完整路径\r\n");
        for child in self.sorted_children(id) {
            let node = &self.nodes[child as usize];
            let pct = node.size as f64 * 100.0 / base as f64;
            out.push_str(&format!(
                "{},{},{},{},{:.2},{}\r\n",
                csv_escape(&node.name),
                node.kind.label(),
                node.size,
                crate::util::format_size(node.size),
                pct,
                csv_escape(&self.path_of(child))
            ));
        }
        out
    }
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> Tree {
        let mut tree = Tree::new("C:\\root".to_string());
        let root = tree.add_node("C:\\root".into(), NO_PARENT, NodeKind::Dir);
        let a = tree.add_node("alpha".into(), root, NodeKind::Dir);
        let b = tree.add_node("beta".into(), root, NodeKind::Dir);
        let f1 = tree.add_node("one.bin".into(), a, NodeKind::File);
        let f2 = tree.add_node("two.bin".into(), a, NodeKind::File);
        let f3 = tree.add_node("three.txt".into(), b, NodeKind::File);
        tree.nodes[f1 as usize].size = 30;
        tree.nodes[f2 as usize].size = 20;
        tree.nodes[f3 as usize].size = 50;
        for i in (1..tree.nodes.len()).rev() {
            let (parent, size, files) = {
                let n = &tree.nodes[i];
                (n.parent, n.size, n.files)
            };
            let p = &mut tree.nodes[parent as usize];
            p.size += size;
            p.files += files;
        }
        tree
    }

    #[test]
    fn aggregates_sizes_and_counts() {
        let tree = sample_tree();
        assert_eq!(tree.total_size(), 100);
        assert_eq!(tree.total_files(), 3);
        assert_eq!(tree.nodes[1].size, 50);
        assert_eq!(tree.nodes[1].files, 2);
    }

    #[test]
    fn detach_updates_ancestor_totals() {
        let mut tree = sample_tree();
        assert!(tree.detach(1));
        assert_eq!(tree.total_size(), 50);
        assert_eq!(tree.total_files(), 1);
        assert_eq!(tree.dir_count(), 2);
        assert_eq!(tree.sorted_children(0), vec![2]);
    }

    #[test]
    fn detach_rejects_root_and_out_of_range() {
        let mut tree = sample_tree();
        assert!(!tree.detach(0));
        assert!(!tree.detach(99));
        assert_eq!(tree.total_size(), 100);
    }

    #[test]
    fn detach_file_keeps_directories() {
        let mut tree = sample_tree();
        assert!(tree.detach(4));
        assert_eq!(tree.total_size(), 80);
        assert_eq!(tree.total_files(), 2);
        assert_eq!(tree.dir_count(), 3);
    }

    #[test]
    fn builds_paths_and_ancestors() {
        let tree = sample_tree();
        assert_eq!(tree.path_of(4), "C:\\root\\alpha\\two.bin");
        assert_eq!(tree.ancestors(4), vec![0, 1, 4]);
    }

    #[test]
    fn sorts_children_by_size_desc() {
        let tree = sample_tree();
        let order = tree.sorted_children(1);
        assert_eq!(order, vec![3, 4]);
    }

    #[test]
    fn finds_top_files_and_search() {
        let tree = sample_tree();
        assert_eq!(tree.top_files(0, 2), vec![5, 3]);
        assert_eq!(tree.search(0, "TWO", 10), vec![4]);
        assert!(tree.search(0, "missing", 10).is_empty());
    }

    #[test]
    fn exports_csv_with_header_and_rows() {
        let tree = sample_tree();
        let csv = tree.children_csv(0);
        assert!(csv.starts_with("名称,类型"));
        assert!(csv.contains("alpha"));
        assert!(csv.contains("50.00"));
        assert!(csv.lines().count() == 3);
    }
}
