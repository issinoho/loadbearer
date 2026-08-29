//! `loadbearer mem` — per-program memory use, in the style of `ps_mem`.
//!
//! Not a benchmark. A diagnostic snapshot, like `info`: which programs are
//! using the machine's RAM right now, grouped by program name, smallest first
//! so the biggest consumers sit next to the grand total.
//!
//! **Linux** reads `/proc/<pid>/smaps_rollup` and reports true **PSS**
//! (proportional set size): a page mapped by N processes counts 1/N toward
//! each, so the per-program totals sum to close to the RAM actually in use.
//! `Private` is a process's unshared pages; `Shared` is its proportional share
//! of the rest; `Private + Shared = PSS`. Reading another user's process needs
//! root — those are counted and reported, not silently dropped.
//!
//! **Windows** has no PSS. It reports each process's **working set** as "RAM
//! used" and splits it into `PrivateUsage` (private, capped at the working set)
//! and the remainder as an estimated "Shared". The report says which of the two
//! kinds of number it is showing.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;

use crate::cli::MemArgs;

/// One process's memory, before grouping.
struct ProcRow {
    name: String,
    private: u64,
    shared: u64,
    swap: u64,
}

/// One program's rolled-up memory use — every process of that name, summed.
#[derive(Debug, Clone, Serialize)]
pub struct ProgramMem {
    pub name: String,
    pub processes: usize,
    pub private_bytes: u64,
    pub shared_bytes: u64,
    /// Paged-out (swap) bytes, proportional where the platform allows. Always
    /// `0` where swap accounting isn't available (all of Windows).
    pub swap_bytes: u64,
}

impl ProgramMem {
    pub fn total_bytes(&self) -> u64 {
        self.private_bytes + self.shared_bytes
    }
}

/// How the numbers were obtained, so a PSS snapshot and a working-set snapshot
/// aren't quietly conflated. Each variant is constructed on exactly one
/// platform, so the other looks dead to the compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
pub enum Source {
    /// Linux: proportional set size from `/proc/<pid>/smaps_rollup`.
    Pss,
    /// Windows: working set, split into private / estimated-shared.
    WorkingSet,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemSnapshot {
    pub source: Source,
    /// Programs, sorted ascending by total — largest last, by the total line.
    pub programs: Vec<ProgramMem>,
    /// Processes that exist but couldn't be read (another user's, no
    /// privilege). The totals are short by their memory.
    pub unreadable: usize,
}

impl MemSnapshot {
    pub fn private_total(&self) -> u64 {
        self.programs.iter().map(|p| p.private_bytes).sum()
    }
    pub fn shared_total(&self) -> u64 {
        self.programs.iter().map(|p| p.shared_bytes).sum()
    }
    pub fn swap_total(&self) -> u64 {
        self.programs.iter().map(|p| p.swap_bytes).sum()
    }
    pub fn ram_total(&self) -> u64 {
        self.private_total() + self.shared_total()
    }
    pub fn has_swap(&self) -> bool {
        self.programs.iter().any(|p| p.swap_bytes > 0)
    }
}

pub fn execute(args: MemArgs) -> Result<()> {
    let snap = collect()?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&snap)?);
    } else {
        crate::output::print_mem(&snap, args.limit, args.swap);
    }
    Ok(())
}

/// Take a snapshot of per-program memory use on this machine.
pub fn collect() -> Result<MemSnapshot> {
    let (procs, unreadable) = platform::read()?;
    Ok(MemSnapshot {
        source: platform::SOURCE,
        programs: group(procs),
        unreadable,
    })
}

/// Roll per-process rows up by program name and sort ascending by total.
fn group(procs: Vec<ProcRow>) -> Vec<ProgramMem> {
    let mut by_name: BTreeMap<String, ProgramMem> = BTreeMap::new();
    for p in procs {
        let e = by_name.entry(p.name.clone()).or_insert_with(|| ProgramMem {
            name: p.name,
            processes: 0,
            private_bytes: 0,
            shared_bytes: 0,
            swap_bytes: 0,
        });
        e.processes += 1;
        e.private_bytes += p.private;
        e.shared_bytes += p.shared;
        e.swap_bytes += p.swap;
    }
    let mut programs: Vec<ProgramMem> = by_name.into_values().collect();
    programs.sort_by(|a, b| {
        a.total_bytes()
            .cmp(&b.total_bytes())
            .then_with(|| a.name.cmp(&b.name))
    });
    programs
}

