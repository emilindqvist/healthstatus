use crate::collectors::{GpuTelemetry, Metrics, SystemDetails};
use crate::{fmt_bytes, fmt_duration, json_escape};

pub fn render_status(data: &Metrics) -> String {
    let mut out = String::new();
    out.push_str(&header(data, "Status", 1));
    out.push_str(&section("CPU", &render_cpu(data)));
    out.push_str(&section("Memory", &render_memory(data)));
    out.push_str(&section("Disks", &render_disks(data)));
    out.push_str(&section("Network", &render_network(data)));
    out.push_str(&section("Top processes", &render_processes(data)));
    out.push_str(&section("Battery", &render_battery(data)));
    out.push_str(&footer(data));
    out
}

pub fn render_details(data: &Metrics, details: &SystemDetails) -> String {
    let mut out = String::new();
    out.push_str(&header(data, "System details", 2));
    let mut rows = vec![
        (
            "Distro".to_string(),
            details.distro.clone().unwrap_or_else(|| "-".to_string()),
        ),
        ("Kernel".to_string(), details.kernel.clone()),
        ("Arch".to_string(), details.arch.clone()),
        (
            "CPU as seen".to_string(),
            details.cpu_model.clone().unwrap_or_else(|| "-".to_string()),
        ),
        (
            "WSL VM RAM".to_string(),
            fmt_bytes(details.vm_ram_total as f64),
        ),
    ];
    rows.extend(details.windows.iter().cloned());
    out.push_str(&section("Host", &kv(&rows)));
    out.push_str(&section(
        "Wi-Fi",
        &kv_or_empty(&details.wifi, "no Wi-Fi interface"),
    ));
    out.push_str("[1] Status  [2] Details  [3] Sensors  [tab] next  [q] quit\n");
    out
}

pub fn render_sensors(data: &Metrics, gpu: &GpuTelemetry) -> String {
    let mut out = String::new();
    out.push_str(&header(data, "Sensors", 3));
    if gpu.available {
        for (idx, g) in gpu.gpus.iter().enumerate() {
            let rows = vec![
                ("Temp".to_string(), opt(g.temp_c, " C", 0)),
                ("GPU util".to_string(), opt(g.gpu_util_pct, "%", 0)),
                ("Mem I/O".to_string(), opt(g.mem_util_pct, "%", 0)),
                (
                    "VRAM".to_string(),
                    match (g.vram_used_mib, g.vram_total_mib) {
                        (Some(used), Some(total)) => {
                            format!("{:.1} / {:.1} GiB", used / 1024.0, total / 1024.0)
                        }
                        _ => "-".to_string(),
                    },
                ),
                ("Fan".to_string(), opt(g.fan_pct, "%", 0)),
                ("Power".to_string(), opt(g.power_w, " W", 1)),
                ("Core clock".to_string(), opt(g.clock_core_mhz, " MHz", 0)),
                ("Mem clock".to_string(), opt(g.clock_mem_mhz, " MHz", 0)),
            ];
            out.push_str(&section(
                &format!("GPU {}: {}", idx + 1, g.name),
                &kv(&rows),
            ));
        }
    } else {
        out.push_str(&section(
            "GPU",
            &format!(
                "GPU telemetry unavailable{}\n",
                gpu.error
                    .as_ref()
                    .map(|e| format!(": {e}"))
                    .unwrap_or_default()
            ),
        ));
    }
    let temp_rows = data
        .temperatures
        .iter()
        .map(|t| {
            (
                t.label.clone(),
                match t.high {
                    Some(high) => format!("{:.1} C (high {:.0} C)", t.current, high),
                    None => format!("{:.1} C", t.current),
                },
            )
        })
        .collect::<Vec<_>>();
    out.push_str(&section(
        "CPU / board sensors",
        &kv_or_empty(&temp_rows, "no sensors available"),
    ));
    let proc_rows = gpu
        .processes
        .iter()
        .map(|p| {
            (
                p.pid.clone(),
                format!("{} ({})", p.name, opt(p.mem_mib, " MiB", 0)),
            )
        })
        .collect::<Vec<_>>();
    out.push_str(&section(
        "GPU processes",
        &kv_or_empty(&proc_rows, "no compute processes visible"),
    ));
    out.push_str("[1] Status  [2] Details  [3] Sensors  [tab] next  [q] quit\n");
    out
}

