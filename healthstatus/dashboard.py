"""TUI dashboard rendering using rich."""

from __future__ import annotations

import os
import re
import select
import sys
import termios
import time
import tty
from datetime import datetime, timezone
from typing import Any

from rich.align import Align
from rich.console import Console, Group
from rich.layout import Layout
from rich.live import Live
from rich.panel import Panel
from rich.progress_bar import ProgressBar
from rich.table import Table
from rich.text import Text

from healthstatus import collectors


def _fmt_bytes(n: float) -> str:
    for unit in ("B", "K", "M", "G", "T", "P"):
        if abs(n) < 1024.0:
            return f"{n:3.1f}{unit}"
        n /= 1024.0
    return f"{n:.1f}E"


def _fmt_bps(bps: float) -> str:
    return _fmt_bytes(bps) + "/s"


def _fmt_duration(seconds: float) -> str:
    seconds = int(seconds)
    days, seconds = divmod(seconds, 86400)
    hours, seconds = divmod(seconds, 3600)
    minutes, seconds = divmod(seconds, 60)
    if days:
        return f"{days}d {hours}h {minutes}m"
    if hours:
        return f"{hours}h {minutes}m"
    return f"{minutes}m {seconds}s"


def _bar_color(pct: float) -> str:
    if pct >= 90:
        return "red"
    if pct >= 75:
        return "yellow"
    return "green"


def _usage_bar(pct: float, width: int = 20) -> ProgressBar:
    return ProgressBar(
        total=100,
        completed=max(0.0, min(pct, 100.0)),
        width=width,
        complete_style=_bar_color(pct),
        finished_style="red",
    )


# ─────────────────────────── Page 1: status ───────────────────────────


def render_cpu(data: dict[str, Any]) -> Panel:
    c = data["cpu"]
    table = Table.grid(expand=True, padding=(0, 1))
    table.add_column(justify="right", width=8)
    table.add_column(ratio=1)
    table.add_column(justify="right", width=6)

    for i, pct in enumerate(c["percent_per_core"]):
        table.add_row(f"core {i}", _usage_bar(pct), f"{pct:5.1f}%")

    load = c["load_avg"]
    freq_txt = f"{c['freq_mhz']:.0f} MHz" if c["freq_mhz"] else "—"
    meta = Text.from_markup(
        f"[bold]{c['percent_total']:.1f}%[/bold] total · "
        f"{c['physical_cores']}c/{c['logical_cores']}t · "
        f"{freq_txt} · load {load[0]:.2f} {load[1]:.2f} {load[2]:.2f}",
        style="dim",
    )

    return Panel(Group(table, Text(""), meta), title="[bold cyan]CPU[/]", border_style="cyan")


def render_memory(data: dict[str, Any]) -> Panel:
    m = data["memory"]
    table = Table.grid(expand=True, padding=(0, 1))
    table.add_column(width=5)
    table.add_column(ratio=1)
    table.add_column(justify="right", width=18)

    table.add_row("RAM", _usage_bar(m["ram_percent"]),
                  f"{_fmt_bytes(m['ram_used'])} / {_fmt_bytes(m['ram_total'])}")
    if m["swap_total"] > 0:
        table.add_row("Swap", _usage_bar(m["swap_percent"]),
                      f"{_fmt_bytes(m['swap_used'])} / {_fmt_bytes(m['swap_total'])}")
    else:
        table.add_row("Swap", Text("(none)", style="dim"), "")

    meta = Text(f"{m['ram_percent']:.1f}% used · {_fmt_bytes(m['ram_available'])} available", style="dim")
    return Panel(Group(table, Text(""), meta), title="[bold magenta]Memory[/]", border_style="magenta")


