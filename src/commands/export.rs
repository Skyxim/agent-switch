use anyhow::Result;
use clap::Args;
use colored::Colorize;
use std::path::{Path, PathBuf};

#[derive(Args, Debug, Clone)]
pub struct ExportArgs {
    /// 导出文件路径（默认: ./ags-backup-<YYYYMMDD_HHMMSS>.agx）
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// 归档加密口令（若未提供且在交互式终端中，将提示输入）
    #[arg(short, long, visible_alias = "key")]
    pub password: Option<String>,

    /// 目标 Agent（如 codex, agy；默认全部导出）
    #[arg(long)]
    pub agent: Option<String>,
}

pub fn execute(base_dir: &Path, args: ExportArgs) -> Result<()> {
    let password = crate::commands::prompt_password(args.password.as_deref(), true)?;

    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let output_path = args
        .output
        .unwrap_or_else(|| PathBuf::from(format!("./ags-backup-{}.agx", ts)));

    let target_agents: Vec<String> = match args.agent {
        Some(a) => {
            if let Some(agent) = crate::agent::get_agent(&a) {
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

    let opts = crate::archive::ExportOptions {
        base_dir: base_dir.to_path_buf(),
        output_path,
        password,
        target_agents,
    };

    let report = crate::archive::run_export(&opts)?;

    println!(
        "{} {}",
        "✔".green().bold(),
        "账号备份归档导出成功！".green().bold()
    );
    println!(
        "  归档文件: {}",
        report.output_path.display().to_string().cyan().bold()
    );
    println!("  文件大小: {:.1} KB", (report.file_size as f64) / 1024.0);
    println!("  导出品类与账号数:");
    for agent in &report.exported_agents {
        let count = report.agent_counts.get(agent).copied().unwrap_or(0);
        println!(
            "    • {}: {} 个账号",
            agent.bold(),
            count.to_string().cyan().bold()
        );
    }
    println!("  账号总数: {}", report.total_accounts.to_string().bold());
    println!();
    println!(
        "  {}",
        "[安全提示] 归档包已使用 PBKDF2 (100,000 轮) + AES-256-GCM 独立强加密，完全脱钩底层存储引擎，可在任意机器通过 `ags import` 导入。"
            .yellow()
    );

    Ok(())
}

pub use execute as handle_export;
