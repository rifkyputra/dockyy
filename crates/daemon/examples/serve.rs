//! Start the daemon against a throwaway root, to look at the pages.
//!
//! A development affordance, not the shipping entrypoint — `kuadrat serve` and
//! its systemd unit are H7. This exists so the pages can be opened in a real
//! browser before then, because handler tests cannot tell you that htmx failed
//! to load or that an attribute is misspelled.
//!
//! `--root` points every managed path at a temporary directory, so running
//! this does not write units into `/etc/containers/systemd`.
//!
//!     cargo run -p kuadrat-daemon --example serve -- /tmp/kuadrat-demo

use std::net::SocketAddr;
use std::path::PathBuf;

use kuadrat_daemon::{serve, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/kuadrat-demo".into())
        .into();
    std::fs::create_dir_all(&root)?;

    let listen: SocketAddr = std::env::var("KUADRAT_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:7457".into())
        .parse()?;

    eprintln!("root: {}", root.display());
    serve(Config {
        listen,
        socket: None,
        root: Some(root),
    })
    .await
}
