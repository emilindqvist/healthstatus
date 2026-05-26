pub mod alerts;
pub mod collectors;
pub mod live;
pub mod logging;
pub mod render;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn fmt_bytes(mut n: f64) -> String {
    for unit in ["B", "K", "M", "G", "T", "P"] {
        if n.abs() < 1024.0 {
            return format!("{n:3.1}{unit}");
        }
        n /= 1024.0;
    }
    format!("{n:.1}E")
}

pub fn fmt_duration(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m {secs}s")
    }
}

pub fn parse_nv_number(raw: &str) -> Option<f64> {
    let s = raw.trim();
    if s.is_empty() || matches!(s, "[N/A]" | "[Not Supported]" | "N/A") {
        return None;
    }
    s.parse::<f64>().ok()
}

pub fn parse_wmi_date(value: &str) -> Option<String> {
    let start = value.find("/Date(")? + 6;
    let end = value[start..].find(")/")? + start;
    let millis = value[start..end].parse::<i64>().ok()?;
    let secs = millis.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    civil_from_days(days).map(|(year, month, day)| format!("{year:04}-{month:02}-{day:02}"))
}

fn civil_from_days(days_since_epoch: i64) -> Option<(i32, u32, u32)> {
    let z = days_since_epoch.checked_add(719_468)?;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    Some((year as i32, m as u32, d as u32))
}

pub fn unix_now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64()
}

pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
