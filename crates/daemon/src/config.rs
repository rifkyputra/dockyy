//! Daemon configuration, and the one guard that is a security boundary.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use anyhow::{bail, Result};

/// Where the daemon listens and which host paths it manages.
#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub socket: Option<PathBuf>,
    /// Relocates every managed path under one directory; `None` uses the real
    /// host locations (`/etc/containers/systemd`, `/var/lib/kuadrat`, …).
    pub root: Option<PathBuf>,
}

/// The default port. Arbitrary but fixed, so a tunnel command can be written
/// down once.
pub const DEFAULT_PORT: u16 = 7457;

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)),
            socket: None,
            root: None,
        }
    }
}

impl Config {
    /// Refuse to start on a non-loopback address.
    ///
    /// This is the phase-1 decision, unchanged: the daemon has no login, no
    /// sessions and no TLS of its own, and it runs privileged enough to write
    /// systemd units. A self-rolled auth stack in front of that is a larger
    /// risk than not exposing the port at all, so exposure is enforced in code
    /// rather than documented as advice. Reaching it from elsewhere is an SSH
    /// tunnel or a VPN — both of which land on loopback and pass this check.
    pub fn validate(&self) -> Result<()> {
        let ip = self.listen.ip();
        if !is_loopback(ip) {
            bail!(
                "refusing to listen on {ip}: kuadrat binds loopback only — it has no \
                 authentication. Reach it with an SSH tunnel \
                 (ssh -L {port}:127.0.0.1:{port} host) or a VPN.",
                port = self.listen.port()
            );
        }
        Ok(())
    }
}

/// `true` for IPv4 and IPv6 loopback, including an IPv4-mapped IPv6 loopback.
///
/// `Ipv6Addr::is_loopback` is false for `::ffff:127.0.0.1`, which a dual-stack
/// resolver can produce for "localhost" — treating that as non-loopback would
/// refuse to start for a genuinely local address.
fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.is_loopback(),
            None => v6.is_loopback(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;
    use std::str::FromStr;

    fn cfg(addr: &str) -> Config {
        Config {
            listen: SocketAddr::from_str(addr).expect("addr"),
            ..Default::default()
        }
    }

    #[test]
    fn the_default_is_loopback_on_7457() {
        let c = Config::default();
        assert_eq!(c.listen.port(), DEFAULT_PORT);
        c.validate().expect("default must be startable");
    }

    #[test]
    fn ipv4_loopback_is_accepted() {
        cfg("127.0.0.1:7457").validate().expect("loopback");
    }

    #[test]
    fn any_address_in_the_127_block_is_loopback() {
        cfg("127.0.0.53:7457").validate().expect("127/8");
    }

    #[test]
    fn ipv6_loopback_is_accepted() {
        cfg("[::1]:7457").validate().expect("::1");
    }

    #[test]
    fn an_ipv4_mapped_ipv6_loopback_is_accepted() {
        // "::ffff:127.0.0.1" — what a dual-stack resolver can hand back for
        // localhost. Ipv6Addr::is_loopback alone says false.
        let mapped = IpAddr::V6(Ipv6Addr::from_str("::ffff:127.0.0.1").unwrap());
        assert!(is_loopback(mapped));
    }

    #[test]
    fn the_wildcard_address_is_refused() {
        let err = cfg("0.0.0.0:7457").validate().expect_err("wildcard");
        assert!(err.to_string().contains("loopback only"), "was: {err}");
    }

    #[test]
    fn a_routable_address_is_refused() {
        let err = cfg("10.0.0.5:7457").validate().expect_err("lan");
        assert!(err.to_string().contains("refusing to listen"), "was: {err}");
    }

    #[test]
    fn the_ipv6_wildcard_is_refused() {
        let err = cfg("[::]:7457").validate().expect_err("ipv6 wildcard");
        assert!(err.to_string().contains("loopback only"), "was: {err}");
    }

    #[test]
    fn the_refusal_names_the_tunnel_that_fixes_it() {
        // The error is the operator's only signal here; it must say what to do
        // instead, not just that it refused.
        let err = cfg("0.0.0.0:7457").validate().expect_err("wildcard");
        let msg = err.to_string();
        assert!(msg.contains("ssh -L 7457:127.0.0.1:7457"), "was: {msg}");
    }
}