def render_battery(data: dict[str, Any]) -> Panel:
    b = data["battery"]
    if b is None:
        body: Any = Align.center(Text("no battery detected", style="dim"), vertical="middle")
    else:
        pct = b.get("percent")
        charging = b.get("charging")
        lines = []
        if pct is not None:
            bar = _usage_bar(pct, width=18)
            icon = "⚡" if charging else "🔋"
            state = "charging" if charging else "on battery"
            lines.append(Text(f"{icon} {pct:.0f}% · {state}", style="bold"))
            lines.append(bar)
        else:
            lines.append(Text("battery present, charge unknown", style="dim"))
        if b.get("secs_left"):
            lines.append(Text(f"~{_fmt_duration(b['secs_left'])} remaining", style="dim"))
        if b.get("source") == "windows":
            lines.append(Text("source: Windows (Win32_Battery)", style="dim"))
        body = Group(*lines)
    return Panel(body, title="[bold yellow]Battery[/]", border_style="yellow")


def render_disks(data: dict[str, Any]) -> Panel:
    table = Table.grid(expand=True, padding=(0, 1))
    table.add_column(width=18, overflow="fold")
    table.add_column(ratio=1)
    table.add_column(justify="right", width=22)

    for d in data["disks"]:
        mount = d["mount"]
        if len(mount) > 18:
            mount = "…" + mount[-17:]
        table.add_row(
            mount,
            _usage_bar(d["percent"]),
            f"{_fmt_bytes(d['used'])} / {_fmt_bytes(d['total'])} ({d['percent']:.0f}%)",
        )
    if not data["disks"]:
        return Panel(Text("no disks reported", style="dim"), title="[bold blue]Disks[/]", border_style="blue")
    return Panel(table, title="[bold blue]Disks[/]", border_style="blue")


def render_network(data: dict[str, Any]) -> Panel:
    table = Table(expand=True, show_header=True, header_style="bold", padding=(0, 1), box=None)
    table.add_column("iface", style="bold", overflow="fold")
    table.add_column("ipv4", overflow="fold")
    table.add_column("↑", justify="right")
    table.add_column("↓", justify="right")

    for iface in data["network"]["interfaces"]:
        table.add_row(
            iface["name"],
            iface["ipv4"] or "—",
            _fmt_bps(iface["up_bps"]),
            _fmt_bps(iface["down_bps"]),
        )
    if not data["network"]["interfaces"]:
        return Panel(Text("no active interfaces", style="dim"), title="[bold green]Network[/]", border_style="green")
    return Panel(table, title="[bold green]Network[/]", border_style="green")


def render_processes(data: dict[str, Any]) -> Panel:
    table = Table(expand=True, show_header=True, header_style="bold", padding=(0, 1), box=None)
    table.add_column("pid", justify="right", width=7)
    table.add_column("name", overflow="ellipsis", no_wrap=True)
    table.add_column("cpu%", justify="right", width=7)
    table.add_column("mem%", justify="right", width=7)

    for p in data["processes"]:
        table.add_row(str(p["pid"]), p["name"], f"{p['cpu']:.1f}", f"{p['mem']:.1f}")
    return Panel(table, title="[bold red]Top processes[/]", border_style="red")


def render_status_footer(data: dict[str, Any]) -> Panel:
    h = data["host"]
    parts = [
        f"Uptime: [bold]{_fmt_duration(h['uptime_s'])}[/]",
        f"OS: {h['distro'] or h['os']}",
        f"Arch: {h['arch']}",
    ]
    if h["is_wsl"]:
        parts.append("[bold blue]WSL2[/]")
    temps = data.get("temperatures") or []
    if temps:
        parts.append("Temps: " + ", ".join(f"{t['label']} {t['current']:.0f}°C" for t in temps[:3]))
    hint = Text.from_markup(r"[dim]\[1] Status  \[2] Details  \[3] Sensors  \[tab] next  \[q] quit[/]")
    body = Group(Text.from_markup(" · ".join(parts)), hint)
    return Panel(body, border_style="white")


