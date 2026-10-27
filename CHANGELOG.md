# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] - 2026-10-27

### Added
- **AI Agent 多账号统一管理**：
  - OpenAI Codex 支持：管理 `~/.codex/auth.json` 凭据，支持账号导入、命名别名、状态查看与快速切换（支持 `-` 切回上一活跃账号）。
  - Google Antigravity (`agy`) 支持：集成 OS Keyring (`gemini:antigravity`)、`AGY_KEYRING_DATA` 与 `~/.gemini/oauth_creds.json`，实现凭据无缝切换。
  - 配额与用量实时监测：集成 Codex 5 小时与每周用量窗口限额查询，以及 Antigravity 模型配额比率实时进度条展示。
  - 多态抽象架构：通过 `AgentApp` Trait 与强类型 Serde DTO 模型实现统一调度（`ags list`、`ags usage`）。
- **凭据存储安全与 AGS_KEY 支持**：
  - 核心环境变量规范：`AGS_KEY` 作为凭据主密钥，`AGS_HOME` 自定义全局配置与数据存储根目录。
  - 存储加密机制：基于 AES-256-GCM 高强度加密，附带 `AGSV1` 帧头与随机 Nonce 保证凭证机密性与完整性；未设密钥时安全回退至 Plaintext 明文模式。
  - 原子落盘与权限保护：凭证与注册表写入统一采用原子写入机制，Unix 系统限制权限为 `0600`。
  - 凭据迁移能力：支持 `ags migrate` 在不同存储引擎与密钥之间平滑迁移，迁移前自动生成时间戳快照备份。
- **便携式跨设备加密归档（Export & Import）**：
  - 存储无关加密归档：`ags export` 采用独立口令结合 PBKDF2 与 AES-256-GCM（`AGSEXP1` 格式）打包加密凭据，完全解耦底层存储引擎。
  - 自适应重加密导入：`ags import` 自动适配目标设备存储引擎，导入前自动生成预备份快照，支持冲突清理与重置。
- **多平台 musl 静态打包与分发**：
  - Linux 平台默认采用 musl 目标（`x86_64-unknown-linux-musl` 与 `aarch64-unknown-linux-musl`）进行零依赖静态构建。
  - 支持 macOS（Apple Silicon 与 Intel）以及 Windows x64。
  - 打包产物收敛为压缩包归档（Linux/macOS `.tar.gz`，Windows `.zip`）以及对应的 `.sha256` 校验和。
- **自我更新与一键安装脚本**：
  - 官方一键安装与卸载脚本（`install.sh` / `install.ps1`），支持指定版本安装与 PATH 自动配置。
  - 官方 Homebrew Formula 支持（`Formula/agent-switch.rb`）。
  - 内置 `ags update` 自我检查更新命令，支持检查、强制更新与自动解压替换。
