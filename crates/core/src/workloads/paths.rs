use std::path::{Path, PathBuf};

use crate::spec::slug;

/// Namespace prefix on every artefact kuadrat creates.
///
/// It is on the unit *filename* — and therefore on the generated systemd service name and
/// the container name — so a workload called "nginx" can never collide with a hand-written
/// `nginx.container` or with the host's real `nginx.service`.
pub const UNIT_PREFIX: &str = "kuadrat-";

/// Filesystem locations kuadrat writes to. Injectable so tests never touch `/etc`.
#[derive(Debug, Clone)]
pub struct Paths {
    pub quadlet_dir: PathBuf,
    pub db_path: PathBuf,
    pub caddy_dir: PathBuf,
    /// Plain systemd units — task `.timer` files. A separate directory
    /// because Quadlet only processes its own extensions: systemd never
    /// reads a `.timer` out of the Quadlet dir.
    pub systemd_dir: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            quadlet_dir: PathBuf::from("/etc/containers/systemd"),
            db_path: PathBuf::from("/var/lib/kuadrat/kuadrat.db"),
            caddy_dir: PathBuf::from("/etc/caddy/kuadrat.d"),
            systemd_dir: PathBuf::from("/etc/systemd/system"),
        }
    }
}

impl Paths {
    /// All paths relative to `root` — for tests and dry runs.
    pub fn rooted(root: &Path) -> Self {
        Self {
            quadlet_dir: root.join("containers/systemd"),
            db_path: root.join("lib/kuadrat/kuadrat.db"),
            caddy_dir: root.join("caddy/kuadrat.d"),
            systemd_dir: root.join("systemd/system"),
        }
    }
}

/// Name of the generated unit, without extension. Also the systemd service name Quadlet
/// derives from the file, so this is what `systemctl start|stop|is-active` must be given.
pub fn unit_name(spec_name: &str) -> String {
    format!("{UNIT_PREFIX}{}", slug(spec_name))
}

/// Path of the generated unit file for a workload name.
pub fn unit_path(paths: &Paths, spec_name: &str) -> PathBuf {
    paths
        .quadlet_dir
        .join(format!("{}.container", unit_name(spec_name)))
}

/// The workload name behind a unit filename stem, or `None` if it is not one of ours.
pub fn spec_name_from_stem(stem: &str) -> Option<&str> {
    stem.strip_prefix(UNIT_PREFIX).filter(|s| !s.is_empty())
}

/// Unit name (no extension) for one scheduled task: `kuadrat-<app>-task-<task>`.
/// Also the container name and the systemd service the timer activates.
pub fn task_unit_name(spec_name: &str, task_name: &str) -> String {
    format!("{UNIT_PREFIX}{}-task-{}", slug(spec_name), slug(task_name))
}

/// The Quadlet `.container` file for one task.
pub fn task_container_path(paths: &Paths, spec_name: &str, task_name: &str) -> PathBuf {
    paths.quadlet_dir.join(format!(
        "{}.container",
        task_unit_name(spec_name, task_name)
    ))
}

/// The `.timer` for one task — in `systemd_dir`, where systemd will read it.
pub fn task_timer_path(paths: &Paths, spec_name: &str, task_name: &str) -> PathBuf {
    paths
        .systemd_dir
        .join(format!("{}.timer", task_unit_name(spec_name, task_name)))
}

/// The filename-stem prefix every one of an app's task units shares — the
/// scan key for pruning.
pub fn task_stem_prefix(spec_name: &str) -> String {
    format!("{UNIT_PREFIX}{}-task-", slug(spec_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_file_is_namespaced_so_it_cannot_clobber_a_foreign_unit() {
        let paths = Paths::rooted(Path::new("/root"));
        assert_eq!(
            unit_path(&paths, "nginx"),
            PathBuf::from("/root/containers/systemd/kuadrat-nginx.container")
        );
        assert_eq!(unit_name("My App"), "kuadrat-my-app");
    }

    #[test]
    fn stem_round_trips_only_for_kuadrat_units() {
        assert_eq!(spec_name_from_stem("kuadrat-alpha"), Some("alpha"));
        assert_eq!(spec_name_from_stem("nginx"), None);
        assert_eq!(spec_name_from_stem("kuadrat-"), None);
    }

    #[test]
    fn db_path_default_is_under_var_lib() {
        let paths = Paths::default();
        assert_eq!(
            paths.db_path,
            std::path::PathBuf::from("/var/lib/kuadrat/kuadrat.db")
        );
    }

    #[test]
    fn db_path_is_rerooted_for_tests() {
        let paths = Paths::rooted(std::path::Path::new("/tmp/kx"));
        assert_eq!(
            paths.db_path,
            std::path::PathBuf::from("/tmp/kx/lib/kuadrat/kuadrat.db")
        );
    }
}

#[cfg(test)]
mod tests_tasks {
    use super::*;

    #[test]
    fn task_units_are_prefixed_and_split_across_the_two_dirs() {
        let paths = Paths::rooted(Path::new("/r"));
        assert_eq!(
            task_container_path(&paths, "My Web", "Daily Cleanup"),
            PathBuf::from("/r/containers/systemd/kuadrat-my-web-task-daily-cleanup.container")
        );
        assert_eq!(
            task_timer_path(&paths, "My Web", "Daily Cleanup"),
            PathBuf::from("/r/systemd/system/kuadrat-my-web-task-daily-cleanup.timer")
        );
        assert_eq!(
            task_unit_name("My Web", "Daily Cleanup"),
            "kuadrat-my-web-task-daily-cleanup"
        );
    }
}