pub fn render_status_live(data: &Metrics, width: usize, height: usize) -> String {
    let mut lines = base_frame(data, "Status", 1, width);

    push_title(&mut lines, "CPU");
    lines.push(clip_line(
        &format!(
            "total {:>5.1}%  {}c/{}t  load {:.2} {:.2} {:.2}",
            data.cpu.percent_total,
            data.cpu.physical_cores.unwrap_or(data.cpu.logical_cores),
            data.cpu.logical_cores,
            data.cpu.load_avg.0,
            data.cpu.load_avg.1,
            data.cpu.load_avg.2
        ),
        width,
    ));
    for (chunk_idx, chunk) in data.cpu.percent_per_core.chunks(8).take(2).enumerate() {
        let row = chunk
            .iter()
            .enumerate()
            .map(|(idx, pct)| format!("c{:02} {:>4.1}%", chunk_idx * 8 + idx, pct))
            .collect::<Vec<_>>()
            .join("  ");
        lines.push(clip_line(&row, width));
    }

    push_title(&mut lines, "Memory");
    lines.push(clip_line(
        &format!(
            "RAM  {} {:>5.1}%  {} / {}",
            bar(data.memory.ram_percent, bar_width(width)),
            data.memory.ram_percent,
            fmt_bytes(data.memory.ram_used as f64),
            fmt_bytes(data.memory.ram_total as f64)
        ),
        width,
    ));
    lines.push(clip_line(
        &format!(
            "Swap {} {:>5.1}%  {} / {}",
            bar(data.memory.swap_percent, bar_width(width)),
            data.memory.swap_percent,
            fmt_bytes(data.memory.swap_used as f64),
            fmt_bytes(data.memory.swap_total as f64)
        ),
        width,
    ));

    push_title(&mut lines, "Disks");
    if data.disks.is_empty() {
        lines.push("no disks reported".to_string());
    } else {
        for disk in data.disks.iter().take(4) {
            lines.push(clip_line(
                &format!(
                    "{:<16} {} {:>5.1}%  {} / {}",
                    truncate_left(&disk.mount, 16),
                    bar(disk.percent, 14),
                    disk.percent,
                    fmt_bytes(disk.used as f64),
                    fmt_bytes(disk.total as f64)
                ),
                width,
            ));
        }
    }

    push_title(&mut lines, "Network");
    if data.network.interfaces.is_empty() {
        lines.push("no active interfaces".to_string());
    } else {
        for iface in data.network.interfaces.iter().take(4) {
            lines.push(clip_line(
                &format!(
                    "{:<10} up {:>9}/s  down {:>9}/s",
                    truncate_right(&iface.name, 10),
                    fmt_bytes(iface.up_bps),
                    fmt_bytes(iface.down_bps)
                ),
                width,
            ));
        }
    }

    push_title(&mut lines, "Top processes");
    let reserved_for_battery_and_footer = 6;
    let remaining = height
        .saturating_sub(lines.len() + reserved_for_battery_and_footer)
        .max(1);
    for proc in data.processes.iter().take(remaining.min(8)) {
        lines.push(clip_line(
            &format!(
                "{:>7} {:<24} {:>8}",
                proc.pid,
                truncate_right(&proc.name, 24),
                fmt_bytes(proc.memory_bytes as f64)
            ),
            width,
        ));
    }

    push_title(&mut lines, "Battery");
    lines.push(clip_line(render_battery(data).trim_end(), width));
    finish_frame(lines, data, width, height)
}

