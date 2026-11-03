//! 磁盘空间分析与安全清理核心库。
//!
//! 职责划分：
//! - [`scan`] 递归扫描目录并聚合大小
//! - [`model`] 目录树数据模型与查询
//! - [`treemap`] squarified treemap 布局
//! - [`delete`] 回收站删除与系统目录保护
//! - [`rules`] 清理候选筛选规则
//! - [`util`] 大小格式化等纯函数

pub mod delete;
pub mod model;
pub mod rules;
pub mod scan;
pub mod treemap;
pub mod util;

pub use delete::{is_protected, recycle, DeleteError};
pub use model::{Node, NodeKind, Tree, NO_PARENT};
pub use rules::{collect as collect_candidates, Candidate, CandidateSet, RuleOptions};
pub use scan::{scan, ScanError, ScanProgress};
pub use treemap::{squarify, Rect};
pub use util::format_size;
