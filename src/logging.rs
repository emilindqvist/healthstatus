use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::collectors::Metrics;
use crate::unix_now_secs;

const CSV_HEADER: &str = concat!(
    "timestamp_s,hostname,cpu_percent,ram_percent,ram_used_bytes,ram_total_bytes,",
    "swap_percent,swap_used_bytes,swap_total_bytes,disk_max_percent,",
    "network_up_bps,network_down_bps,battery_percent\n"
);

pub struct CsvLogger {
    file: File,
}

impl CsvLogger {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let needs_header = path.metadata().map(|m| m.len() == 0).unwrap_or(true);
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        if needs_header {
            file.write_all(CSV_HEADER.as_bytes())?;
        }
        Ok(Self { file })
    }

    pub fn write_sample(&mut self, data: &Metrics) -> io::Result<()> {
        self.file.write_all(csv_row(data).as_bytes())?;
        self.file.flush()
    }
}

pub fn csv_row(data: &Metrics) -> String {
    let disk_max = data
        .disks
        .iter()
        .map(|disk| disk.percent)
        .fold(0.0, f64::max);
    let network_up = data
        .network
        .interfaces
        .iter()
        .map(|iface| iface.up_bps)
        .sum::<f64>();
    let network_down = data
        .network
        .interfaces
        .iter()
        .map(|iface| iface.down_bps)
        .sum::<f64>();
    let battery = data
        .battery
        .as_ref()
        .and_then(|battery| battery.percent)
        .map(|pct| format!("{pct:.1}"))
        .unwrap_or_default();

    format!(
        "{:.3},{},{:.1},{:.1},{},{},{:.1},{},{},{:.1},{:.1},{:.1},{}\n",
        unix_now_secs(),
        csv_escape(&data.host.hostname),
        data.cpu.percent_total,
        data.memory.ram_percent,
        data.memory.ram_used,
        data.memory.ram_total,
        data.memory.swap_percent,
        data.memory.swap_used,
        data.memory.swap_total,
        disk_max,
        network_up,
        network_down,
        battery
    )
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