def render_header(data: dict[str, Any], page: int, total_pages: int) -> Panel:
    h = data["host"]
    now = datetime.now().strftime("%H:%M:%S")
    page_name = {1: "Status", 2: "System details", 3: "Sensors"}.get(page, f"Page {page}")
    left = f"[bold]healthstatus[/] · {h['user']}@{h['hostname']}"
    middle = f"[bold cyan]{page_name}[/] ({page}/{total_pages})"
    text = Text.from_markup(f"{left}    {middle}    ") + Text(now, style="dim")
    return Panel(Align.center(text), border_style="white")


def build_status_layout() -> Layout:
    layout = Layout()
    layout.split(
        Layout(name="header", size=3),
        Layout(name="body", ratio=1),
        Layout(name="footer", size=4),
    )
    layout["body"].split_column(
        Layout(name="row1", ratio=1),
        Layout(name="row2", ratio=1),
        Layout(name="row3", ratio=1),
    )
    layout["row1"].split_row(
        Layout(name="cpu", ratio=2),
        Layout(name="memory", ratio=2),
        Layout(name="battery", ratio=1),
    )
    layout["row2"].split_row(Layout(name="disks"))
    layout["row3"].split_row(
        Layout(name="network", ratio=1),
        Layout(name="processes", ratio=1),
    )
    return layout


def populate_status(layout: Layout, data: dict[str, Any]) -> None:
    layout["header"].update(render_header(data, page=1, total_pages=3))
    layout["cpu"].update(render_cpu(data))
    layout["memory"].update(render_memory(data))
    layout["battery"].update(render_battery(data))
    layout["disks"].update(render_disks(data))
    layout["network"].update(render_network(data))
    layout["processes"].update(render_processes(data))
    layout["footer"].update(render_status_footer(data))


# ─────────────────────────── Page 2: system details ───────────────────────────


_WMI_DATE_RE = re.compile(r"/Date\((-?\d+)\)/")


def _parse_wmi_date(value: Any) -> str | None:
    """Convert '/Date(1587340800000)/' → 'YYYY-MM-DD'."""
    if not isinstance(value, str):
        return None
    m = _WMI_DATE_RE.search(value)
    if not m:
        return None
    ms = int(m.group(1))
    try:
        return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).strftime("%Y-%m-%d")
    except (OverflowError, OSError, ValueError):
        return None


def _kv_table(rows: list[tuple[str, Any]]) -> Table:
    table = Table.grid(expand=True, padding=(0, 1))
    table.add_column(style="dim", width=14, no_wrap=True)
    table.add_column(ratio=1, overflow="fold")
    for label, value in rows:
        if value is None or value == "":
            value = "—"
        table.add_row(label, str(value))
    return table


def render_details_host(details: dict[str, Any]) -> Panel:
    win = details.get("windows") or {}
    comp = win.get("computer") or {}
    wsl = details.get("wsl", {})
    rows = [
        ("Manufacturer", comp.get("Manufacturer")),
        ("Model", comp.get("Model")),
        ("System type", comp.get("SystemType")),
        ("Hostname", comp.get("DNSHostName") or wsl.get("distro")),
        ("Domain", comp.get("Domain")),
    ]
    return Panel(_kv_table(rows), title="[bold cyan]Host[/]", border_style="cyan")


def render_details_cpu(details: dict[str, Any]) -> Panel:
    win = details.get("windows") or {}
    cpu_win = win.get("cpu") or {}
    wsl = details.get("wsl", {})
    name = (cpu_win.get("Name") or wsl.get("cpu_model_as_seen") or "—").strip()
    rows = [
        ("Model", name),
        ("Vendor", cpu_win.get("Manufacturer")),
        ("Socket", cpu_win.get("SocketDesignation")),
        ("Cores", f"{cpu_win.get('NumberOfCores', '?')}c / {cpu_win.get('NumberOfLogicalProcessors', '?')}t"),
        ("Base clock", f"{cpu_win.get('MaxClockSpeed')} MHz" if cpu_win.get("MaxClockSpeed") else None),
        ("L2 cache", f"{cpu_win.get('L2CacheSize')} KB" if cpu_win.get("L2CacheSize") else None),
        ("L3 cache", f"{cpu_win.get('L3CacheSize')} KB" if cpu_win.get("L3CacheSize") else None),
    ]
    return Panel(_kv_table(rows), title="[bold cyan]CPU[/]", border_style="cyan")