// --- Linux: PSS from /proc/<pid>/smaps_rollup -----------------------------

#[cfg(target_os = "linux")]
mod platform {
    use std::fs;
    use std::io::ErrorKind;

    use super::{ProcRow, Source};

    pub const SOURCE: Source = Source::Pss;

    enum Outcome {
        Row(ProcRow),
        /// Kernel thread, exited mid-scan, or nothing resident — nothing to show.
        Skip,
        /// Exists but not readable without more privilege.
        Denied,
    }

    pub fn read() -> anyhow::Result<(Vec<ProcRow>, usize)> {
        let entries = fs::read_dir("/proc").map_err(|e| anyhow::anyhow!("reading /proc: {e}"))?;
        let mut rows = Vec::new();
        let mut unreadable = 0usize;
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            match read_one(pid) {
                Outcome::Row(r) => rows.push(r),
                Outcome::Skip => {}
                Outcome::Denied => unreadable += 1,
            }
        }
        Ok((rows, unreadable))
    }

    fn read_one(pid: u32) -> Outcome {
        let name = match program_name(pid) {
            Some(n) => n,
            None => return Outcome::Skip,
        };
        let rollup = match fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::PermissionDenied => return Outcome::Denied,
            Err(_) => return Outcome::Skip, // NotFound: kernel thread or gone
        };
        let Some((private, pss, swap)) = parse_rollup(&rollup) else {
            return Outcome::Skip;
        };
        if pss == 0 {
            return Outcome::Skip;
        }
        Outcome::Row(ProcRow {
            name,
            private,
            shared: pss.saturating_sub(private),
            swap,
        })
    }

    /// `(private, pss, swap)` in bytes from an `smaps_rollup` body. `private` is
    /// `Private_Clean + Private_Dirty`; `swap` prefers `SwapPss` over `Swap`.
    fn parse_rollup(body: &str) -> Option<(u64, u64, u64)> {
        let (mut private, mut pss, mut swap, mut swap_pss) = (0u64, 0u64, 0u64, 0u64);
        let mut saw_pss = false;
        for line in body.lines() {
            let Some((key, rest)) = line.split_once(':') else {
                continue;
            };
            let kb = rest
                .trim()
                .strip_suffix(" kB")
                .and_then(|n| n.trim().parse::<u64>().ok());
            let Some(kb) = kb else { continue };
            match key {
                "Private_Clean" | "Private_Dirty" => private += kb * 1024,
                "Pss" => {
                    pss = kb * 1024;
                    saw_pss = true;
                }
                "Swap" => swap = kb * 1024,
                "SwapPss" => swap_pss = kb * 1024,
                _ => {}
            }
        }
        saw_pss.then_some((private, pss, if swap_pss > 0 { swap_pss } else { swap }))
    }

    /// Display name for a pid: the basename of `cmdline[0]`, or `comm` in
    /// brackets for a kernel thread. `None` if the process is already gone.
    fn program_name(pid: u32) -> Option<String> {
        if let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline"))
            && let Some(first) = cmdline.split(|&b| b == 0).find(|s| !s.is_empty())
        {
            let raw = String::from_utf8_lossy(first);
            // Some programs (Firefox, Chrome) rewrite their process title,
            // which flattens argv into one space-joined blob in `cmdline`.
            // Take the part before the first whitespace, then the basename.
            let head = raw.split_whitespace().next().unwrap_or(&raw);
            let base = head.rsplit(['/', '\\']).next().unwrap_or(head).trim();
            if !base.is_empty() {
                return Some(base.to_string());
            }
        }
        let comm = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        let comm = comm.trim();
        (!comm.is_empty()).then(|| format!("[{comm}]"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const ROLLUP: &str = "\
55a0c0000000-7ffffffff000 ---p 00000000 00:00 0                          [rollup]
Rss:               24576 kB
Pss:               10240 kB
Pss_Dirty:          8000 kB
Shared_Clean:       6144 kB
Shared_Dirty:          0 kB
Private_Clean:       512 kB
Private_Dirty:      3584 kB
Referenced:        24576 kB
Anonymous:          3584 kB
Swap:                200 kB
SwapPss:             100 kB
Locked:                0 kB
";

        #[test]
        fn rollup_private_is_clean_plus_dirty_and_swap_prefers_pss() {
            let (private, pss, swap) = parse_rollup(ROLLUP).unwrap();
            assert_eq!(private, (512 + 3584) * 1024);
            assert_eq!(pss, 10240 * 1024);
            assert_eq!(swap, 100 * 1024); // SwapPss, not Swap
        }

        #[test]
        fn rollup_without_pss_is_none() {
            assert!(parse_rollup("Rss: 10 kB\nPrivate_Clean: 4 kB\n").is_none());
        }

        #[test]
        fn rollup_falls_back_to_swap_when_no_swappss() {
            let body = "Pss: 100 kB\nPrivate_Dirty: 40 kB\nSwap: 8 kB\n";
            let (_, _, swap) = parse_rollup(body).unwrap();
            assert_eq!(swap, 8 * 1024);
        }
    }
}

// --- Windows: working set, split via GetProcessMemoryInfo ----------------

#[cfg(windows)]
mod platform {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    use super::{ProcRow, Source};

    pub const SOURCE: Source = Source::WorkingSet;

    pub fn read() -> anyhow::Result<(Vec<ProcRow>, usize)> {
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_memory()),
        );
        sys.refresh_processes(ProcessesToUpdate::All, true);

        let mut rows = Vec::new();
        let mut unreadable = 0usize;
        for (pid, proc_) in sys.processes() {
            let name = proc_.name().to_string_lossy().to_string();
            match counters(pid.as_u32()) {
                Some((working_set, private)) => {
                    let private = private.min(working_set);
                    rows.push(ProcRow {
                        name,
                        private,
                        shared: working_set - private,
                        swap: 0,
                    });
                }
                None => {
                    // No handle (a protected process, no privilege): fall back
                    // to sysinfo's working set with no split.
                    let ws = proc_.memory();
                    if ws > 0 {
                        rows.push(ProcRow {
                            name,
                            private: ws,
                            shared: 0,
                            swap: 0,
                        });
                    } else {
                        unreadable += 1;
                    }
                }
            }
        }
        Ok((rows, unreadable))
    }

    /// `(working_set, private_usage)` in bytes, or `None` if the process can't
    /// be opened or queried.
    fn counters(pid: u32) -> Option<(u64, u64)> {
        unsafe {
            let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut c = PROCESS_MEMORY_COUNTERS_EX {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
                ..Default::default()
            };
            let ok = K32GetProcessMemoryInfo(
                handle,
                (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            );
            CloseHandle(handle);
            (ok != 0).then_some((c.WorkingSetSize as u64, c.PrivateUsage as u64))
        }
    }
}

