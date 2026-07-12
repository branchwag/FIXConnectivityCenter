//! Host resource metrics (CPU, memory, swap, disk, load, uptime) for the
//! machine-health monitoring page.
//!
//! A single [`System`] is kept alive and refreshed on a background tick rather
//! than rebuilt per request: CPU usage is a *delta* between two refreshes, so
//! the steady 2s cadence is what makes the percentages meaningful (a fresh
//! `System` would always read 0%). The `/tools/metrics` handler just reads the
//! latest snapshot.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use sysinfo::{Disks, System};

/// How often the background task refreshes the host stats. Also the spacing that
/// gives CPU usage its delta, so keep it comfortably above sysinfo's minimum.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct MetricsState {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    sys: System,
    disks: Disks,
}

impl MetricsState {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        let disks = Disks::new_with_refreshed_list();
        Self {
            inner: Arc::new(Mutex::new(Inner { sys, disks })),
        }
    }

    /// Refresh CPU, memory and disk usage in place. Cheap; runs on the tick.
    fn refresh(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.sys.refresh_cpu_usage();
        inner.sys.refresh_memory();
        inner.disks.refresh(true);
    }

    /// Build a serializable snapshot from the latest refreshed data.
    pub fn snapshot(&self) -> Metrics {
        let inner = self.inner.lock().unwrap();
        let sys = &inner.sys;

        let cores: Vec<f32> = sys.cpus().iter().map(|c| round1(c.cpu_usage())).collect();
        let load = System::load_average();

        let disks = inner
            .disks
            .iter()
            .map(|d| {
                let total = d.total_space();
                let available = d.available_space();
                let used = total.saturating_sub(available);
                Disk {
                    mount: d.mount_point().to_string_lossy().into_owned(),
                    file_system: d.file_system().to_string_lossy().into_owned(),
                    total,
                    used,
                    available,
                    used_pct: pct(used, total),
                }
            })
            .collect();

        Metrics {
            cpu: Cpu {
                usage: round1(sys.global_cpu_usage()),
                cores,
            },
            memory: Mem {
                used: sys.used_memory(),
                total: sys.total_memory(),
                used_pct: pct(sys.used_memory(), sys.total_memory()),
            },
            swap: Mem {
                used: sys.used_swap(),
                total: sys.total_swap(),
                used_pct: pct(sys.used_swap(), sys.total_swap()),
            },
            load: Load {
                one: round2(load.one),
                five: round2(load.five),
                fifteen: round2(load.fifteen),
            },
            uptime_secs: System::uptime(),
            disks,
            host: Host {
                name: System::host_name().unwrap_or_else(|| "unknown".into()),
                os: System::long_os_version().unwrap_or_else(|| "unknown".into()),
                kernel: System::kernel_version().unwrap_or_else(|| "unknown".into()),
                arch: System::cpu_arch(),
                cpu_count: sys.cpus().len(),
            },
        }
    }
}

/// Spawn the background refresh loop. Runs for the life of the process.
pub fn spawn_refresher(state: MetricsState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(REFRESH_INTERVAL).await;
            state.refresh();
        }
    });
}

#[derive(Serialize)]
pub struct Metrics {
    pub cpu: Cpu,
    pub memory: Mem,
    pub swap: Mem,
    pub load: Load,
    pub uptime_secs: u64,
    pub disks: Vec<Disk>,
    pub host: Host,
}

#[derive(Serialize)]
pub struct Cpu {
    /// Global CPU utilization, percent.
    pub usage: f32,
    /// Per-core utilization, percent.
    pub cores: Vec<f32>,
}

#[derive(Serialize)]
pub struct Mem {
    /// Bytes.
    pub used: u64,
    /// Bytes.
    pub total: u64,
    pub used_pct: f32,
}

#[derive(Serialize)]
pub struct Load {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

#[derive(Serialize)]
pub struct Disk {
    pub mount: String,
    pub file_system: String,
    /// Bytes.
    pub total: u64,
    /// Bytes.
    pub used: u64,
    /// Bytes.
    pub available: u64,
    pub used_pct: f32,
}

#[derive(Serialize)]
pub struct Host {
    pub name: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub cpu_count: usize,
}

/// Used / total as a percentage, guarding against divide-by-zero (e.g. no swap).
fn pct(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        round1(used as f32 / total as f32 * 100.0)
    }
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
