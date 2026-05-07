"""Pure data-gathering functions. Each returns a plain dict/list — no rendering."""

from __future__ import annotations

import json
import os
import platform
import shutil
import subprocess
import time
from typing import Any

import psutil


def is_wsl() -> bool:
    try:
        with open("/proc/version", "r") as f:
            return "microsoft" in f.read().lower()
    except OSError:
        return False


_WSL = is_wsl()


def cpu() -> dict[str, Any]:
    freq = psutil.cpu_freq()
    try:
        load = os.getloadavg()
    except (AttributeError, OSError):
        load = (0.0, 0.0, 0.0)
    return {
        "percent_total": psutil.cpu_percent(interval=None),
        "percent_per_core": psutil.cpu_percent(interval=None, percpu=True),
        "logical_cores": psutil.cpu_count(logical=True),
        "physical_cores": psutil.cpu_count(logical=False),
        "freq_mhz": freq.current if freq else None,
        "freq_max_mhz": freq.max if freq else None,
        "load_avg": load,
    }


def memory() -> dict[str, Any]:
    vm = psutil.virtual_memory()
    sm = psutil.swap_memory()
    return {
        "ram_total": vm.total,
        "ram_used": vm.used,
        "ram_available": vm.available,
        "ram_percent": vm.percent,
        "swap_total": sm.total,
        "swap_used": sm.used,
        "swap_percent": sm.percent,
    }


_REAL_FS = {
    "ext2", "ext3", "ext4", "xfs", "btrfs", "zfs", "f2fs", "reiserfs", "jfs",
    "ntfs", "ntfs3", "vfat", "fat32", "exfat", "hfs", "hfsplus", "apfs",
    "9p", "drvfs", "fuseblk", "cifs", "smbfs", "nfs", "nfs4",
}

_SKIP_MOUNT_PREFIXES = ("/init", "/mnt/wslg", "/usr/lib/wsl", "/usr/lib/modules")


def disks() -> list[dict[str, Any]]:
    out = []
    seen_devices: set[str] = set()
    for part in psutil.disk_partitions(all=True):
        fstype = part.fstype.lower()
        if fstype not in _REAL_FS:
            continue
        if any(part.mountpoint.startswith(p) for p in _SKIP_MOUNT_PREFIXES):
            continue
        # Dedupe same physical device mounted multiple times (e.g. /dev/sdd at / and /mnt/wslg/distro)
        if part.device and part.device in seen_devices:
            continue
        try:
            usage = psutil.disk_usage(part.mountpoint)
        except (PermissionError, OSError):
            continue
        if part.device:
            seen_devices.add(part.device)
        out.append({
            "mount": part.mountpoint,
            "device": part.device,
            "fstype": part.fstype,
            "total": usage.total,
            "used": usage.used,
            "free": usage.free,
            "percent": usage.percent,
        })
    return out


def network(prev: dict[str, Any] | None, interval: float) -> dict[str, Any]:
    """Returns per-interface state plus throughput vs. previous snapshot."""
    addrs = psutil.net_if_addrs()
    stats = psutil.net_if_stats()
    counters = psutil.net_io_counters(pernic=True)
    now = time.monotonic()

    interfaces = []
    for name, addr_list in addrs.items():
        if name not in stats or not stats[name].isup:
            continue
        ipv4 = next((a.address for a in addr_list if a.family.name == "AF_INET"), None)
        c = counters.get(name)
        if not c:
            continue
        up_bps = down_bps = 0.0
        if prev and name in prev["counters"]:
            dt = max(now - prev["time"], 1e-6)
            pc = prev["counters"][name]
            up_bps = (c.bytes_sent - pc.bytes_sent) / dt
            down_bps = (c.bytes_recv - pc.bytes_recv) / dt
        interfaces.append({
            "name": name,
            "ipv4": ipv4,
            "up_bps": max(up_bps, 0.0),
            "down_bps": max(down_bps, 0.0),
            "total_sent": c.bytes_sent,
            "total_recv": c.bytes_recv,
        })

    return {
        "interfaces": interfaces,
        "_snapshot": {"time": now, "counters": counters},
    }


def _battery_from_windows() -> dict[str, Any] | None:
    ps = shutil.which("powershell.exe")
    if not ps:
        return None
    try:
        result = subprocess.run(
            [
                ps,
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_Battery | Select-Object EstimatedChargeRemaining,BatteryStatus | ConvertTo-Json -Compress",
            ],
            capture_output=True,
            text=True,
            timeout=3,
        )
    except (subprocess.TimeoutExpired, OSError):
        return None
    if result.returncode != 0 or not result.stdout.strip():
        return None
    try:
        data = json.loads(result.stdout)
    except json.JSONDecodeError:
        return None
    if isinstance(data, list):
        data = data[0] if data else None
    if not data:
        return None
    percent = data.get("EstimatedChargeRemaining")
    # Win32_Battery BatteryStatus: 2 = plugged in, 1 = on battery, higher = charging/full
    status = data.get("BatteryStatus")
    charging = status in (2, 6, 7, 8, 9)
    return {
        "percent": float(percent) if percent is not None else None,
        "charging": charging,
        "source": "windows",
    }