def render_details_gpu(details: dict[str, Any]) -> Panel:
    win = details.get("windows") or {}
    gpus = win.get("gpus") or []
    if not gpus:
        return Panel(Text("no GPU info", style="dim"), title="[bold magenta]GPU[/]", border_style="magenta")
    groups: list[Any] = []
    for i, g in enumerate(gpus):
        vram = g.get("AdapterRAM")
        # Win32 AdapterRAM caps at ~4GB due to DWORD; it's indicative, not authoritative.
        vram_txt = _fmt_bytes(vram) if isinstance(vram, (int, float)) and vram > 0 else "—"
        res = ""
        if g.get("CurrentHorizontalResolution") and g.get("CurrentVerticalResolution"):
            res = f"{g['CurrentHorizontalResolution']}×{g['CurrentVerticalResolution']}"
            if g.get("CurrentRefreshRate"):
                res += f" @ {g['CurrentRefreshRate']}Hz"
        rows = [
            ("Name", g.get("Name")),
            ("Driver", g.get("DriverVersion")),
            ("Driver date", _parse_wmi_date(g.get("DriverDate"))),
            ("VRAM (DWORD)", vram_txt),
            ("Display", res or None),
        ]
        if i > 0:
            groups.append(Text(""))
        groups.append(_kv_table(rows))
    return Panel(Group(*groups), title="[bold magenta]GPU[/]", border_style="magenta")


def render_details_memory(details: dict[str, Any]) -> Panel:
    win = details.get("windows") or {}
    comp = win.get("computer") or {}
    modules = win.get("memory") or []
    wsl = details.get("wsl", {})

    host_total = comp.get("TotalPhysicalMemory")
    vm_total = wsl.get("vm_ram_total")

    lines: list[Any] = []
    summary = _kv_table([
        ("Host RAM", _fmt_bytes(host_total) if host_total else None),
        ("WSL VM RAM", _fmt_bytes(vm_total) if vm_total else None),
        ("DIMM count", len(modules) if modules else None),
    ])
    lines.append(summary)

    if modules:
        lines.append(Text(""))
        t = Table(expand=True, show_header=True, header_style="bold", padding=(0, 1), box=None)
        t.add_column("slot", overflow="fold")
        t.add_column("size", justify="right")
        t.add_column("speed", justify="right")
        t.add_column("mfr", overflow="fold")
        for m in modules:
            cap = m.get("Capacity")
            try:
                cap_txt = _fmt_bytes(int(cap)) if cap else "—"
            except (TypeError, ValueError):
                cap_txt = "—"
            speed = m.get("ConfiguredClockSpeed")
            speed_txt = f"{speed} MHz" if speed else "—"
            t.add_row(m.get("BankLabel") or "—", cap_txt, speed_txt, m.get("Manufacturer") or "—")
        lines.append(t)

    return Panel(Group(*lines), title="[bold yellow]Memory[/]", border_style="yellow")


def render_details_os(details: dict[str, Any]) -> Panel:
    win = details.get("windows") or {}
    os_ = win.get("os") or {}
    wsl = details.get("wsl", {})
    rows = [
        ("Windows", os_.get("Caption")),
        ("Version", f"{os_.get('Version')} (build {os_.get('BuildNumber')})" if os_.get("Version") else None),
        ("Arch", os_.get("OSArchitecture")),
        ("Installed", _parse_wmi_date(os_.get("InstallDate"))),
        ("Last boot", _parse_wmi_date(os_.get("LastBootUpTime"))),
        ("WSL distro", wsl.get("distro")),
        ("WSL kernel", wsl.get("kernel")),
        ("Python", wsl.get("python")),
    ]
    return Panel(_kv_table(rows), title="[bold green]OS[/]", border_style="green")