// --- Other platforms ----------------------------------------------------

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::{ProcRow, Source};

    pub const SOURCE: Source = Source::WorkingSet;

    pub fn read() -> anyhow::Result<(Vec<ProcRow>, usize)> {
        anyhow::bail!("`loadbearer mem` is supported on Linux and Windows only")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, private: u64, shared: u64, swap: u64) -> ProcRow {
        ProcRow {
            name: name.to_string(),
            private,
            shared,
            swap,
        }
    }

    #[test]
    fn group_sums_and_counts_by_name() {
        let progs = group(vec![
            row("bash", 100, 50, 0),
            row("bash", 120, 40, 10),
            row("firefox", 900, 300, 0),
        ]);
        // ascending by total: bash (310) then firefox (1200)
        assert_eq!(progs.len(), 2);
        assert_eq!(progs[0].name, "bash");
        assert_eq!(progs[0].processes, 2);
        assert_eq!(progs[0].private_bytes, 220);
        assert_eq!(progs[0].shared_bytes, 90);
        assert_eq!(progs[0].swap_bytes, 10);
        assert_eq!(progs[1].name, "firefox");
        assert_eq!(progs[1].total_bytes(), 1200);
    }

    #[test]
    fn snapshot_totals_add_up() {
        let snap = MemSnapshot {
            source: Source::Pss,
            programs: group(vec![row("a", 10, 5, 1), row("b", 20, 5, 0)]),
            unreadable: 3,
        };
        assert_eq!(snap.private_total(), 30);
        assert_eq!(snap.shared_total(), 10);
        assert_eq!(snap.ram_total(), 40);
        assert_eq!(snap.swap_total(), 1);
        assert!(snap.has_swap());
    }
}