def battery() -> dict[str, Any] | None:
    try:
        b = psutil.sensors_battery()
    except (AttributeError, NotImplementedError):
        b = None
    if b is not None:
        return {
            "percent": b.percent,
            "charging": b.power_plugged,
            "secs_left": None if b.secsleft in (psutil.POWER_TIME_UNLIMITED, psutil.POWER_TIME_UNKNOWN) else b.secsleft,
            "source": "psutil",
        }
    if _WSL:
        return _battery_from_windows()
    return None


def processes(top_n: int = 8) -> list[dict[str, Any]]:
    procs = []
    for p in psutil.process_iter(["pid", "name", "cpu_percent", "memory_percent"]):
        try:
            procs.append({
                "pid": p.info["pid"],
                "name": p.info["name"] or "?",
                "cpu": p.info["cpu_percent"] or 0.0,
                "mem": p.info["memory_percent"] or 0.0,
            })
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue
    procs.sort(key=lambda x: (x["cpu"], x["mem"]), reverse=True)
    return procs[:top_n]


def temperatures() -> list[dict[str, Any]]:
    try:
        data = psutil.sensors_temperatures()
    except AttributeError:
        return []
    out = []
    for chip, entries in data.items():
        for e in entries:
            out.append({
                "chip": chip,
                "label": e.label or chip,
                "current": e.current,
                "high": e.high,
            })
    return out


def host() -> dict[str, Any]:
    boot = psutil.boot_time()
    uptime_s = time.time() - boot
    try:
        user = os.getlogin()
    except OSError:
        user = os.environ.get("USER") or os.environ.get("USERNAME") or "?"
    return {
        "user": user,
        "hostname": platform.node(),
        "os": f"{platform.system()} {platform.release()}",
        "distro": _linux_pretty_name(),
        "arch": platform.machine(),
        "python": platform.python_version(),
        "uptime_s": uptime_s,
        "is_wsl": _WSL,
    }


def _linux_pretty_name() -> str | None:
    try:
        with open("/etc/os-release") as f:
            for line in f:
                if line.startswith("PRETTY_NAME="):
                    return line.split("=", 1)[1].strip().strip('"')
    except OSError:
        pass
    return None


def collect_all(prev_net: dict[str, Any] | None, interval: float) -> dict[str, Any]:
    net = network(prev_net, interval)
    return {
        "host": host(),
        "cpu": cpu(),
        "memory": memory(),
        "battery": battery(),
        "disks": disks(),
        "network": net,
        "processes": processes(),
        "temperatures": temperatures(),
    }


_PS_DETAILS_SCRIPT = r"""
$ErrorActionPreference = 'SilentlyContinue'
$data = [ordered]@{
  cpu      = Get-CimInstance Win32_Processor      | Select-Object Name,Manufacturer,NumberOfCores,NumberOfLogicalProcessors,MaxClockSpeed,L2CacheSize,L3CacheSize,SocketDesignation
  computer = Get-CimInstance Win32_ComputerSystem | Select-Object Manufacturer,Model,SystemType,TotalPhysicalMemory,NumberOfProcessors,DNSHostName,Domain
  board    = Get-CimInstance Win32_BaseBoard      | Select-Object Manufacturer,Product,Version,SerialNumber
  bios     = Get-CimInstance Win32_BIOS           | Select-Object Manufacturer,SMBIOSBIOSVersion,ReleaseDate,SerialNumber
  os       = Get-CimInstance Win32_OperatingSystem| Select-Object Caption,Version,BuildNumber,OSArchitecture,InstallDate,LastBootUpTime,RegisteredUser
  gpus     = @(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,DriverDate,AdapterRAM,VideoProcessor,CurrentHorizontalResolution,CurrentVerticalResolution,CurrentRefreshRate)
  memory   = @(Get-CimInstance Win32_PhysicalMemory | Select-Object BankLabel,Capacity,ConfiguredClockSpeed,Manufacturer,PartNumber,FormFactor)
}
$data | ConvertTo-Json -Depth 5 -Compress
"""