def render_details_board(details: dict[str, Any]) -> Panel:
    win = details.get("windows") or {}
    board = win.get("board") or {}
    bios = win.get("bios") or {}
    rows = [
        ("Board mfr", board.get("Manufacturer")),
        ("Board model", board.get("Product")),
        ("Board rev", board.get("Version")),
        ("BIOS mfr", bios.get("Manufacturer")),
        ("BIOS ver", bios.get("SMBIOSBIOSVersion")),
        ("BIOS date", _parse_wmi_date(bios.get("ReleaseDate"))),
    ]
    return Panel(_kv_table(rows), title="[bold blue]Motherboard / BIOS[/]", border_style="blue")


def render_details_wifi(details: dict[str, Any]) -> Panel:
    w = details.get("wifi")
    if not w:
        return Panel(Text("no Wi-Fi interface", style="dim"), title="[bold red]Wi-Fi[/]", border_style="red")
    rows = [
        ("SSID", w.get("ssid")),
        ("State", w.get("state")),
        ("Signal", w.get("signal")),
        ("Radio", w.get("radio")),
        ("Rx / Tx", f"{w.get('rx_mbps') or '—'} / {w.get('tx_mbps') or '—'} Mbps"),
    ]
    return Panel(_kv_table(rows), title="[bold red]Wi-Fi[/]", border_style="red")


def render_details_footer(details: dict[str, Any]) -> Panel:
    parts = ["Windows host info (WMI via powershell.exe) · Wi-Fi via netsh"]
    hint = Text.from_markup(r"[dim]\[1] Status  \[2] Details  \[3] Sensors  \[tab] next  \[q] quit[/]")
    return Panel(Group(Text(" · ".join(parts), style="dim"), hint), border_style="white")


def build_details_layout() -> Layout:
    layout = Layout()
    layout.split(
        Layout(name="header", size=3),
        Layout(name="body", ratio=1),
        Layout(name="footer", size=4),
    )
    layout["body"].split_column(
        Layout(name="row1", ratio=1),
        Layout(name="row2", ratio=1),
        Layout(name="row3", ratio=1),
    )
    layout["row1"].split_row(
        Layout(name="host", ratio=1),
        Layout(name="cpu", ratio=1),
        Layout(name="gpu", ratio=1),
    )
    layout["row2"].split_row(
        Layout(name="memory", ratio=1),
        Layout(name="board", ratio=1),
    )
    layout["row3"].split_row(
        Layout(name="os", ratio=2),
        Layout(name="wifi", ratio=1),
    )
    return layout


# ─────────────────────────── Page 3: sensors ───────────────────────────


def _fmt_num(value: Any, unit: str = "", digits: int = 0) -> str:
    if value is None:
        return "—"
    try:
        if digits == 0:
            return f"{int(round(float(value)))}{unit}"
        return f"{float(value):.{digits}f}{unit}"
    except (TypeError, ValueError):
        return "—"


def _temp_color(c: float | None) -> str:
    if c is None:
        return "white"
    if c >= 85:
        return "red"
    if c >= 75:
        return "yellow"
    return "green"


