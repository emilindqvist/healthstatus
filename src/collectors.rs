use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::process::Command;
use std::time::Instant;

use crate::{parse_nv_number, unix_now_secs};

#[derive(Clone, Debug, Default)]
pub struct Metrics {
    pub host: Host,
    pub cpu: Cpu,
    pub memory: Memory,
    pub battery: Option<Battery>,
    pub disks: Vec<Disk>,
    pub network: Network,
    pub processes: Vec<ProcessInfo>,
    pub temperatures: Vec<Temperature>,
}

#[derive(Clone, Debug, Default)]
pub struct Host {
    pub user: String,
    pub hostname: String,
    pub os: String,
    pub distro: Option<String>,
    pub arch: String,
    pub uptime_s: f64,
    pub is_wsl: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Cpu {
    pub percent_total: f64,
    pub percent_per_core: Vec<f64>,
    pub logical_cores: usize,
    pub physical_cores: Option<usize>,
    pub freq_mhz: Option<f64>,
    pub load_avg: (f64, f64, f64),
}

#[derive(Clone, Debug, Default)]
pub struct Memory {
    pub ram_total: u64,
    pub ram_used: u64,
    pub ram_available: u64,
    pub ram_percent: f64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub swap_percent: f64,
}

#[derive(Clone, Debug)]
pub struct Battery {
    pub percent: Option<f64>,
    pub charging: Option<bool>,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct Disk {
    pub mount: String,
    pub device: String,
    pub fstype: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub percent: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Network {
    pub interfaces: Vec<NetworkInterface>,
}

#[derive(Clone, Debug)]
pub struct NetworkInterface {
    pub name: String,
    pub up_bps: f64,
    pub down_bps: f64,
    pub total_sent: u64,
    pub total_recv: u64,
}

#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_ticks: u64,
    pub memory_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Temperature {
    pub chip: String,
    pub label: String,
    pub current: f64,
    pub high: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct NetSnapshot {
    time: Option<Instant>,
    counters: HashMap<String, (u64, u64)>,
}

#[derive(Clone, Debug, Default)]
pub struct GpuTelemetry {
    pub available: bool,
    pub gpus: Vec<Gpu>,
    pub processes: Vec<GpuProcess>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Gpu {
    pub name: String,
    pub temp_c: Option<f64>,
    pub gpu_util_pct: Option<f64>,
    pub mem_util_pct: Option<f64>,
    pub vram_used_mib: Option<f64>,
    pub vram_total_mib: Option<f64>,
    pub fan_pct: Option<f64>,
    pub power_w: Option<f64>,
    pub power_limit_w: Option<f64>,
    pub clock_core_mhz: Option<f64>,
    pub clock_mem_mhz: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct GpuProcess {
    pub pid: String,
    pub name: String,
    pub mem_mib: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct SystemDetails {
    pub wsl: bool,
    pub kernel: String,
    pub distro: Option<String>,
    pub arch: String,
    pub cpu_model: Option<String>,
    pub vm_ram_total: u64,
    pub windows: Vec<(String, String)>,
    pub wifi: Vec<(String, String)>,
}

pub fn collect_all(prev_net: &mut Option<NetSnapshot>) -> Metrics {
    Metrics {
        host: host(),
        cpu: cpu(),
        memory: memory(),
        battery: battery(),
        disks: disks(),
        network: network(prev_net),
        processes: processes(8),
        temperatures: temperatures(),
    }
}

pub fn is_wsl() -> bool {
    fs::read_to_string("/proc/version")
        .map(|s| s.to_ascii_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

fn host() -> Host {
    let uptime_s = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0);
    Host {
        user: env::var("USER")
            .or_else(|_| env::var("USERNAME"))
            .unwrap_or_else(|_| "?".to_string()),
        hostname: fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| command_text("hostname", &[]).unwrap_or_else(|| "?".to_string())),
        os: format!(
            "{} {}",
            env::consts::OS,
            command_text("uname", &["-r"]).unwrap_or_default()
        ),
        distro: linux_pretty_name(),
        arch: command_text("uname", &["-m"]).unwrap_or_else(|| env::consts::ARCH.to_string()),
        uptime_s,
        is_wsl: is_wsl(),
    }
}

fn cpu() -> Cpu {
    let samples = read_proc_stat_cpu();
    let total = samples
        .first()
        .map(|sample| cpu_percent_from_sample(sample))
        .unwrap_or(0.0);
    let per_core = samples
        .iter()
        .skip(1)
        .map(|sample| cpu_percent_from_sample(sample))
        .collect::<Vec<_>>();
    let logical = per_core.len().max(1);
    Cpu {
        percent_total: total,
        percent_per_core: per_core,
        logical_cores: logical,
        physical_cores: cpu_physical_cores(),
        freq_mhz: cpu_freq_mhz(),
        load_avg: load_avg(),
    }
}

fn read_proc_stat_cpu() -> Vec<Vec<u64>> {
    fs::read_to_string("/proc/stat")
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("cpu"))
        .map(|line| {
            line.split_whitespace()
                .skip(1)
                .filter_map(|v| v.parse().ok())
                .collect()
        })
        .collect()
}

fn cpu_percent_from_sample(values: &[u64]) -> f64 {
    if values.len() < 5 {
        return 0.0;
    }
    let idle = values.get(3).copied().unwrap_or(0) + values.get(4).copied().unwrap_or(0);
    let total: u64 = values.iter().sum();
    if total == 0 {
        0.0
    } else {
        ((total.saturating_sub(idle)) as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    }
}

fn memory() -> Memory {
    let mem = meminfo();
    let total = kib(&mem, "MemTotal");
    let available = kib(&mem, "MemAvailable");
    let free = kib(&mem, "MemFree");
    let used = total.saturating_sub(if available > 0 { available } else { free });
    let swap_total = kib(&mem, "SwapTotal");
    let swap_free = kib(&mem, "SwapFree");
    let swap_used = swap_total.saturating_sub(swap_free);
    Memory {
        ram_total: total,
        ram_used: used,
        ram_available: available,
        ram_percent: percent(used, total),
        swap_total,
        swap_used,
        swap_percent: percent(swap_used, swap_total),
    }
}

fn disks() -> Vec<Disk> {
    let real_fs = HashSet::from([
        "ext2", "ext3", "ext4", "xfs", "btrfs", "zfs", "f2fs", "ntfs", "ntfs3", "vfat", "fat32",
        "exfat", "apfs", "9p", "drvfs", "fuseblk", "cifs", "nfs", "nfs4",
    ]);
    let output = Command::new("df")
        .args(["-B1", "-T", "-P"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() < 7 || !real_fs.contains(parts[1]) {
                return None;
            }
            let total = parts[2].parse::<u64>().ok()?;
            let used = parts[3].parse::<u64>().ok()?;
            let free = parts[4].parse::<u64>().ok()?;
            Some(Disk {
                device: parts[0].to_string(),
                fstype: parts[1].to_string(),
                total,
                used,
                free,
                percent: percent(used, total),
                mount: parts[6..].join(" "),
            })
        })
        .collect()
}

fn network(prev: &mut Option<NetSnapshot>) -> Network {
    let now = Instant::now();
    let counters = read_net_dev();
    let interfaces = counters
        .iter()
        .map(|(name, (recv, sent))| {
            let (down_bps, up_bps) = prev
                .as_ref()
                .and_then(|p| {
                    let dt = p
                        .time
                        .map(|t| now.duration_since(t).as_secs_f64())
                        .unwrap_or(0.0);
                    let (old_recv, old_sent) = p.counters.get(name).copied()?;
                    if dt > 0.0 {
                        Some((
                            recv.saturating_sub(old_recv) as f64 / dt,
                            sent.saturating_sub(old_sent) as f64 / dt,
                        ))
                    } else {
                        None
                    }
                })
                .unwrap_or((0.0, 0.0));
            NetworkInterface {
                name: name.clone(),
                down_bps,
                up_bps,
                total_recv: *recv,
                total_sent: *sent,
            }
        })
        .collect();
    *prev = Some(NetSnapshot {
        time: Some(now),
        counters,
    });
    Network { interfaces }
}

fn processes(top_n: usize) -> Vec<ProcessInfo> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let base = entry.path();
        let name = fs::read_to_string(base.join("comm"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "?".to_string());
        let stat = fs::read_to_string(base.join("stat")).unwrap_or_default();
        let parts = stat.split_whitespace().collect::<Vec<_>>();
        let cpu_ticks = parts
            .get(13)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            + parts
                .get(14)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
        let memory_bytes = fs::read_to_string(base.join("status"))
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|line| line.starts_with("VmRSS:"))
                    .and_then(|line| line.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|kib| kib * 1024)
            })
            .unwrap_or(0);
        out.push(ProcessInfo {
            pid,
            name,
            cpu_ticks,
            memory_bytes,
        });
    }
    out.sort_by_key(|p| std::cmp::Reverse((p.cpu_ticks, p.memory_bytes)));
    out.truncate(top_n);
    out
}

fn battery() -> Option<Battery> {
    if is_wsl() {
        return battery_from_windows();
    }
    let entries = fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let dir = entry.path();
        if fs::read_to_string(dir.join("type")).ok()?.trim() != "Battery" {
            continue;
        }
        let percent = fs::read_to_string(dir.join("capacity"))
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok());
        let charging = fs::read_to_string(dir.join("status"))
            .ok()
            .map(|s| matches!(s.trim(), "Charging" | "Full"));
        return Some(Battery {
            percent,
            charging,
            source: "sysfs".to_string(),
        });
    }
    None
}

fn battery_from_windows() -> Option<Battery> {
    let text = command_text(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_Battery | ForEach-Object { \"$($_.EstimatedChargeRemaining),$($_.BatteryStatus)\" } | Select-Object -First 1",
        ],
    )?;
    let parts = text.trim().split(',').collect::<Vec<_>>();
    let percent = parts.first().and_then(|v| v.trim().parse::<f64>().ok());
    let charging = parts
        .get(1)
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(|status| matches!(status, 2 | 6 | 7 | 8 | 9));
    Some(Battery {
        percent,
        charging,
        source: "windows".to_string(),
    })
}

fn temperatures() -> Vec<Temperature> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/thermal") else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let temp = fs::read_to_string(dir.join("temp"))
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .map(|v| v / 1000.0);
        if let Some(current) = temp {
            let label = fs::read_to_string(dir.join("type"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "thermal".to_string());
            out.push(Temperature {
                chip: "thermal".to_string(),
                label,
                current,
                high: None,
            });
        }
    }
    out
}

