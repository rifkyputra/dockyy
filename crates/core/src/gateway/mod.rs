//! One Caddy fragment per app. kuadrat writes `<caddy_dir>/<slug>.caddy`; the
//! operator's Caddyfile imports them with `import kuadrat.d/*.caddy`. Each
//! fragment is marker-guarded so kuadrat never clobbers a hand-written file.

use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::exec::Executor;
use crate::fs::FileSystem;
use crate::managed::ensure_owned;
use crate::workloads::paths::Paths;
use crate::workloads::render::MANAGED_MARKER;

/// A public route: a domain reverse-proxied to a local port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub domain: String,
    pub port: u16,
}

/// Render the Caddy fragment for a route. Pure — no I/O. Caddy auto-provisions
/// TLS for a public domain.
pub fn render_fragment(route: &Route) -> String {
    format!(
        "{MANAGED_MARKER}\n{} {{\n\treverse_proxy localhost:{}\n}}\n",
        route.domain, route.port
    )
}

/// Path of the fragment kuadrat writes for an app.
pub fn fragment_path(paths: &Paths, slug: &str) -> PathBuf {
    paths.caddy_dir.join(format!("{slug}.caddy"))
}

/// Write the fragment (refusing to clobber a foreign file) and reload Caddy.
pub async fn apply_route(
    exec: &dyn Executor,
    fsys: &dyn FileSystem,
    paths: &Paths,
    slug: &str,
    route: &Route,
) -> Result<()> {
    let path = fragment_path(paths, slug);
    ensure_owned(fsys, &path, MANAGED_MARKER, "overwrite").await?;

    fsys.create_dir_all(&paths.caddy_dir).await?;
    fsys.write(&path, &render_fragment(route)).await?;

    reload_caddy(exec).await
}

/// Delete the fragment (if kuadrat owns it) and reload Caddy. Safe if absent.
pub async fn remove_route(
    exec: &dyn Executor,
    fsys: &dyn FileSystem,
    paths: &Paths,
    slug: &str,
) -> Result<()> {
    let path = fragment_path(paths, slug);
    if !ensure_owned(fsys, &path, MANAGED_MARKER, "remove").await? {
        return Ok(());
    }
    fsys.remove_file(&path).await?;
    reload_caddy(exec).await
}

async fn reload_caddy(exec: &dyn Executor) -> Result<()> {
    let out = exec
        .run("systemctl", &["reload".to_string(), "caddy".to_string()])
        .await?;
    if !out.success() {
        bail!("systemctl reload caddy failed: {}", out.stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::workloads::paths::Paths;
    use std::path::Path;

    fn route() -> Route {
        Route {
            domain: "example.com".to_string(),
            port: 3000,
        }
    }
    fn ok() -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    #[test]
    fn renders_the_golden_fragment() {
        let expected = include_str!("../../tests/golden/route.caddy");
        assert_eq!(render_fragment(&route()), expected);
    }

    #[test]
    fn fragment_path_is_slug_dot_caddy_under_caddy_dir() {
        let paths = Paths::rooted(Path::new("/root"));
        assert_eq!(
            fragment_path(&paths, "web"),
            Path::new("/root/caddy/kuadrat.d/web.caddy")
        );
    }

    #[tokio::test]
    async fn apply_route_writes_the_fragment_and_reloads_caddy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fsys = crate::fs::local::LocalFileSystem;
        let exec = FakeExecutor::new();
        exec.expect_call("systemctl", &["reload", "caddy"], ok());

        apply_route(&exec, &fsys, &paths, "web", &route())
            .await
            .expect("apply");

        let written = std::fs::read_to_string(fragment_path(&paths, "web")).expect("fragment");
        assert!(written.contains("reverse_proxy localhost:3000"));
        assert!(written.starts_with(crate::workloads::render::MANAGED_MARKER));
        assert_eq!(
            exec.calls()[0].1,
            vec!["reload".to_string(), "caddy".to_string()]
        );
    }

    #[tokio::test]
    async fn apply_route_refuses_a_foreign_fragment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fsys = crate::fs::local::LocalFileSystem;
        let exec = FakeExecutor::new();

        std::fs::create_dir_all(&paths.caddy_dir).expect("mkdir");
        std::fs::write(fragment_path(&paths, "web"), "hand written\n").expect("foreign");

        let err = apply_route(&exec, &fsys, &paths, "web", &route())
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("did not write it"),
            "message was: {err}"
        );
    }

    #[tokio::test]
    async fn remove_route_deletes_and_reloads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fsys = crate::fs::local::LocalFileSystem;
        let exec = FakeExecutor::new();
        exec.expect_call("systemctl", &["reload", "caddy"], ok());

        apply_route(&exec, &fsys, &paths, "web", &route())
            .await
            .expect("apply");
        remove_route(&exec, &fsys, &paths, "web")
            .await
            .expect("remove");
        assert!(!fragment_path(&paths, "web").exists());
    }

    #[tokio::test]
    async fn remove_route_is_ok_when_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fsys = crate::fs::local::LocalFileSystem;
        let exec = FakeExecutor::new();
        remove_route(&exec, &fsys, &paths, "never")
            .await
            .expect("no error");
    }
}