def render_sensors_gpu(gpu: dict[str, Any]) -> Panel:
    name = gpu.get("name") or "GPU"
    temp = gpu.get("temp_c")
    util = gpu.get("gpu_util_pct") or 0.0
    mem_util = gpu.get("mem_util_pct") or 0.0
    vram_used = gpu.get("vram_used_mib") or 0.0
    vram_total = gpu.get("vram_total_mib") or 0.0
    vram_pct = (vram_used / vram_total * 100.0) if vram_total else 0.0
    fan = gpu.get("fan_pct")
    power = gpu.get("power_w")
    power_limit = gpu.get("power_limit_w")
    power_pct = (power / power_limit * 100.0) if (power is not None and power_limit) else 0.0

    tbl = Table.grid(expand=True, padding=(0, 1))
    tbl.add_column(style="bold", no_wrap=True)
    tbl.add_column(ratio=1)
    tbl.add_column(justify="right", no_wrap=True)

    tbl.add_row(
        "Temp",
        _usage_bar(min(100.0, (temp or 0) * 100.0 / 100.0) if temp else 0.0),
        Text(_fmt_num(temp, "°C"), style=_temp_color(temp)),
    )
    tbl.add_row("GPU util", _usage_bar(util), f"{util:.0f}%")
    tbl.add_row(
        "VRAM",
        _usage_bar(vram_pct),
        f"{vram_used/1024:.1f} / {vram_total/1024:.1f} GiB",
    )
    tbl.add_row("Mem I/O", _usage_bar(mem_util), f"{mem_util:.0f}%")
    if power_limit:
        tbl.add_row("Power", _usage_bar(power_pct), f"{_fmt_num(power, ' W', 1)} / {_fmt_num(power_limit, ' W', 0)}")
    else:
        tbl.add_row("Power", Text("—", style="dim"), _fmt_num(power, " W", 1))
    tbl.add_row("Fan", _usage_bar(fan or 0.0), _fmt_num(fan, "%"))
    clocks = Text(
        f"Core {_fmt_num(gpu.get('clock_core_mhz'), ' MHz')}   Mem {_fmt_num(gpu.get('clock_mem_mhz'), ' MHz')}",
        style="dim",
    )
    tbl.add_row("Clocks", clocks, "")

    return Panel(tbl, title=f"[bold green]{name}[/]", border_style="green")


def render_sensors_processes(gpu_data: dict[str, Any]) -> Panel:
    procs = gpu_data.get("processes") or []
    tbl = Table(expand=True, show_header=True, header_style="bold")
    tbl.add_column("PID", justify="right", width=7)
    tbl.add_column("Process", ratio=1, overflow="ellipsis", no_wrap=True)
    tbl.add_column("VRAM", justify="right", width=10)
    if not procs:
        tbl.add_row("—", Text("no compute processes visible", style="dim"), "—")
    else:
        for p in procs[:10]:
            mem = p.get("mem_mib")
            tbl.add_row(
                str(p.get("pid") or "?"),
                str(p.get("name") or "?"),
                f"{mem:.0f} MiB" if mem is not None else "—",
            )
    return Panel(tbl, title="[bold cyan]GPU processes[/]", border_style="cyan")


def render_sensors_cpu(status_data: dict[str, Any], gpu_data: dict[str, Any]) -> Panel:
    temps = status_data.get("temperatures") or []
    if temps:
        tbl = Table(expand=True, show_header=True, header_style="bold")
        tbl.add_column("Sensor", ratio=1, overflow="ellipsis", no_wrap=True)
        tbl.add_column("Now", justify="right", width=8)
        tbl.add_column("High", justify="right", width=8)
        for t in temps:
            cur = t.get("current")
            high = t.get("high")
            tbl.add_row(
                t.get("label") or t.get("chip") or "?",
                Text(_fmt_num(cur, "°C", 1), style=_temp_color(cur)),
                _fmt_num(high, "°C", 0) if high else "—",
            )
        body: Any = tbl
    else:
        msg_lines = [
            Text("No CPU/board sensors available from WSL.", style="dim"),
            Text(""),
            Text("On Windows, ACPI thermal zones and Win32_Fan require admin.", style="dim"),
            Text("Install LibreHardwareMonitor and run elevated to expose full sensors.", style="dim"),
        ]
        err = gpu_data.get("error")
        if err:
            msg_lines.append(Text(""))
            msg_lines.append(Text(f"nvidia-smi: {err}", style="yellow"))
        body = Group(*msg_lines)
    return Panel(body, title="[bold yellow]CPU / board sensors[/]", border_style="yellow")


