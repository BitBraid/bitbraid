//! How much memory the process holds, for the measuring runs
//! (`BITBRAID_BENCH`, `cargo xtask appbench`): the budget is on the most
//! the application held with a million commits open, and that is a number
//! only the system knows.
//!
//! On Linux the kernel keeps the figure; on macOS the system's `footprint`
//! tells it: the memory the process is charged for (`phys_footprint`, what
//! Activity Monitor shows as its memory) and the most it ever was. On
//! Windows the system keeps the working set and its peak, Linux's resident
//! set and its peak by another name, and the system's PowerShell reads them
//! out (`Get-Process`), since asking the system straight would take code
//! the workspace forbids. Each is a program started once, at the end of a
//! measuring run. Elsewhere there is none and the measuring run says so by
//! leaving it out.
//!
//! Sources: proc(5) (`/proc/[pid]/status`: `VmHWM`, `VmRSS`); macOS
//! footprint(1) (`phys_footprint`, `phys_footprint_peak`); Microsoft
//! Learn, "Process.PeakWorkingSet64", "Process.WorkingSet64",
//! "Get-Process" and "Process Memory Usage Information"
//! (`PeakWorkingSetSize`, `WorkingSetSize`).

/// Memory the process holds in physical pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resident {
    /// The most it ever held, in kibibytes.
    pub peak_kib: u64,
    /// What it holds now, in kibibytes.
    pub now_kib: u64,
}

impl Resident {
    /// The peak in mebibytes, rounded down.
    pub fn peak_mib(&self) -> u64 {
        self.peak_kib / 1024
    }

    /// What is held now in mebibytes, rounded down.
    pub fn now_mib(&self) -> u64 {
        self.now_kib / 1024
    }
}

/// What this process holds, where the system tells.
pub fn resident() -> Option<Resident> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/footprint")
            .arg(std::process::id().to_string())
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        parse_footprint(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(windows)]
    {
        // The system's own PowerShell, by its place, not whatever `PATH`
        // finds first under that name.
        let root = std::env::var_os("SystemRoot")?;
        let powershell = std::path::Path::new(&root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let script = format!(
            "$p = Get-Process -Id {}; \"$($p.PeakWorkingSet64) $($p.WorkingSet64)\"", // not shown
            std::process::id()
        );
        let mut command = std::process::Command::new(powershell);
        command
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let out = bitbraid_core::process::no_window(&mut command)
            .output()
            .ok()?;
        parse_working_set(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        // A few kilobytes of text; anything else at that name is not read.
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        parse(&status)
    }
}

/// The two figures out of what the script of [`resident`] writes on
/// Windows: the peak working set and the working set, in bytes, on one
/// line.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_working_set(text: &str) -> Option<Resident> {
    let mut parts = text.split_whitespace();
    let peak = parts.next()?.parse::<u64>().ok()?;
    let now = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(Resident {
        peak_kib: peak / 1024,
        now_kib: now / 1024,
    })
}

/// The two figures out of what macOS's `footprint` writes:
/// `phys_footprint: 84 MB` and `phys_footprint_peak: 90 MB`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_footprint(text: &str) -> Option<Resident> {
    let field = |name: &str| -> Option<u64> {
        text.lines().find_map(|line| {
            let rest = line.trim().strip_prefix(name)?.strip_prefix(':')?;
            let mut parts = rest.split_whitespace();
            let number = parts.next()?.parse::<f64>().ok()?;
            let kib = match parts.next()? {
                "B" => number / 1024.0,
                "KB" => number,
                "MB" => number * 1024.0,
                "GB" => number * 1024.0 * 1024.0,
                _ => return None,
            };
            // NaN is in no range.
            (0.0..1e15).contains(&kib).then_some(kib.round() as u64)
        })
    };
    Some(Resident {
        peak_kib: field("phys_footprint_peak")?,
        now_kib: field("phys_footprint")?,
    })
}

