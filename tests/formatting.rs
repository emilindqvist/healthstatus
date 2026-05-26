use healthstatus::alerts::warnings;
use healthstatus::collectors::{
    Cpu, Disk, Host, Memory, Metrics, Network, NetworkInterface, Temperature,
};
use healthstatus::logging::csv_row;
use healthstatus::{fmt_bytes, fmt_duration, parse_nv_number, parse_wmi_date};

#[test]
fn formats_bytes() {
    assert_eq!(fmt_bytes(0.0), "0.0B");
    assert_eq!(fmt_bytes(1024.0), "1.0K");
    assert_eq!(fmt_bytes(1536.0), "1.5K");
    assert_eq!(fmt_bytes(1024.0 * 1024.0), "1.0M");
}

#[test]
fn formats_duration() {
    assert_eq!(fmt_duration(59.0), "0m 59s");
    assert_eq!(fmt_duration(60.0), "1m 0s");
    assert_eq!(fmt_duration(3661.0), "1h 1m");
    assert_eq!(fmt_duration(90_061.0), "1d 1h 1m");
}

#[test]
fn parses_nvidia_numbers() {
    assert_eq!(parse_nv_number("42"), Some(42.0));
    assert_eq!(parse_nv_number("12.5"), Some(12.5));
    assert_eq!(parse_nv_number("[N/A]"), None);
    assert_eq!(parse_nv_number("[Not Supported]"), None);
    assert_eq!(parse_nv_number("wat"), None);
}

#[test]
fn parses_wmi_dates() {
    assert_eq!(
        parse_wmi_date("/Date(1587340800000)/").as_deref(),
        Some("2020-04-20")
    );
    assert_eq!(parse_wmi_date("not a date"), None);
    assert_eq!(parse_wmi_date("/Date(nope)/"), None);
}

#[test]
fn renders_csv_metrics_row() {
    let data = Metrics {
        host: Host {
            hostname: "host,one".to_string(),
            ..Host::default()
        },
        cpu: Cpu {
            percent_total: 12.3,
            ..Cpu::default()
        },
        memory: Memory {
            ram_total: 100,
            ram_used: 40,
            ram_percent: 40.0,
            swap_total: 50,
            swap_used: 5,
            swap_percent: 10.0,
            ..Memory::default()
        },
        network: Network {
            interfaces: vec![NetworkInterface {
                name: "eth0".to_string(),
                up_bps: 1.5,
                down_bps: 2.5,
                total_sent: 0,
                total_recv: 0,
            }],
        },
        ..Metrics::default()
    };

    let row = csv_row(&data);

    assert!(row.contains("\"host,one\""));
    assert!(row.contains(",12.3,40.0,40,100,10.0,5,50,0.0,1.5,2.5,"));
}

#[test]
fn reports_threshold_warnings() {
    let data = Metrics {
        cpu: Cpu {
            percent_total: 95.0,
            ..Cpu::default()
        },
        memory: Memory {
            ram_percent: 92.0,
            ..Memory::default()
        },
        disks: vec![Disk {
            mount: "/".to_string(),
            device: "/dev/sda1".to_string(),
            fstype: "ext4".to_string(),
            total: 100,
            used: 95,
            free: 5,
            percent: 95.0,
        }],
        temperatures: vec![Temperature {
            chip: "coretemp".to_string(),
            label: "CPU".to_string(),
            current: 86.0,
            high: None,
        }],
        ..Metrics::default()
    };

    assert_eq!(
        warnings(&data),
        vec!["CPU 95.0%", "RAM 92.0%", "disk / 95.0%", "CPU 86.0 C"]
    );
}
