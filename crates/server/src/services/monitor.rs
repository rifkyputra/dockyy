use serde::{Deserialize, Serialize};
use std::sync::Arc;
use sysinfo::{Disks, System};
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

/// Snapshot of host system health metrics.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SystemMetrics {
    /// 0–100 percentage
    pub cpu_usage_pct: f32,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    /// Root filesystem (or largest disk) used bytes
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub docker_ok: bool,
    /// RFC-3339 timestamp of last successful collection
    pub checked_at: String,
}

pub type MetricsState = Arc<RwLock<SystemMetrics>>;

pub fn new_metrics_state() -> MetricsState {
    Arc::new(RwLock::new(SystemMetrics::default()))
}

pub async fn run_monitor(state: Arc<crate::AppState>) {
    tracing::info!("Starting health monitor (interval: 30s)");

    loop {
        // Collect CPU/RAM/disk on a blocking thread so we don't stall the async runtime.
        let sys_metrics = tokio::task::spawn_blocking(collect_system_metrics)
            .await
            .unwrap_or_default();

        // Check Docker connectivity in async context.
        let docker_ok = state.docker.list_containers(false).await.is_ok();

        let updated = SystemMetrics {
            docker_ok,
            checked_at: chrono::Utc::now().to_rfc3339(),
            ..sys_metrics
        };

        tracing::debug!(
            cpu = updated.cpu_usage_pct,
            mem_used = updated.mem_used_bytes,
            mem_total = updated.mem_total_bytes,
            disk_used = updated.disk_used_bytes,
            disk_total = updated.disk_total_bytes,
            docker_ok = updated.docker_ok,
            "Health metrics collected"
        );

        *state.metrics.write().await = updated;

        sleep(Duration::from_secs(30)).await;
    }
}

/// Synchronous metric collection — runs inside `spawn_blocking`.
fn collect_system_metrics() -> SystemMetrics {
    let mut sys = System::new();

    // Two refreshes with a small pause for accurate CPU delta.
    sys.refresh_cpu_usage();
    std::thread::sleep(std::time::Duration::from_millis(200));
    sys.refresh_cpu_usage();

    sys.refresh_memory();

    let cpu_usage_pct = sys.global_cpu_usage();
    let mem_used_bytes = sys.used_memory();
    let mem_total_bytes = sys.total_memory();

    // Prefer the root filesystem; fall back to summing all disks.
    let disks = Disks::new_with_refreshed_list();
    let root = std::path::Path::new("/");
    let (disk_used_bytes, disk_total_bytes) = disks
        .iter()
        .find(|d| d.mount_point() == root)
        .map(|d| {
            let total = d.total_space();
            let used = total.saturating_sub(d.available_space());
            (used, total)
        })
        .unwrap_or_else(|| {
            disks.iter().fold((0u64, 0u64), |(used, total), d| {
                let t = d.total_space();
                let u = t.saturating_sub(d.available_space());
                (used + u, total + t)
            })
        });

    SystemMetrics {
        cpu_usage_pct,
        mem_used_bytes,
        mem_total_bytes,
        disk_used_bytes,
        disk_total_bytes,
        // filled in by the async caller
        docker_ok: false,
        checked_at: String::new(),
    }
}