/// The two figures out of the text of `/proc/<pid>/status`.
#[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
fn parse(status: &str) -> Option<Resident> {
    let field = |name: &str| -> Option<u64> {
        status.lines().find_map(|line| {
            let rest = line.strip_prefix(name)?.strip_prefix(':')?;
            let mut parts = rest.split_whitespace();
            let number = parts.next()?.parse::<u64>().ok()?;
            // The kernel writes kibibytes and says so.
            (parts.next() == Some("kB")).then_some(number)
        })
    };
    Some(Resident {
        peak_kib: field("VmHWM")?,
        now_kib: field("VmRSS")?,
    })
}

#[cfg(test)]
mod tests {
    use super::{Resident, parse};

    #[test]
    fn the_figures_are_read_from_the_status_text() {
        let status = "Name:\tbitbraid\nVmPeak:\t 9000000 kB\nVmSize:\t 8000000 kB\n\
                      VmHWM:\t  524288 kB\nVmRSS:\t  262144 kB\nThreads:\t12\n";
        let read = parse(status).expect("both are there");
        assert_eq!(
            read,
            Resident {
                peak_kib: 524_288,
                now_kib: 262_144
            }
        );
        assert_eq!((read.peak_mib(), read.now_mib()), (512, 256));
    }

    #[test]
    fn text_that_is_not_a_status_gives_nothing() {
        for hostile in [
            "",
            "VmHWM:",
            "VmHWM:\tmany kB\nVmRSS:\t1 kB\n",
            "VmHWM:\t1 kB\n",
            "VmRSS:\t1 kB\n",
            "VmHWM:\t1 MB\nVmRSS:\t1 kB\n",
            "VmHWM:\t-1 kB\nVmRSS:\t1 kB\n",
            "VmHWM:\t99999999999999999999999 kB\nVmRSS:\t1 kB\n",
            "VmHWMx:\t1 kB\nVmRSS:\t1 kB\n",
        ] {
            assert_eq!(parse(hostile), None, "{hostile:?}");
        }
        // The largest number there is does not overflow on the way out.
        let huge = format!("VmHWM:\t{0} kB\nVmRSS:\t{0} kB\n", u64::MAX);
        assert_eq!(parse(&huge).map(|r| r.peak_mib()), Some(u64::MAX / 1024));
    }

    #[test]
    fn macos_footprint_is_read() {
        let text = "bitbraid [62306]: 64-bit    Footprint: 84 MB (16384 bytes per page)\n\
                    \n    phys_footprint: 84 MB\n    phys_footprint_peak: 1.5 GB\n";
        let read = super::parse_footprint(text).expect("both are there");
        assert_eq!(read.now_mib(), 84);
        assert_eq!(read.peak_mib(), 1536);
        for hostile in [
            "",
            "    phys_footprint: 84 MB\n",
            "    phys_footprint_peak: 9 MB\n",
            "    phys_footprint: 84 XB\n    phys_footprint_peak: 9 MB\n",
            "    phys_footprint: -4 MB\n    phys_footprint_peak: 9 MB\n",
            "    phys_footprint: NaN MB\n    phys_footprint_peak: 9 MB\n",
        ] {
            assert_eq!(super::parse_footprint(hostile), None, "{hostile:?}");
        }
    }

    #[test]
    fn the_working_set_is_read_from_powershells_line() {
        let read = super::parse_working_set("1610612736 268435456\r\n").expect("both are there");
        assert_eq!((read.peak_mib(), read.now_mib()), (1536, 256));
        for hostile in [
            "",
            "\r\n",
            "1610612736",
            "1610612736 many",
            "-1 268435456",
            "1610612736 268435456 7",
            "99999999999999999999999 1",
            "Get-Process : Cannot find a process",
        ] {
            assert_eq!(super::parse_working_set(hostile), None, "{hostile:?}");
        }
        let huge = format!("{0} {0}", u64::MAX);
        assert_eq!(
            super::parse_working_set(&huge).map(|r| r.peak_kib),
            Some(u64::MAX / 1024)
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn this_process_holds_something_and_never_more_than_its_peak() {
        let read = super::resident().expect("the system tells");
        assert!(read.now_kib > 0);
        assert!(read.peak_kib >= read.now_kib);
    }
}
