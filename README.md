# disk-cleaner-rs

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

发布产物同时提供 `SHA256SUMS.txt` 与构建来源证明（GitHub Artifact Attestation），来源证明绑定到本仓库的 Release 工作流。

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

## 目录结构

```text
crates/disk-core   扫描、规则、回收站删除、树图布局
crates/disk-app    eframe/egui 图形界面
```

## 许可证

MIT，详见 [LICENSE](LICENSE)。
