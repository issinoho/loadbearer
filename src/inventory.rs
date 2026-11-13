//! Machine inventory: the hardware and OS facts recorded alongside every result
//! so that two result files can be compared meaningfully.

use serde::{Deserialize, Serialize};
use sysinfo::{DiskKind, Disks, System};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub hostname: Option<String>,
    /// Firmware / OS identifiers, for telling repeated runs of the same machine
    /// apart from a machine not seen before. Absent when none could be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<crate::identity::MachineId>,
    /// System model and firmware version / date. Absent when none could be
    /// read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<crate::firmware::Firmware>,
    pub os: Option<String>,
    pub kernel: Option<String>,
    /// The OS release in parts — build, version, edition, distribution —
    /// beside the free-text `os` / `kernel`. Absent when none could be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_detail: Option<crate::os_detail::OsDetail>,
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
    /// Slots, populated slots, and each module's size, type and speed.
    /// Absent where the firmware table can't be read (Linux without root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<crate::ram::Memory>,
    pub disks: Vec<DiskInfo>,
    /// Health counters for each NVMe or SATA drive this process could query
    /// — empty without root / admin, or with no drive that reports them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drives: Vec<crate::drives::DriveHealth>,
    /// The GPU `loadbearer` would benchmark via OpenCL, if any was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<crate::benches::GpuInfo>,
    /// The primary battery and its wear state, on a machine that has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<crate::battery::BatteryInfo>,
    /// Mains or battery, and the power plan / mode / profile in force. Not
    /// graded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<crate::power::Power>,
    /// How old the machine is, as far as its CPU, firmware, drive and battery
    /// say. Not graded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<crate::age::Age>,
    /// When the machine last booted. Only with `--collect uptime`; absent
    /// otherwise, or when it couldn't be read (`run.collect` says which).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot: Option<crate::boot::Boot>,
    /// Firmware mode, Secure Boot, TPM, disk encryption. Only with
    /// `--collect security`; each part absent where it couldn't be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<crate::security::Security>,
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

impl Inventory {
    /// A placeholder inventory for a synthetic result (e.g. a model reference
    /// piped into `compare`). Only `hostname` / `cpu_model` are usually set by
    /// the caller.
    pub fn blank() -> Self {
        Inventory {
            hostname: None,
            identity: None,
            firmware: None,
            os: None,
            kernel: None,
            os_detail: None,
            arch: String::new(),
            cpu_model: String::new(),
            cpu_vendor: String::new(),
            cpu_physical_cores: None,
            cpu_logical_cores: 0,
            cpu_mhz_spot: 0,
            ram_bytes: 0,
            swap_bytes: 0,
            memory: None,
            disks: Vec::new(),
            drives: Vec::new(),
            gpu: None,
            battery: None,
            power: None,
            age: None,
            boot: None,
            security: None,
        }
    }

    /// Add the opt-in machine facts [`crate::optin::gather`] found. Elevation
    /// is not among them: it belongs to the run, not the machine.
    pub fn with_opt_in(mut self, gathered: &crate::optin::Gathered) -> Self {
        self.boot = gathered.boot.clone();
        self.security = gathered.security.clone();
        self
    }
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

    let mut inv = Inventory {
        hostname: System::host_name(),
        identity: crate::identity::collect(),
        firmware: crate::firmware::collect(),
        os: System::long_os_version().or_else(System::os_version),
        kernel: System::kernel_version(),
        os_detail: crate::os_detail::collect(),
        arch: System::cpu_arch(),
        cpu_model,
        cpu_vendor,
        cpu_physical_cores: System::physical_core_count(),
        cpu_logical_cores: cpus.len(),
        cpu_mhz_spot,
        ram_bytes: sys.total_memory(),
        swap_bytes: sys.total_swap(),
        memory: crate::ram::collect(),
        disks,
        drives: crate::drives::collect(),
        gpu: crate::benches::gpu_probe().cloned(),
        battery: crate::battery::probe().cloned(),
        power: crate::power::collect(),
        age: None,
        // Opt-in only: filled by `with_opt_in` from `optin::gather`, never here.
        boot: None,
        security: None,
    };
    inv.age = crate::age::Age::assess(&inv, crate::age::this_month());
    log::debug!(
        target: "loadbearer::inventory",
        // No hostname: a machine name is frequently a person's name, and the
        // diagnostic log is the artefact that gets pasted into bug reports.
        // Whether one was readable is all this line needs to say.
        "collected: {} · {} logical cores · {} MiB RAM · {} disk(s) · hostname={} · gpu={} · battery={}",
        inv.cpu_model,
        inv.cpu_logical_cores,
        inv.ram_bytes / (1024 * 1024),
        inv.disks.len(),
        inv.hostname.is_some(),
        inv.gpu.is_some(),
        inv.battery.is_some(),
    );
    inv
}