pub fn gpu_telemetry() -> GpuTelemetry {
    let Some(exe) = find_command(&["nvidia-smi", "nvidia-smi.exe"]) else {
        return GpuTelemetry {
            error: Some("nvidia-smi not found".to_string()),
            ..GpuTelemetry::default()
        };
    };
    let fields = [
        "name",
        "temperature.gpu",
        "utilization.gpu",
        "utilization.memory",
        "memory.used",
        "memory.total",
        "fan.speed",
        "power.draw",
        "power.limit",
        "clocks.gr",
        "clocks.mem",
    ];
    let output = Command::new(&exe)
        .args([
            format!("--query-gpu={}", fields.join(",")),
            "--format=csv,noheader,nounits".to_string(),
        ])
        .output();
    let Ok(output) = output else {
        return GpuTelemetry {
            error: Some("nvidia-smi failed to start".to_string()),
            ..GpuTelemetry::default()
        };
    };
    if !output.status.success() {
        return GpuTelemetry {
            error: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            ..GpuTelemetry::default()
        };
    }
    let mut gpus = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts = line.split(',').map(str::trim).collect::<Vec<_>>();
        if parts.len() != fields.len() {
            continue;
        }
        gpus.push(Gpu {
            name: parts[0].to_string(),
            temp_c: parse_nv_number(parts[1]),
            gpu_util_pct: parse_nv_number(parts[2]),
            mem_util_pct: parse_nv_number(parts[3]),
            vram_used_mib: parse_nv_number(parts[4]),
            vram_total_mib: parse_nv_number(parts[5]),
            fan_pct: parse_nv_number(parts[6]),
            power_w: parse_nv_number(parts[7]),
            power_limit_w: parse_nv_number(parts[8]),
            clock_core_mhz: parse_nv_number(parts[9]),
            clock_mem_mhz: parse_nv_number(parts[10]),
        });
    }
    let processes = gpu_processes(&exe);
    GpuTelemetry {
        available: !gpus.is_empty(),
        gpus,
        processes,
        error: None,
    }
}

