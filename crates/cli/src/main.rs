use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use kuadrat_core::exec::local::LocalExecutor;
use kuadrat_core::spec::WorkloadSpec;
use kuadrat_core::workloads::apply::{apply, remove, Paths};
use kuadrat_core::workloads::query::{list, status};

#[derive(Parser)]
#[command(
    name = "kuadrat",
    about = "Podman Quadlet deployment for a single host"
)]
struct Cli {
    /// Treat all paths as relative to this root (for testing without touching /etc)
    #[arg(long)]
    root: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Apply a workload spec from a JSON file
    Apply { file: std::path::PathBuf },
    /// Remove a workload by name
    Remove { name: String },
    /// Show a workload's state
    Status { name: String },
    /// List kuadrat-managed workloads
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = match &cli.root {
        Some(root) => Paths::rooted(root),
        None => Paths::default(),
    };
    let exec = LocalExecutor;

    match cli.command {
        Command::Apply { file } => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let spec: WorkloadSpec = serde_json::from_str(&text).context("parsing spec JSON")?;
            apply(&exec, &paths, &spec).await?;
            println!("applied {}", spec.name);
        }
        Command::Remove { name } => {
            remove(&exec, &paths, &name).await?;
            println!("removed {name}");
        }
        Command::Status { name } => {
            let state = status(&exec, &paths, &name).await?;
            println!("{}", state.label());
        }
        Command::List => {
            for name in list(&paths).await? {
                println!("{name}");
            }
        }
    }

    Ok(())
}
