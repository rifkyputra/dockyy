use std::path::{Path, PathBuf};

use crate::spec::slug;

/// Filesystem locations kuadrat writes to. Injectable so tests never touch `/etc`.
#[derive(Debug, Clone)]
pub struct Paths {
    pub quadlet_dir: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            quadlet_dir: PathBuf::from("/etc/containers/systemd"),
        }
    }
}

impl Paths {
    /// All paths relative to `root` — for tests and dry runs.
    pub fn rooted(root: &Path) -> Self {
        Self {
            quadlet_dir: root.join("containers/systemd"),
        }
    }
}

/// Path of the generated unit file for a workload name.
pub fn unit_path(paths: &Paths, spec_name: &str) -> PathBuf {
    paths
        .quadlet_dir
        .join(format!("{}.container", slug(spec_name)))
}
