use crate::collectors::Metrics;

const CPU_WARN_PERCENT: f64 = 90.0;
const MEMORY_WARN_PERCENT: f64 = 90.0;
const DISK_WARN_PERCENT: f64 = 90.0;
const TEMP_WARN_C: f64 = 85.0;

pub fn warnings(data: &Metrics) -> Vec<String> {
    let mut warnings = Vec::new();
    if data.cpu.percent_total >= CPU_WARN_PERCENT {
        warnings.push(format!("CPU {:.1}%", data.cpu.percent_total));
    }
    if data.memory.ram_percent >= MEMORY_WARN_PERCENT {
        warnings.push(format!("RAM {:.1}%", data.memory.ram_percent));
    }
    warnings.extend(
        data.disks
            .iter()
            .filter(|disk| disk.percent >= DISK_WARN_PERCENT)
            .map(|disk| format!("disk {} {:.1}%", disk.mount, disk.percent)),
    );
    warnings.extend(
        data.temperatures
            .iter()
            .filter(|temp| temp.current >= TEMP_WARN_C)
            .map(|temp| format!("{} {:.1} C", temp.label, temp.current)),
    );
    warnings
}
