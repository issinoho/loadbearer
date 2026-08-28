//! Machine inventory: the hardware and OS facts recorded alongside every result
//! so that two result files can be compared meaningfully.

use serde::{Deserialize, Serialize};
use sysinfo::{DiskKind, Disks, System};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_vendor: String,
    pub cpu_physical_cores: Option<usize>,
    pub cpu_logical_cores: usize,
    /// Spot CPU-frequency reading in MHz at inventory time; 0 when unavailable.
    /// This is a momentary sample, not a rated base clock.
    pub cpu_mhz_spot: u64,
    pub ram_bytes: u64,
    pub swap_bytes: u64,
    pub disks: Vec<DiskInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub file_system: String,
    /// "SSD", "HDD" or "Unknown".
    pub kind: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub removable: bool,
}

/// Collect a full inventory snapshot of the current machine.
pub fn collect() -> Inventory {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpus = sys.cpus();
    let cpu_model = cpus
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let cpu_vendor = cpus
        .first()
        .map(|c| c.vendor_id().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let cpu_mhz_spot = cpus.first().map(|c| c.frequency()).unwrap_or(0);

    let disks = Disks::new_with_refreshed_list()
        .list()
        .iter()
        // Skip pseudo-filesystems (binderfs, lxcfs, overlay mounts) that report
        // no capacity; they are noise for a hardware inventory.
        .filter(|d| d.total_space() > 0)
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().into_owned(),
            mount_point: d.mount_point().to_string_lossy().into_owned(),
            file_system: d.file_system().to_string_lossy().into_owned(),
            kind: match d.kind() {
                DiskKind::SSD => "SSD",
                DiskKind::HDD => "HDD",
                _ => "Unknown",
            }
            .to_string(),
            total_bytes: d.total_space(),
            available_bytes: d.available_space(),
            removable: d.is_removable(),
        })
        .collect();

    Inventory {
        hostname: System::host_name(),
        os: System::long_os_version().or_else(System::os_version),
        kernel: System::kernel_version(),
        arch: System::cpu_arch(),
        cpu_model,
        cpu_vendor,
        cpu_physical_cores: sys.physical_core_count(),
        cpu_logical_cores: cpus.len(),
        cpu_mhz_spot,
        ram_bytes: sys.total_memory(),
        swap_bytes: sys.total_swap(),
        disks,
    }
}