fn gpu_processes(exe: &str) -> Vec<GpuProcess> {
    let output = Command::new(exe)
        .args([
            "--query-compute-apps=pid,process_name,used_memory",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    output
        .lines()
        .filter_map(|line| {
            let parts = line.split(',').map(str::trim).collect::<Vec<_>>();
            (parts.len() == 3).then(|| GpuProcess {
                pid: parts[0].to_string(),
                name: parts[1].to_string(),
                mem_mib: parse_nv_number(parts[2]),
            })
        })
        .collect()
}

pub fn system_details() -> SystemDetails {
    let mut details = SystemDetails {
        wsl: is_wsl(),
        kernel: command_text("uname", &["-r"]).unwrap_or_default(),
        distro: linux_pretty_name(),
        arch: command_text("uname", &["-m"]).unwrap_or_default(),
        cpu_model: cpu_model_linux(),
        vm_ram_total: memory().ram_total,
        windows: Vec::new(),
        wifi: Vec::new(),
    };
    if details.wsl {
        details.windows = powershell_pairs(
            "$cs=Get-CimInstance Win32_ComputerSystem; $cpu=Get-CimInstance Win32_Processor | Select-Object -First 1; $os=Get-CimInstance Win32_OperatingSystem; $bios=Get-CimInstance Win32_BIOS; \"Manufacturer=$($cs.Manufacturer)\"; \"Model=$($cs.Model)\"; \"Windows=$($os.Caption)\"; \"Build=$($os.BuildNumber)\"; \"CPU=$($cpu.Name)\"; \"Cores=$($cpu.NumberOfCores)c/$($cpu.NumberOfLogicalProcessors)t\"; \"BIOS=$($bios.SMBIOSBIOSVersion)\"",
        );
        details.wifi = netsh_wifi_pairs();
    }
    details
}

fn meminfo() -> HashMap<String, u64> {
    fs::read_to_string("/proc/meminfo")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let key = parts.next()?.trim_end_matches(':').to_string();
            let value = parts.next()?.parse::<u64>().ok()? * 1024;
            Some((key, value))
        })
        .collect()
}