def render_sensors_footer(gpu_data: dict[str, Any]) -> Panel:
    if gpu_data.get("available"):
        count = len(gpu_data.get("gpus") or [])
        status = f"{count} NVIDIA GPU{'s' if count != 1 else ''} via nvidia-smi"
    elif gpu_data.get("error"):
        status = f"GPU unavailable: {gpu_data['error']}"
    else:
        status = "GPU telemetry unavailable"
    hint = Text.from_markup(r"[dim]\[1] Status  \[2] Details  \[3] Sensors  \[tab] next  \[q] quit[/]")
    return Panel(Group(Text(status, style="dim"), hint), border_style="white")


def build_sensors_layout() -> Layout:
    layout = Layout()
    layout.split(
        Layout(name="header", size=3),
        Layout(name="body", ratio=1),
        Layout(name="footer", size=4),
    )
    layout["body"].split_column(
        Layout(name="row1", ratio=2),
        Layout(name="row2", ratio=1),
    )
    layout["row1"].split_row(
        Layout(name="gpu", ratio=2),
        Layout(name="cpu", ratio=1),
    )
    layout["row2"].split_row(Layout(name="processes"))
    return layout


def populate_sensors(
    layout: Layout,
    gpu_data: dict[str, Any],
    status_data: dict[str, Any],
) -> None:
    layout["header"].update(render_header(status_data, page=3, total_pages=3))
    gpus = gpu_data.get("gpus") or []
    if gpus:
        layout["gpu"].update(render_sensors_gpu(gpus[0]))
    else:
        msg = Text("GPU telemetry unavailable", style="yellow")
        if gpu_data.get("error"):
            msg = Group(msg, Text(gpu_data["error"], style="dim"))
        layout["gpu"].update(Panel(msg, title="[bold green]GPU[/]", border_style="green"))
    layout["cpu"].update(render_sensors_cpu(status_data, gpu_data))
    layout["processes"].update(render_sensors_processes(gpu_data))
    layout["footer"].update(render_sensors_footer(gpu_data))


def populate_details(layout: Layout, details: dict[str, Any], status_data: dict[str, Any]) -> None:
    layout["header"].update(render_header(status_data, page=2, total_pages=3))
    layout["host"].update(render_details_host(details))
    layout["cpu"].update(render_details_cpu(details))
    layout["gpu"].update(render_details_gpu(details))
    layout["memory"].update(render_details_memory(details))
    layout["board"].update(render_details_board(details))
    layout["os"].update(render_details_os(details))
    layout["wifi"].update(render_details_wifi(details))
    layout["footer"].update(render_details_footer(details))


# ─────────────────────────── Input + live loop ───────────────────────────


class _RawStdin:
    """Non-blocking single-char stdin reader for the live loop."""

    def __init__(self) -> None:
        self.fd = sys.stdin.fileno() if sys.stdin and sys.stdin.isatty() else -1
        self._old: Any = None

    def __enter__(self) -> "_RawStdin":
        if self.fd >= 0:
            try:
                self._old = termios.tcgetattr(self.fd)
                tty.setcbreak(self.fd)
            except termios.error:
                self._old = None
        return self

    def __exit__(self, *exc: Any) -> None:
        if self.fd >= 0 and self._old is not None:
            termios.tcsetattr(self.fd, termios.TCSADRAIN, self._old)

    def read_until(self, deadline: float) -> str | None:
        """Poll for a single char until `deadline` (monotonic). Returns char or None."""
        if self.fd < 0:
            # No tty — just sleep.
            remaining = deadline - time.monotonic()
            if remaining > 0:
                time.sleep(remaining)
            return None
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return None
            r, _, _ = select.select([self.fd], [], [], remaining)
            if not r:
                return None
            try:
                ch = os.read(self.fd, 1).decode("utf-8", "ignore")
            except OSError:
                return None
            if ch:
                return ch


