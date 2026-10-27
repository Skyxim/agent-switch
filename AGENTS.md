# AGENTS.md

`agent-switch` (`ags`) 是用于快速切换 AI Agent（Codex 与 Antigravity）凭证的多账号管理工具。

## 常用命令速查

本项目统一使用 `just` 管理构建、测试、检查与打包发布：

```bash
just                                     # 列出所有可用命令
just check                               # 快速检查代码语法与类型
just check-target <target>               # 检查指定架构的代码语法与二进制构建
just test                                # 运行全量单元测试 (支持参数透传，如 just test -- --nocapture)
just clippy                              # 代码静态规范分析
just fmt                                 # 代码格式化 (CI 检查: just fmt -- --check)
just build [profile] [target]            # 编译二进制 (默认: profile="release", target="")
just package [profile] [target]          # 构建并打包至 dist/ (生成归档及 sha256 校验和)
just run info                            # 运行查看当前环境与账号状态
just bump 0.2.0                          # 升级版本号
just changelog                           # 使用 git-cliff 更新 CHANGELOG.md
```

## 核心环境变量

本项目仅定义且依赖两个核心环境变量：

- `AGS_HOME`：全局配置与数据根目录（默认 `~/.config/agent-switch`，解析优先级：`AGS_HOME` -> `XDG_CONFIG_HOME/agent-switch` -> `~/.config/agent-switch` -> `HOME/.config/agent-switch` -> `.config/agent-switch`）。
- `AGS_KEY`：存储加密主密钥（若未设置或为空则为明文 Plaintext，向下兼容旧名 `AGS_ENCRYPT_KEY`）。

## 架构与核心约定

1. **分层架构与代理抽象 (`AgentApp`)**：
   - 统一多态抽象：所有 Agent（Codex 与 Antigravity）均实现 `AgentApp` Trait（`src/agent/traits.rs`），返回纯强类型 DTO，彻底解耦终端打印；
   - 动态调度：通过 `all_agents()` 与 `get_agent()` 统一调度，方便全局命令（`ags list`, `ags usage`, `ags export`）多态操作。
2. **强类型 Serde DTO (`src/agent/dto.rs`)**：
   - 全面采用 Serde 强类型模型解析 JWT Claims、Token 与配额 API 响应，严禁散落弱类型索引提取。
3. **表现层与通用 UI (`src/ui/`)**：
   - 终端表格、使用量进度条与状态颜色统一定义于 `src/ui/`，供 CLI 命令复用。
4. **存储引擎抽象 (`StoreEngine`)**：
   - 模型抽象：`(store_engine, key)`。
   - `env`（默认）：基于环境变量 `AGS_KEY`（向下兼容 `AGS_ENCRYPT_KEY`）。若未设置或为空，**严格视为不加密**（以明文方式存储/读取，显示为 `Plaintext` 状态）；若设置了密钥，则采用 AES-256-GCM 加密，带 `AGSV1` 帧头与 12 字节 Nonce。
   - `plain`：明文存储，带 `AGSPLAIN` 帧头。
   - `system`：预留给系统底层 Keyring 引擎。
5. **导出与导入归档机制 (`ags export` / `ags import`)**：
   - 存储无关加密归档：`ags export` 采用独立 Key（PBKDF2 + AES-256-GCM，外壳 `AGSEXP1`）打包纯凭据，完全解耦底层存储引擎；
   - 自适应重加密导入：`ags import` 自动适配目标机器存储引擎，导入前自动快照。若遇旧不兼容数据，交互询问清理重置或通过 `--clean` 自动确认。
6. **存储路径规范**：
   - 根目录：统一由环境变量 `AGS_HOME` 自定义配置与数据存储根目录（默认解析优先级：`AGS_HOME` -> `XDG_CONFIG_HOME/agent-switch` -> `~/.config/agent-switch` -> `HOME/.config/agent-switch` -> `.config/agent-switch`）。
   - 子目录组织：`codex/`、`agy/` 下分别维护 `registry.json`、`accounts/` 与 `backups/`。

## 安全与测试规范

- **安全红线**：严禁在代码、日志、测试输出或命令示例中硬编码或回显真实的 Token、密码或密钥。
- **原子落盘**：所有凭证与注册表写入必须使用 `atomic_write`，并在 Unix 系统上限制文件权限为 `0600`。
- **测试闭环**：单测覆盖加密解密边界、注册表查找逻辑、迁移全流程；所有测试必须在隔离的临时目录中运行，禁止读写用户实际凭证路径。
