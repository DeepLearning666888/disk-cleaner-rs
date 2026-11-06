# disk-cleaner-rs

[![release](https://img.shields.io/github/v/tag/DeepLearning666888/disk-cleaner-rs?label=release)](https://github.com/DeepLearning666888/disk-cleaner-rs/releases/latest) [![build](https://img.shields.io/github/actions/workflow/status/DeepLearning666888/disk-cleaner-rs/release.yml?branch=main&label=build)](https://github.com/DeepLearning666888/disk-cleaner-rs/actions/workflows/release.yml) [![license](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

Windows 磁盘空间分析与清理工具。扫描目录占用，用矩形树图展示空间分布，按规则给出清理建议；所有删除操作一律移入回收站。

## 功能

- 目录扫描：统计子目录与文件占用，支持 `C:\` 等大目录
- 矩形树图：按占用比例可视化，可点击下钻并查看详情
- 清理建议：按规则识别临时文件、缓存、旧安装包等候选
- 安全删除：只移入 Windows 回收站，不永久删除
- 搜索与大文件清单：筛选大文件、批量选择候选

## 安全说明

- 分析阶段只读，不修改磁盘内容。
- 删除通过回收站完成，可在资源管理器中还原。
- 内置受保护路径拦截：`C:\`、`C:\Windows`、`C:\Program Files`、`C:\ProgramData`、用户目录整体等会被拒绝。
- 删除前展示目标清单，需要显式确认。

## 下载安装（推荐）

无需安装 Rust 工具链，直接使用预构建版本：

1. 前往 [Releases](https://github.com/DeepLearning666888/disk-cleaner-rs/releases/latest) 页面，下载最新版本的 `disk-cleaner-rs-<版本号>-x86_64-pc-windows-gnu.zip` 与同目录下的 `SHA256SUMS.txt`
2. 按下文「校验下载的二进制」完成 SHA256 与来源证明两步校验
3. 校验通过后解压，运行其中的 `disk-app.exe` 即可

Windows 可能弹出 SmartScreen「未知发布者」提示：本工具未做代码签名，若信任该来源可选择「仍要运行」；有疑虑可参考下一节自行构建。

发布与版本约定：发布版本统一使用 `v` 前缀语义化 tag（如 `v0.1.0`），推送 tag 后由 GitHub Actions 自动构建并创建 Release，不混用其他前缀的 tag。

## 环境要求

- Windows 10 / 11
- Rust stable（`rust-toolchain.toml` 指定 `stable-x86_64-pc-windows-gnu`）
- MinGW-w64（提供 `x86_64-w64-mingw32-gcc`），默认位于 `%USERPROFILE%\.mingw64`

## 构建与运行

```powershell
$env:PATH = "$env:USERPROFILE\.mingw64\bin;$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build --release
.\target\release\disk-app.exe
```

也可以直接双击 `启动.bat`：已构建时直接启动，否则先编译。

## 校验下载的二进制（防篡改）

Release 页面下载的 zip 与仓库构建产物为同一来源，均适用以下两步校验。发布产物同时提供 `SHA256SUMS.txt` 与构建来源证明（GitHub Artifact Attestation），来源证明绑定到本仓库的 Release 工作流。

```powershell
# 1. 校验文件完整性：输出哈希需与 SHA256SUMS.txt 中的值一致
Get-FileHash .\disk-cleaner-rs-*-x86_64-pc-windows-gnu.zip -Algorithm SHA256

# 2. 校验来源证明：确认产物由本仓库的 GitHub Actions 构建（需已登录 gh）
gh attestation verify .\disk-cleaner-rs-*-x86_64-pc-windows-gnu.zip -R DeepLearning666888/disk-cleaner-rs
```

未通过以上两步校验的二进制不要运行。

可复现性说明：构建时已关闭 PE 时间戳写入（`-Wl,--no-insert-timestamp`），同一缓存下重复构建（增量重建）可得到相同字节；但冷构建（清空 `target` 后重建）与跨机器构建不保证字节一致，原因是 rustc 冷构建会生成随机临时目录名并写入产物符号表，该差异无法通过编译器参数消除。

## 质量检查

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 依赖安全审计

CI 会执行 `cargo-audit audit` 检查 `Cargo.lock`，已知的传递依赖告警在 `.cargo/audit.toml` 中维护：

- 4 条 `unmaintained`（ttf-parser、zbus_names、zvariant、zvariant_derive）按 RUSTSEC ID 显式忽略——均为传递依赖、无已知可利用漏洞，且受 eframe / rfd 版本锁定短期无法升级；
- `async-executor 1.14.0`（yanked）保留告警：cargo-audit 不支持按 crate 精确忽略 yanked，整体关闭检查会失去对未来新 yank 的感知，故仅显示为黄色注解，不影响 CI 结果。

约定：若出现新的 vulnerability（非 unmaintained / yanked），必须单独评估，不得引用以上清单放过；上游升级后应及时移除对应忽略项。

## 目录结构

```text
crates/disk-core   扫描、规则、回收站删除、树图布局
crates/disk-app    eframe/egui 图形界面
```

## 许可证

MIT，详见 [LICENSE](LICENSE)。