def _run_powershell(script: str, timeout: float = 6.0) -> Any:
    ps = shutil.which("powershell.exe")
    if not ps:
        return None
    try:
        result = subprocess.run(
            [ps, "-NoProfile", "-NonInteractive", "-Command", script],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except (subprocess.TimeoutExpired, OSError):
        return None
    if result.returncode != 0 or not result.stdout.strip():
        return None
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        return None


def _wifi_info() -> dict[str, Any] | None:
    """Parse `netsh wlan show interfaces` for the active connection."""
    netsh = shutil.which("netsh.exe")
    if not netsh:
        return None
    try:
        result = subprocess.run(
            [netsh, "wlan", "show", "interfaces"],
            capture_output=True,
            text=True,
            timeout=3,
        )
    except (subprocess.TimeoutExpired, OSError):
        return None
    if result.returncode != 0:
        return None
    info: dict[str, Any] = {}
    for line in result.stdout.splitlines():
        if ":" not in line:
            continue
        key, _, value = line.partition(":")
        key = key.strip().lower()
        value = value.strip()
        if key == "ssid" and "bssid" not in key:
            info.setdefault("ssid", value)
        elif key == "signal":
            info["signal"] = value
        elif key == "radio type":
            info["radio"] = value
        elif key == "receive rate (mbps)":
            info["rx_mbps"] = value
        elif key == "transmit rate (mbps)":
            info["tx_mbps"] = value
        elif key == "state":
            info["state"] = value
    return info or None


def _kernel_version() -> str | None:
    try:
        return platform.release()
    except Exception:
        return None


def _cpu_model_linux() -> str | None:
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return None


_GPU_FIELDS = [
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
]


def _nvidia_smi_path() -> str | None:
    for candidate in ("nvidia-smi", "nvidia-smi.exe"):
        p = shutil.which(candidate)
        if p:
            return p
    return None


def _parse_nv_number(raw: str) -> float | None:
    s = raw.strip()
    if not s or s in ("[N/A]", "[Not Supported]", "N/A"):
        return None
    try:
        return float(s)
    except ValueError:
        return None


def gpu_telemetry() -> dict[str, Any]:
    """Query NVIDIA GPU telemetry via nvidia-smi. Returns {available, gpus, processes, error}."""
    out: dict[str, Any] = {"available": False, "gpus": [], "processes": [], "error": None}
    exe = _nvidia_smi_path()
    if not exe:
        out["error"] = "nvidia-smi not found"
        return out
    try:
        result = subprocess.run(
            [exe, f"--query-gpu={','.join(_GPU_FIELDS)}", "--format=csv,noheader,nounits"],
            capture_output=True,
            text=True,
            timeout=2.0,
        )
    except (subprocess.TimeoutExpired, OSError) as exc:
        out["error"] = f"nvidia-smi timeout/error: {exc}"
        return out
    if result.returncode != 0:
        out["error"] = result.stderr.strip().splitlines()[-1] if result.stderr else f"exit {result.returncode}"
        return out
    for line in result.stdout.strip().splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) != len(_GPU_FIELDS):
            continue
        gpu = {
            "name": parts[0],
            "temp_c": _parse_nv_number(parts[1]),
            "gpu_util_pct": _parse_nv_number(parts[2]),
            "mem_util_pct": _parse_nv_number(parts[3]),
            "vram_used_mib": _parse_nv_number(parts[4]),
            "vram_total_mib": _parse_nv_number(parts[5]),
            "fan_pct": _parse_nv_number(parts[6]),
            "power_w": _parse_nv_number(parts[7]),
            "power_limit_w": _parse_nv_number(parts[8]),
            "clock_core_mhz": _parse_nv_number(parts[9]),
            "clock_mem_mhz": _parse_nv_number(parts[10]),
        }
        out["gpus"].append(gpu)
    try:
        proc_result = subprocess.run(
            [exe, "--query-compute-apps=pid,process_name,used_memory", "--format=csv,noheader,nounits"],
            capture_output=True,
            text=True,
            timeout=2.0,
        )
        if proc_result.returncode == 0:
            for line in proc_result.stdout.strip().splitlines():
                parts = [p.strip() for p in line.split(",")]
                if len(parts) == 3:
                    out["processes"].append({
                        "pid": parts[0],
                        "name": parts[1],
                        "mem_mib": _parse_nv_number(parts[2]),
                    })
    except (subprocess.TimeoutExpired, OSError):
        pass
    out["available"] = bool(out["gpus"])
    return out


def system_details() -> dict[str, Any]:
    """Collect mostly-static host/system info. Slow (shells out to PowerShell) — cache in caller."""
    details: dict[str, Any] = {
        "wsl": {
            "is_wsl": _WSL,
            "kernel": _kernel_version(),
            "distro": _linux_pretty_name(),
            "arch": platform.machine(),
            "python": platform.python_version(),
            "cpu_model_as_seen": _cpu_model_linux(),
            "vm_ram_total": psutil.virtual_memory().total,
            "vm_cpu_count": psutil.cpu_count(logical=True),
        },
        "windows": None,
        "wifi": None,
    }
    if _WSL:
        win = _run_powershell(_PS_DETAILS_SCRIPT)
        if win:
            details["windows"] = win
        details["wifi"] = _wifi_info()
    return details
