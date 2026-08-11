//! Parsing of the two CLI arguments that carry real logic: `--route
//! domain:port`, and the app name `kuadrat build` derives from a repo path.
//!
//! They live here rather than inline in the `match` arms so they can be tested
//! without spawning the binary. Both fail *before* anything touches the host —
//! a malformed route must not reach the gateway as an empty Caddy site address,
//! and an unnameable path must not become the image tag `localhost/kuadrat-:sha`.

use std::net::SocketAddr;
use std::path::Path;

use anyhow::{bail, Context, Result};
use kuadrat_core::spec::{slug, Route};

/// The default `--listen` address for `kuadrat serve`: loopback on the
/// documented port. A function rather than a string literal so the CLI and
/// the daemon's own default (`Config::default`) cannot drift apart silently.
pub fn default_listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], kuadrat_daemon::config::DEFAULT_PORT))
}

/// Parse `--route domain:port` into a [`Route`].
///
/// Splits on the *last* colon, so a domain may itself contain one. Rejects an
/// empty domain, a port of 0, and a domain carrying a scheme or path — Caddy
/// takes a bare site address, and each of those renders a fragment that either
/// fails to load or silently serves nothing.
pub fn parse_route(s: &str) -> Result<Route> {
    let (domain, port) = s.rsplit_once(':').context("--route must be domain:port")?;

    if domain.is_empty() {
        bail!("--route must be domain:port; the domain in {s:?} is empty");
    }
    if domain.contains('/') {
        bail!("--route takes a bare domain, not a URL: {domain:?}");
    }

    let port: u16 = port
        .parse()
        .with_context(|| format!("--route port must be a number 1-65535, got {port:?}"))?;
    if port == 0 {
        bail!("--route port must be 1-65535, got 0");
    }

    Ok(Route {
        domain: domain.to_string(),
        port,
    })
}

/// The app name `kuadrat build` derives from a repo path: its final component.
///
/// Rejects a path with no final component, and one whose final component slugs
/// to nothing — the slug becomes the image tag and the unit name, and an empty
/// one would collide with every other empty one.
pub fn app_name(path: &Path) -> Result<&str> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("path has no final component to name the app after")?;

    if slug(name).is_empty() {
        bail!("directory name {name:?} yields an empty app name; pass a repo whose directory has at least one letter or digit");
    }

    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_domain_and_port_parse() {
        let route = parse_route("example.com:3000").expect("parse");
        assert_eq!(route.domain, "example.com");
        assert_eq!(route.port, 3000);
    }

    #[test]
    fn the_split_is_on_the_last_colon_so_the_domain_may_contain_one() {
        let route = parse_route("a:b.example.com:8080").expect("parse");
        assert_eq!(route.domain, "a:b.example.com");
        assert_eq!(route.port, 8080);
    }

    #[test]
    fn a_route_without_a_colon_is_rejected() {
        let err = parse_route("example.com").expect_err("no colon");
        assert!(err.to_string().contains("domain:port"), "was: {err}");
    }

    #[test]
    fn an_empty_domain_is_rejected() {
        // ":3000" would otherwise render a Caddy fragment with no site address.
        let err = parse_route(":3000").expect_err("empty domain");
        assert!(err.to_string().contains("empty"), "was: {err}");
    }

    #[test]
    fn a_url_is_rejected_rather_than_parsed_as_a_domain() {
        // rsplit_once would happily read "https://example.com" as the domain.
        let err = parse_route("https://example.com:3000").expect_err("scheme");
        assert!(err.to_string().contains("bare domain"), "was: {err}");
    }

    #[test]
    fn a_non_numeric_port_is_rejected() {
        let err = parse_route("example.com:http").expect_err("not a number");
        assert!(err.to_string().contains("number"), "was: {err}");
    }

    #[test]
    fn a_port_above_the_u16_range_is_rejected() {
        let err = parse_route("example.com:70000").expect_err("out of range");
        assert!(err.to_string().contains("number"), "was: {err}");
    }

    #[test]
    fn a_zero_port_is_rejected() {
        let err = parse_route("example.com:0").expect_err("port zero");
        assert!(err.to_string().contains("1-65535"), "was: {err}");
    }

    #[test]
    fn an_empty_port_is_rejected() {
        let err = parse_route("example.com:").expect_err("empty port");
        assert!(err.to_string().contains("number"), "was: {err}");
    }

    #[test]
    fn the_app_name_is_the_final_path_component() {
        assert_eq!(app_name(Path::new("/home/me/apps/web")).unwrap(), "web");
    }

    #[test]
    fn a_trailing_slash_does_not_hide_the_name() {
        assert_eq!(app_name(Path::new("/home/me/apps/web/")).unwrap(), "web");
    }

    #[test]
    fn the_root_path_has_no_name() {
        let err = app_name(Path::new("/")).expect_err("root");
        assert!(err.to_string().contains("final component"), "was: {err}");
    }

    #[test]
    fn a_name_that_slugs_to_nothing_is_rejected() {
        // "---" carries no letter or digit, so the image tag would be
        // "localhost/kuadrat-:<sha>" and the unit "kuadrat-.container".
        let err = app_name(Path::new("/home/me/---")).expect_err("empty slug");
        assert!(err.to_string().contains("empty app name"), "was: {err}");
    }

    /// The guard lives in the daemon, but the CLI must not mangle the address
    /// before it gets there.
    #[test]
    fn serve_defaults_to_loopback_on_the_documented_port() {
        assert_eq!(
            default_listen(),
            "127.0.0.1:7457".parse::<SocketAddr>().unwrap()
        );
    }
}
