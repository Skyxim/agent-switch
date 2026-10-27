use anyhow::{bail, Result};
use clap::Args;
use colored::Colorize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Args, Debug, Clone)]
pub struct ImportArgs {
    /// 待导入的加密归档文件路径 (.agx)
    pub file: PathBuf,

    /// 归档解密口令（若未提供且在交互式终端中，将提示输入）
    #[arg(short, long, visible_alias = "key")]
    pub password: Option<String>,

    /// 限制仅导入的 Agent（如 codex, agy；默认全部导入）
    #[arg(long)]
    pub agent: Option<String>,

    /// 是否覆盖本地同名/同邮箱账号（默认: true）
    #[arg(long, default_value_t = true)]
    pub overwrite: bool,

    /// 若检测到本地存在损坏或密钥不一致的不兼容旧数据，确认直接清理并重新导入
    #[arg(long, default_value_t = false)]
    pub clean: bool,
}

pub fn execute(base_dir: &Path, args: ImportArgs) -> Result<()> {
    if !args.file.exists() {
        bail!("备份文件不存在: {}", args.file.display());
    }

    let password = crate::commands::prompt_password(args.password.as_deref(), false)?;

    let global_config = crate::config::load_global_config(base_dir).unwrap_or_default();
    let local_crypto = crate::crypto::CryptoManager::create_provider(
        global_config.crypto_provider.as_deref(),
        None,
    );

    let target_agents: Vec<String> = match &args.agent {
        Some(a) => {
            if let Some(agent) = crate::agent::get_agent(a) {
                vec![agent.id().to_string()]
            } else {
                vec![a.to_lowercase()]
            }
        }
        None => crate::agent::all_agents()
            .iter()
            .map(|a| a.id().to_string())
            .collect(),
    };

    let mut clean_incompatible = args.clean;
    if !clean_incompatible {
        for agent in &target_agents {
            let agent_dir = base_dir.join(agent);
            if let crate::archive::CompatibilityStatus::Incompatible(reason) =
                crate::archive::check_agent_compatibility(&agent_dir, &*local_crypto)
            {
                println!(
                    "{} 检测到本地已存在 [{}] 数据，但与当前环境不兼容（{}）。",
                    "⚠".yellow().bold(),
                    agent.bold(),
                    reason.dimmed()
                );
                print!("? 是否备份并清理此应用的旧不兼容数据，以便继续导入？ (y/N): ");
                io::stdout().flush()?;
                let mut answer = String::new();
                io::stdin().read_line(&mut answer)?;
                let trimmed = answer.trim().to_lowercase();
                if trimmed == "y" || trimmed == "yes" {
                    clean_incompatible = true;
                } else {
                    bail!("操作已取消。本地不兼容数据保持原样未变。如确认需要清理，请使用 --clean 参数或在提示时输入 y。");
                }
            }
        }
    }

    let opts = crate::archive::ImportOptions {
        base_dir: base_dir.to_path_buf(),
        input_path: args.file.clone(),
        password,
        target_agents: args.agent.map(|a| vec![a.to_lowercase()]),
        clean_incompatible,
        overwrite: args.overwrite,
    };

    let report = crate::archive::run_import(&opts)?;

    println!(
        "{} {}",
        "✔".green().bold(),
        "账号备份归档导入成功！".green().bold()
    );
    println!("  归档源文件: {}", args.file.display().to_string().cyan());
    for (agent, (new_count, merged_count)) in &report.agent_details {
        println!(
            "  • {}: 新增 {} 个账号，合并/覆盖 {} 个账号",
            agent.bold(),
            new_count.to_string().cyan().bold(),
            merged_count.to_string().yellow().bold()
        );
    }
    println!(
        "  汇总统计: 共导入/处理 {} 个账号 (新增 {}, 合并 {})",
        report.total_accounts.to_string().bold(),
        report.total_new.to_string().cyan(),
        report.total_merged.to_string().yellow()
    );

    if !report.backup_dirs.is_empty() {
        println!("  安全快照备份路径:");
        for b in &report.backup_dirs {
            println!("    - {}", b.display().to_string().dimmed());
        }
    }

    if !report.cleaned_agents.is_empty() {
        println!(
            "  {} 已安全清理并重置不兼容旧数据: {}",
            "✔".green(),
            report.cleaned_agents.join(", ").yellow()
        );
    }

    let engine_name = global_config
        .crypto_provider
        .unwrap_or_else(|| "env".to_string());
    println!(
        "  {}",
        format!(
            "[存储适配] 凭据已自动根据当前本机存储引擎 [{}] 重加密存储闭环。",
            engine_name
        )
        .dimmed()
    );

    Ok(())
}

pub use execute as handle_import;
