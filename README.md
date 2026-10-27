# agent-switch (`ags`)

AI Agent (Codex & Antigravity) 多账号凭证切换与配额管理工具。

## 功能特性

- **多账号切换**：统一管理 OpenAI Codex 与 Google Antigravity 账号凭证，支持查看账号列表并快速切换当前活跃账号。
- **凭据存储方式**：当前仅支持通过环境变量密钥进行加密存储；若未设置密钥，则以明文形式保存。
- **导出与导入**：支持将账号凭据导出为加密备份归档，方便在不同设备间迁移与恢复。
- **配额与用量**：实时查询各账号的额度与使用情况（如 Codex 的 5 小时与每周窗口限额、Antigravity 的配额使用比例）。

## 环境变量与安全性

`ags` 支持以下两个环境变量：

- `AGS_HOME`：配置与数据存储根目录（默认 `~/.config/agent-switch`）。
- `AGS_KEY`：凭据存储加密密钥。

### 局限性与安全建议

当前版本仅支持通过环境变量密钥进行加密存储。若在 Shell 配置文件（如 `~/.bashrc`、`~/.zshrc`）中静态写入 `export AGS_KEY="..."`，存在一定的安全隐患：

- 密钥可能被记录到 Shell 历史命令中；
- 暴露在系统进程环境中（如通过进程查看工具可能对其他同权限进程可见）；
- 容易在系统日志或崩溃转储中被意外捕获。

推荐配合密码管理器 CLI 工具使用，在执行命令时动态注入密钥，避免静态明文写入配置文件：

- **1Password CLI (`op run`)**：
  ```bash
  op run -- ags <command>
  ```
- **Proton Pass CLI (`pass-cli run`)**：
  ```bash
  pass-cli run -- ags <command>
  ```

## 安装与卸载

### 安装

**Linux & macOS**：
```bash
curl -fsSL https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.sh | bash
```

**Windows**（PowerShell）：
```powershell
irm https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.ps1 | iex
```

**Homebrew (macOS / Linux)**：
```bash
brew install Skyxim/agent-switch/agent-switch
```

### 卸载

- **Linux / macOS**：
  ```bash
  curl -fsSL https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.sh | bash -s -- --uninstall
  # 若需彻底清除配置与数据 (~/.config/agent-switch)：
  curl -fsSL https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.sh | bash -s -- --uninstall --clean-data
  ```

- **Windows**：
  ```powershell
  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.ps1))) -Uninstall
  # 若需彻底清除配置与数据：
  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.ps1))) -Uninstall -CleanData
  ```

## 常用命令

```bash
# 查看当前环境与账号状态
ags info

# ==================== Codex ====================
ags codex add [alias]    # 保存当前 ~/.codex/auth.json 凭证
ags codex ls             # 列出已保存的 Codex 账号
ags codex sw <name>      # 切换账号（支持别名、邮箱、序号，'-' 切回上一账号）
ags codex current        # 查看当前活跃账号详情
ags codex rm <name>      # 删除账号
ags codex usage          # 查看 ChatGPT 配额与用量

# ================= Antigravity =================
ags agy add [alias]      # 保存当前 Antigravity 凭证
ags agy ls               # 列出已保存的 Antigravity 账号
ags agy sw <name>        # 切换账号（支持 '-' 切回上一账号）
ags agy current          # 查看当前活跃账号详情
ags agy rm <name>        # 删除账号
ags agy usage            # 查看 Google Cloud Code 配额与用量

# ================= 导出与导入 =================
# 导出加密备份归档（支持交互输入口令或传参）
ags export -k "my-archive-password"

# 导出指定 Agent 并自定义输出文件路径
ags export --agent codex -o ./codex-backup.agx -k "my-archive-password"

# 导入加密备份归档
ags import ./ags-backup.agx -k "my-archive-password"

# 导入时如遇旧不兼容数据直接清理重置
ags import ./ags-backup.agx -k "my-archive-password" --clean
```

## 工程指令

```bash
just check   # 语法与编译检查
just test    # 运行全量单元测试
just build   # 编译 Release 二进制产物
```

## 鸣谢

本项目在设计与实现过程中参考了以下开源项目，特此鸣谢：

- [WellWells/agentswap](https://github.com/WellWells/agentswap)
- [g1mn/agyswap](https://github.com/g1mn/agyswap)