pub fn render_details_live(
    data: &Metrics,
    details: &SystemDetails,
    width: usize,
    height: usize,
) -> String {
    let mut lines = base_frame(data, "Details", 2, width);
    let mut rows = vec![
        (
            "Distro",
            details.distro.as_deref().unwrap_or("-").to_string(),
        ),
        ("Kernel", details.kernel.clone()),
        ("Arch", details.arch.clone()),
        (
            "CPU",
            details.cpu_model.as_deref().unwrap_or("-").to_string(),
        ),
        ("WSL RAM", fmt_bytes(details.vm_ram_total as f64)),
    ];
    rows.extend(
        details
            .windows
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone())),
    );
    push_title(&mut lines, "Host");
    push_kv_rows(&mut lines, &rows, width, height.saturating_sub(8));

    push_title(&mut lines, "Wi-Fi");
    if details.wifi.is_empty() {
        lines.push("no Wi-Fi interface".to_string());
    } else {
        let wifi = details
            .wifi
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect::<Vec<_>>();
        push_kv_rows(&mut lines, &wifi, width, 6);
    }
    finish_frame(lines, data, width, height)
}

pub fn render_sensors_live(
    data: &Metrics,
    gpu: &GpuTelemetry,
    width: usize,
    height: usize,
) -> String {
    let mut lines = base_frame(data, "Sensors", 3, width);
    push_title(&mut lines, "GPU");
    if let Some(g) = gpu.gpus.first() {
        lines.push(clip_line(&g.name, width));
        let vram = match (g.vram_used_mib, g.vram_total_mib) {
            (Some(used), Some(total)) => {
                format!("{:.1} / {:.1} GiB", used / 1024.0, total / 1024.0)
            }
            _ => "-".to_string(),
        };
        let rows = vec![
            ("Temp", opt(g.temp_c, " C", 0)),
            ("GPU util", opt(g.gpu_util_pct, "%", 0)),
            ("Mem I/O", opt(g.mem_util_pct, "%", 0)),
            ("VRAM", vram),
            ("Fan", opt(g.fan_pct, "%", 0)),
            ("Power", opt(g.power_w, " W", 1)),
            ("Core clock", opt(g.clock_core_mhz, " MHz", 0)),
            ("Mem clock", opt(g.clock_mem_mhz, " MHz", 0)),
        ];
        push_kv_rows(&mut lines, &rows, width, 8);
    } else {
        lines.push(clip_line(
            &format!(
                "GPU telemetry unavailable{}",
                gpu.error
                    .as_ref()
                    .filter(|e| !e.is_empty())
                    .map(|e| format!(": {e}"))
                    .unwrap_or_default()
            ),
            width,
        ));
    }

    push_title(&mut lines, "CPU / board sensors");
    if data.temperatures.is_empty() {
        lines.push("no sensors available".to_string());
    } else {
        for temp in data.temperatures.iter().take(4) {
            lines.push(clip_line(
                &format!(
                    "{} {:>5.1} C",
                    truncate_right(&temp.label, 28),
                    temp.current
                ),
                width,
            ));
        }
    }

    push_title(&mut lines, "GPU processes");
    if gpu.processes.is_empty() {
        lines.push("no compute processes visible".to_string());
    } else {
        let room = height.saturating_sub(lines.len() + 3).max(1);
        for proc in gpu.processes.iter().take(room.min(6)) {
            lines.push(clip_line(
                &format!(
                    "{:>7} {:<28} {:>8}",
                    proc.pid,
                    truncate_right(&proc.name, 28),
                    opt(proc.mem_mib, " MiB", 0)
                ),
                width,
            ));
        }
    }
    finish_frame(lines, data, width, height)
}