fn kib(mem: &HashMap<String, u64>, key: &str) -> u64 {
    mem.get(key).copied().unwrap_or(0)
}

fn percent(used: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        used as f64 / total as f64 * 100.0
    }
}

fn linux_pretty_name() -> Option<String> {
    fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|content| {
            content.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|s| s.trim_matches('"').to_string())
            })
        })
}

fn cpu_model_linux() -> Option<String> {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|content| {
            content.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|s| s.split_once(':').map(|(_, value)| value.trim().to_string()))
            })
        })
}

fn cpu_physical_cores() -> Option<usize> {
    let content = fs::read_to_string("/proc/cpuinfo").ok()?;
    let mut cores = HashSet::new();
    for block in content.split("\n\n") {
        let physical = block.lines().find_map(|l| {
            l.strip_prefix("physical id")
                .and_then(|s| s.split_once(':').map(|(_, v)| v.trim()))
        });
        let core = block.lines().find_map(|l| {
            l.strip_prefix("core id")
                .and_then(|s| s.split_once(':').map(|(_, v)| v.trim()))
        });
        if let (Some(physical), Some(core)) = (physical, core) {
            cores.insert(format!("{physical}:{core}"));
        }
    }
    (!cores.is_empty()).then_some(cores.len())
}

fn cpu_freq_mhz() -> Option<f64> {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|content| {
            content.lines().find_map(|line| {
                line.strip_prefix("cpu MHz")
                    .and_then(|s| s.split_once(':'))
                    .and_then(|(_, value)| value.trim().parse::<f64>().ok())
            })
        })
}

fn load_avg() -> (f64, f64, f64) {
    let content = fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let mut parts = content
        .split_whitespace()
        .filter_map(|v| v.parse::<f64>().ok());
    (
        parts.next().unwrap_or(0.0),
        parts.next().unwrap_or(0.0),
        parts.next().unwrap_or(0.0),
    )
}

fn read_net_dev() -> HashMap<String, (u64, u64)> {
    fs::read_to_string("/proc/net/dev")
        .unwrap_or_default()
        .lines()
        .skip(2)
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let parts = rest.split_whitespace().collect::<Vec<_>>();
            let recv = parts.first()?.parse::<u64>().ok()?;
            let sent = parts.get(8)?.parse::<u64>().ok()?;
            Some((name.trim().to_string(), (recv, sent)))
        })
        .collect()
}

fn powershell_pairs(script: &str) -> Vec<(String, String)> {
    command_text(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    )
    .unwrap_or_default()
    .lines()
    .filter_map(|line| {
        let (k, v) = line.split_once('=')?;
        Some((k.trim().to_string(), v.trim().to_string()))
    })
    .collect()
}

fn netsh_wifi_pairs() -> Vec<(String, String)> {
    let text = command_text("netsh.exe", &["wlan", "show", "interfaces"]).unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            let key = k.trim();
            let label = match key {
                "SSID" => "SSID",
                "State" => "State",
                "Signal" => "Signal",
                "Radio type" => "Radio",
                "Receive rate (Mbps)" => "Rx Mbps",
                "Transmit rate (Mbps)" => "Tx Mbps",
                _ => return None,
            };
            Some((label.to_string(), v.trim().to_string()))
        })
        .collect()
}

fn command_text(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn find_command(candidates: &[&str]) -> Option<String> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        for candidate in candidates {
            let full = dir.join(candidate);
            if full.exists() {
                return Some(full.to_string_lossy().to_string());
            }
        }
    }
    None
}

#[allow(dead_code)]
fn _timestamp_for_json() -> f64 {
    unix_now_secs()
}