def run_live(interval: float = 1.0, console: Console | None = None) -> None:
    console = console or Console()
    status_layout = build_status_layout()
    details_layout = build_details_layout()
    sensors_layout = build_sensors_layout()
    prev_net: dict[str, Any] | None = None
    cached_details: dict[str, Any] | None = None
    details_fetched_at = 0.0
    DETAILS_TTL = 15.0  # seconds

    import psutil
    psutil.cpu_percent(interval=None, percpu=True)

    # Initial sample so first frame has non-zero CPU values.
    status_data = collectors.collect_all(prev_net, interval)
    prev_net = status_data["network"].pop("_snapshot")
    populate_status(status_layout, status_data)

    page = 1
    active_layout: Layout = status_layout

    with Live(active_layout, console=console, screen=True, refresh_per_second=max(1.0, 1.0 / interval)) as live, _RawStdin() as keys:
        try:
            while True:
                # Refresh data for the active page.
                status_data = collectors.collect_all(prev_net, interval)
                prev_net = status_data["network"].pop("_snapshot")
                populate_status(status_layout, status_data)

                if page == 2:
                    now = time.monotonic()
                    if cached_details is None or now - details_fetched_at > DETAILS_TTL:
                        cached_details = collectors.system_details()
                        details_fetched_at = now
                    populate_details(details_layout, cached_details, status_data)
                    live.update(details_layout, refresh=True)
                elif page == 3:
                    gpu_data = collectors.gpu_telemetry()
                    populate_sensors(sensors_layout, gpu_data, status_data)
                    live.update(sensors_layout, refresh=True)
                else:
                    live.update(status_layout, refresh=True)

                # Poll keys for `interval` seconds.
                deadline = time.monotonic() + interval
                should_refresh_now = False
                while time.monotonic() < deadline and not should_refresh_now:
                    ch = keys.read_until(deadline)
                    if ch is None:
                        break
                    if ch in ("q", "Q", "\x03", "\x04"):
                        return
                    if ch == "1" and page != 1:
                        page = 1
                        should_refresh_now = True
                    elif ch == "2" and page != 2:
                        page = 2
                        should_refresh_now = True
                    elif ch == "3" and page != 3:
                        page = 3
                        should_refresh_now = True
                    elif ch == "\t":
                        page = page % 3 + 1
                        should_refresh_now = True
        except KeyboardInterrupt:
            pass


def render_once(console: Console | None = None) -> None:
    console = console or Console()
    import psutil
    psutil.cpu_percent(interval=None, percpu=True)
    time.sleep(0.2)
    data = collectors.collect_all(None, 1.0)
    data["network"].pop("_snapshot", None)
    layout = build_status_layout()
    populate_status(layout, data)
    console.print(layout)


def render_details_once(console: Console | None = None) -> None:
    console = console or Console()
    import psutil
    psutil.cpu_percent(interval=None, percpu=True)
    time.sleep(0.2)
    status_data = collectors.collect_all(None, 1.0)
    status_data["network"].pop("_snapshot", None)
    details = collectors.system_details()
    layout = build_details_layout()
    populate_details(layout, details, status_data)
    console.print(layout)


def render_sensors_once(console: Console | None = None) -> None:
    console = console or Console()
    import psutil
    psutil.cpu_percent(interval=None, percpu=True)
    time.sleep(0.2)
    status_data = collectors.collect_all(None, 1.0)
    status_data["network"].pop("_snapshot", None)
    gpu_data = collectors.gpu_telemetry()
    layout = build_sensors_layout()
    populate_sensors(layout, gpu_data, status_data)
    console.print(layout)