pub fn render_json(data: &Metrics, include_details: bool, include_sensors: bool) -> String {
    let mut parts = vec![
        format!(
            "\"host\":{{\"user\":\"{}\",\"hostname\":\"{}\",\"os\":\"{}\",\"distro\":{},\"arch\":\"{}\",\"uptime_s\":{},\"is_wsl\":{}}}",
            json_escape(&data.host.user),
            json_escape(&data.host.hostname),
            json_escape(&data.host.os),
            json_opt(data.host.distro.as_deref()),
            json_escape(&data.host.arch),
            data.host.uptime_s,
            data.host.is_wsl
        ),
        format!(
            "\"cpu\":{{\"percent_total\":{},\"percent_per_core\":[{}],\"logical_cores\":{},\"physical_cores\":{},\"freq_mhz\":{},\"load_avg\":[{},{},{}]}}",
            data.cpu.percent_total,
            data.cpu.percent_per_core.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","),
            data.cpu.logical_cores,
            data.cpu.physical_cores.map(|v| v.to_string()).unwrap_or_else(|| "null".to_string()),
            data.cpu.freq_mhz.map(|v| v.to_string()).unwrap_or_else(|| "null".to_string()),
            data.cpu.load_avg.0,
            data.cpu.load_avg.1,
            data.cpu.load_avg.2
        ),
        format!(
            "\"memory\":{{\"ram_total\":{},\"ram_used\":{},\"ram_available\":{},\"ram_percent\":{},\"swap_total\":{},\"swap_used\":{},\"swap_percent\":{}}}",
            data.memory.ram_total,
            data.memory.ram_used,
            data.memory.ram_available,
            data.memory.ram_percent,
            data.memory.swap_total,
            data.memory.swap_used,
            data.memory.swap_percent
        ),
        format!(
            "\"disks\":[{}]",
            data.disks
                .iter()
                .map(|d| format!(
                    "{{\"mount\":\"{}\",\"device\":\"{}\",\"fstype\":\"{}\",\"total\":{},\"used\":{},\"free\":{},\"percent\":{}}}",
                    json_escape(&d.mount), json_escape(&d.device), json_escape(&d.fstype), d.total, d.used, d.free, d.percent
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        format!(
            "\"network\":{{\"interfaces\":[{}]}}",
            data.network
                .interfaces
                .iter()
                .map(|i| format!(
                    "{{\"name\":\"{}\",\"up_bps\":{},\"down_bps\":{},\"total_sent\":{},\"total_recv\":{}}}",
                    json_escape(&i.name), i.up_bps, i.down_bps, i.total_sent, i.total_recv
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
    ];
    if include_details {
        let details = crate::collectors::system_details();
        parts.push(format!(
            "\"system_details\":{{\"wsl\":{},\"kernel\":\"{}\",\"distro\":{},\"arch\":\"{}\",\"cpu_model\":{},\"vm_ram_total\":{},\"windows\":{},\"wifi\":{}}}",
            details.wsl,
            json_escape(&details.kernel),
            json_opt(details.distro.as_deref()),
            json_escape(&details.arch),
            json_opt(details.cpu_model.as_deref()),
            details.vm_ram_total,
            json_pairs(&details.windows),
            json_pairs(&details.wifi)
        ));
    }
    if include_sensors {
        parts.push(format!(
            "\"gpu_telemetry\":{}",
            render_gpu_json(&crate::collectors::gpu_telemetry())
        ));
    }
    format!("{{{}}}", parts.join(","))
}

fn render_cpu(data: &Metrics) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Total {:>5.1}% | {}c/{}t | load {:.2} {:.2} {:.2}\n",
        data.cpu.percent_total,
        data.cpu.physical_cores.unwrap_or(data.cpu.logical_cores),
        data.cpu.logical_cores,
        data.cpu.load_avg.0,
        data.cpu.load_avg.1,
        data.cpu.load_avg.2
    ));
    for (idx, pct) in data.cpu.percent_per_core.iter().take(16).enumerate() {
        out.push_str(&format!("core {idx:>2} {} {:>5.1}%\n", bar(*pct, 24), pct));
    }
    out
}

fn render_memory(data: &Metrics) -> String {
    format!(
        "RAM  {} {:>5.1}%  {} / {}\nSwap {} {:>5.1}%  {} / {}\n",
        bar(data.memory.ram_percent, 24),
        data.memory.ram_percent,
        fmt_bytes(data.memory.ram_used as f64),
        fmt_bytes(data.memory.ram_total as f64),
        bar(data.memory.swap_percent, 24),
        data.memory.swap_percent,
        fmt_bytes(data.memory.swap_used as f64),
        fmt_bytes(data.memory.swap_total as f64)
    )
}

fn render_disks(data: &Metrics) -> String {
    if data.disks.is_empty() {
        return "no disks reported\n".to_string();
    }
    data.disks
        .iter()
        .map(|d| {
            format!(
                "{:<18} {} {:>5.1}%  {} / {}\n",
                truncate_left(&d.mount, 18),
                bar(d.percent, 18),
                d.percent,
                fmt_bytes(d.used as f64),
                fmt_bytes(d.total as f64)
            )
        })
        .collect()
}

fn render_network(data: &Metrics) -> String {
    if data.network.interfaces.is_empty() {
        return "no active interfaces\n".to_string();
    }
    data.network
        .interfaces
        .iter()
        .map(|i| {
            format!(
                "{:<12} up {:>9}/s  down {:>9}/s\n",
                i.name,
                fmt_bytes(i.up_bps),
                fmt_bytes(i.down_bps)
            )
        })
        .collect()
}

fn render_processes(data: &Metrics) -> String {
    data.processes
        .iter()
        .map(|p| {
            format!(
                "{:>7} {:<28} {}\n",
                p.pid,
                truncate_right(&p.name, 28),
                fmt_bytes(p.memory_bytes as f64)
            )
        })
        .collect()
}

fn render_battery(data: &Metrics) -> String {
    match &data.battery {
        Some(b) => format!(
            "{}{} ({})\n",
            b.percent
                .map(|v| format!("{v:.0}%"))
                .unwrap_or_else(|| "unknown".to_string()),
            b.charging
                .map(|v| if v { " charging" } else { " on battery" })
                .unwrap_or(""),
            b.source
        ),
        None => "no battery detected\n".to_string(),
    }
}

fn base_frame(data: &Metrics, page: &str, page_num: usize, width: usize) -> Vec<String> {
    vec![
        clip_line(
            &format!(
                "healthstatus  {} ({}/3)  {}@{}",
                page, page_num, data.host.user, data.host.hostname
            ),
            width,
        ),
        "-".repeat(width.max(1)),
    ]
}

fn finish_frame(mut lines: Vec<String>, data: &Metrics, width: usize, height: usize) -> String {
    let footer = vec![
        "-".repeat(width.max(1)),
        clip_line(
            &format!(
                "Uptime: {} | OS: {} | Arch: {}",
                fmt_duration(data.host.uptime_s),
                data.host.distro.as_deref().unwrap_or(&data.host.os),
                data.host.arch
            ),
            width,
        ),
        clip_line(
            "[1] Status  [2] Details  [3] Sensors  [tab] next  [q] quit",
            width,
        ),
    ];

    let max_body = height.saturating_sub(footer.len()).max(1);
    if lines.len() > max_body {
        lines.truncate(max_body);
    }
    while lines.len() < max_body {
        lines.push(String::new());
    }
    lines.extend(footer);
    lines
        .into_iter()
        .take(height.max(1))
        .map(|line| clip_line(&line, width))
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn push_title(lines: &mut Vec<String>, title: &str) {
    lines.push(String::new());
    lines.push(format!("== {title} =="));
}

fn push_kv_rows(lines: &mut Vec<String>, rows: &[(&str, String)], width: usize, max_rows: usize) {
    for (key, value) in rows.iter().take(max_rows) {
        lines.push(clip_line(
            &format!("{:<12} {}", truncate_right(key, 12), value),
            width,
        ));
    }
}

fn header(data: &Metrics, page: &str, page_num: usize) -> String {
    format!(
        "healthstatus | {}@{} | {} ({}/3)\n{}\n",
        data.host.user,
        data.host.hostname,
        page,
        page_num,
        "-".repeat(78)
    )
}

fn footer(data: &Metrics) -> String {
    let temps = if data.temperatures.is_empty() {
        String::new()
    } else {
        format!(
            " | Temps: {}",
            data.temperatures
                .iter()
                .take(3)
                .map(|t| format!("{} {:.0} C", t.label, t.current))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "{} | Uptime: {} | OS: {} | Arch: {}{}\n[1] Status  [2] Details  [3] Sensors  [tab] next  [q] quit\n",
        "-".repeat(78),
        fmt_duration(data.host.uptime_s),
        data.host.distro.as_deref().unwrap_or(&data.host.os),
        data.host.arch,
        temps
    )
}

fn section(title: &str, body: &str) -> String {
    format!("\n== {title} ==\n{body}")
}

fn kv(rows: &[(String, String)]) -> String {
    rows.iter().map(|(k, v)| format!("{k:<16} {v}\n")).collect()
}

fn kv_or_empty(rows: &[(String, String)], empty: &str) -> String {
    if rows.is_empty() {
        format!("{empty}\n")
    } else {
        kv(rows)
    }
}

fn bar(pct: f64, width: usize) -> String {
    let pct = pct.clamp(0.0, 100.0);
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    format!(
        "[{}{}]",
        "#".repeat(filled),
        "-".repeat(width.saturating_sub(filled))
    )
}

fn bar_width(width: usize) -> usize {
    width.saturating_sub(42).clamp(8, 24)
}

fn opt(value: Option<f64>, unit: &str, digits: usize) -> String {
    match value {
        Some(v) if digits == 0 => format!("{:.0}{unit}", v),
        Some(v) => format!("{v:.digits$}{unit}"),
        None => "-".to_string(),
    }
}

fn truncate_left(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value
            .chars()
            .rev()
            .take(max - 1)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
            .replacen("", "~", 1)
    }
}

fn truncate_right(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!("{}~", value.chars().take(max - 1).collect::<String>())
    }
}

fn clip_line(value: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    for ch in value.chars().take(max) {
        out.push(ch);
    }
    out
}

fn json_opt(value: Option<&str>) -> String {
    value
        .map(|v| format!("\"{}\"", json_escape(v)))
        .unwrap_or_else(|| "null".to_string())
}

fn json_pairs(rows: &[(String, String)]) -> String {
    format!(
        "{{{}}}",
        rows.iter()
            .map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn render_gpu_json(gpu: &GpuTelemetry) -> String {
    format!(
        "{{\"available\":{},\"error\":{},\"gpus\":[{}],\"processes\":[{}]}}",
        gpu.available,
        json_opt(gpu.error.as_deref()),
        gpu.gpus
            .iter()
            .map(|g| format!(
                "{{\"name\":\"{}\",\"temp_c\":{},\"gpu_util_pct\":{},\"mem_util_pct\":{},\"vram_used_mib\":{},\"vram_total_mib\":{},\"fan_pct\":{},\"power_w\":{},\"power_limit_w\":{},\"clock_core_mhz\":{},\"clock_mem_mhz\":{}}}",
                json_escape(&g.name),
                num(g.temp_c),
                num(g.gpu_util_pct),
                num(g.mem_util_pct),
                num(g.vram_used_mib),
                num(g.vram_total_mib),
                num(g.fan_pct),
                num(g.power_w),
                num(g.power_limit_w),
                num(g.clock_core_mhz),
                num(g.clock_mem_mhz)
            ))
            .collect::<Vec<_>>()
            .join(","),
        gpu.processes
            .iter()
            .map(|p| format!(
                "{{\"pid\":\"{}\",\"name\":\"{}\",\"mem_mib\":{}}}",
                json_escape(&p.pid),
                json_escape(&p.name),
                num(p.mem_mib)
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn num(value: Option<f64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string())
}
