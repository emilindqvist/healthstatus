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
