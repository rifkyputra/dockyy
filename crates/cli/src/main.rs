use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use kuadrat_core::exec::local::LocalExecutor;
use kuadrat_core::fs::local::LocalFileSystem;
use kuadrat_core::spec::WorkloadSpec;
use kuadrat_core::workloads::apply::{apply, remove, Paths};
use kuadrat_core::workloads::query::{list, status};

mod resolve;

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
    /// Build a repo's image without deploying it
    Build { path: std::path::PathBuf },
    /// Build and deploy an app from a local repo
    Deploy {
        app: String,
        path: std::path::PathBuf,
        /// Route this app: domain:port (e.g. example.com:3000)
        #[arg(long)]
        route: Option<String>,
    },
    /// Manage podman secrets (values read from stdin, never argv)
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },
    /// Recover from crashed deploys: roll back anything left in progress
    Reconcile,
}

#[derive(Subcommand)]
enum SecretAction {
    /// Create or replace a secret; the value is read verbatim from stdin
    Set { name: String },
    /// List secret names
    Ls,
    /// Remove a secret
    Rm { name: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = match &cli.root {
        Some(root) => Paths::rooted(root),
        None => Paths::default(),
    };
    let exec = LocalExecutor;
    let fsys = LocalFileSystem;

    match cli.command {
        Command::Apply { file } => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let spec: WorkloadSpec = serde_json::from_str(&text).context("parsing spec JSON")?;
            apply(&exec, &fsys, &paths, &spec).await?;
            println!("applied {}", spec.name);
        }
        Command::Remove { name } => {
            remove(&exec, &fsys, &paths, &name).await?;
            println!("removed {name}");
        }
        Command::Status { name } => {
            let state = status(&exec, &fsys, &paths, &name).await?;
            println!("{}", state.label());
        }
        Command::List => {
            for name in list(&fsys, &paths).await? {
                println!("{name}");
            }
        }
        Command::Build { path } => {
            use kuadrat_core::deploy::{build::build, detect::detect};
            use kuadrat_core::spec::slug;

            let abs = path
                .canonicalize()
                .with_context(|| format!("no such path: {}", path.display()))?;
            let name = abs
                .file_name()
                .and_then(|n| n.to_str())
                .context("path has no final component to name the app after")?;
            let plan = detect(&exec, &fsys, &abs).await?;
            let image = build(&exec, &plan, &slug(name)).await?;
            println!("{image}");
        }
        Command::Deploy { app, path, route } => {
            use kuadrat_core::deploy::{run, Ctx, DeployOutcome};
            use kuadrat_core::spec::Route;
            use kuadrat_core::store::Store;

            let route_override = match route {
                Some(s) => {
                    let (domain, port) =
                        s.rsplit_once(':').context("--route must be domain:port")?;
                    Some(Route {
                        domain: domain.to_string(),
                        port: port.parse().context("--route port must be a number")?,
                    })
                }
                None => None,
            };

            let store = Store::open(&paths.db_path)?;
            let spec = resolve::resolve_spec(&app, &path, &store, route_override)?;
            let ctx = Ctx {
                exec: &exec,
                fsys: &fsys,
                store: &store,
                paths: &paths,
            };
            let outcome = run(&ctx, spec, &path).await?;
            println!("{outcome:?}");
            // A rolled-back or failed deploy exits non-zero (CI-friendly); only
            // `Done` is success.
            if !matches!(outcome, DeployOutcome::Done { .. }) {
                std::process::exit(1);
            }
        }
        Command::Secret { action } => {
            use kuadrat_core::secrets;
            match action {
                SecretAction::Set { name } => {
                    use std::io::Read;
                    let mut value = String::new();
                    std::io::stdin()
                        .read_to_string(&mut value)
                        .context("reading the secret value from stdin")?;
                    secrets::set(&exec, &name, &value).await?;
                    println!("set secret {name}");
                }
                SecretAction::Ls => {
                    for n in secrets::list(&exec).await? {
                        println!("{n}");
                    }
                }
                SecretAction::Rm { name } => {
                    secrets::remove(&exec, &name).await?;
                    println!("removed secret {name}");
                }
            }
        }
        Command::Reconcile => {
            use kuadrat_core::deploy::{reconcile, Ctx};
            use kuadrat_core::store::Store;

            let store = Store::open(&paths.db_path)?;
            let ctx = Ctx {
                exec: &exec,
                fsys: &fsys,
                store: &store,
                paths: &paths,
            };
            let outcomes = reconcile(&ctx).await?;
            if outcomes.is_empty() {
                println!("nothing to reconcile");
            } else {
                for outcome in &outcomes {
                    println!("{outcome:?}");
                }
            }
        }
    }

    Ok(())
}
