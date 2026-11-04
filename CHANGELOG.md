# Changelog

本项目遵循 [语义化版本](https://semver.org/lang/zh-CN/)，记录格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [未发布]

## [0.2.0] - 2026-11-04

### 新增

- 磁盘空间扫描与树图可视化（treemap）。
- 大文件、搜索结果、清理候选三类清单。
- 基于规则的清理建议，删除操作统一走回收站。
- 侧栏、详情与弹窗过渡动画，扫描进度约 100ms 刷新。
- `启动.bat` 一键构建并运行，控制台代码页切换为 UTF-8。
- 统一错误类型 `CoreError`，可由 `ScanError`、`DeleteError`、`io::Error` 转换。

### 工程

- 集成测试覆盖扫描、规则筛选与删除拦截（`crates/disk-core/tests`）。
- GitHub Actions CI：`cargo fmt --check`、`clippy -D warnings`、`cargo test`、`cargo-audit` 依赖扫描与依赖来源检查。
- 供应链加固：第三方 Action 固定 commit SHA、`--locked` 构建、分支保护、Release 附带 SHA-256 校验和与构建来源证明。
- 启用 Dependabot 每周更新 GitHub Actions。
