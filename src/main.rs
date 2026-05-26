use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use healthstatus::collectors::{collect_all, gpu_telemetry, system_details, NetSnapshot};
use healthstatus::logging::CsvLogger;
use healthstatus::render::{render_details, render_json, render_sensors, render_status};

#[derive(Debug)]
struct Args {
    once: bool,
    details: bool,
    sensors: bool,
    json: bool,
    interval: f64,
    log: Option<PathBuf>,
}

fn main() -> ExitCode {
    match parse_args(env::args().skip(1)) {
        Ok(args) => {
            if args.json {
                let mut prev = None;
                warm_cpu();
                let data = collect_all(&mut prev);
                if let Some(path) = &args.log {
                    if let Err(err) = log_once(path, &data) {
                        eprintln!("healthstatus: failed to write log: {err}");
                        return ExitCode::from(1);
                    }
                }
                println!("{}", render_json(&data, args.details, args.sensors));
                return ExitCode::SUCCESS;
            }
            if args.once {
                warm_cpu();
                let mut prev = None;
                let data = collect_all(&mut prev);
                if let Some(path) = &args.log {
                    if let Err(err) = log_once(path, &data) {
                        eprintln!("healthstatus: failed to write log: {err}");
                        return ExitCode::from(1);
                    }
                }
                if args.details {
                    println!("{}", render_details(&data, &system_details()));
                } else if args.sensors {
                    println!("{}", render_sensors(&data, &gpu_telemetry()));
                } else {
                    println!("{}", render_status(&data));
                }
                return ExitCode::SUCCESS;
            }
            healthstatus::live::run(args.interval, args.log);
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn log_once(path: &PathBuf, data: &healthstatus::collectors::Metrics) -> std::io::Result<()> {
    let mut logger = CsvLogger::open(path)?;
    logger.write_sample(data)
}

fn warm_cpu() {
    let mut prev: Option<NetSnapshot> = None;
    let _ = collect_all(&mut prev);
    thread::sleep(Duration::from_millis(200));
}

fn parse_args<I>(args: I) -> Result<Args, String>
where
    I: IntoIterator<Item = String>,
{
    let mut parsed = Args {
        once: false,
        details: false,
        sensors: false,
        json: false,
        interval: 1.0,
        log: None,
    };
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("healthstatus {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--once" => parsed.once = true,
            "--details" => parsed.details = true,
            "--sensors" => parsed.sensors = true,
            "--json" => parsed.json = true,
            "--interval" => {
                let value = iter.next().ok_or("--interval requires a value")?;
                parsed.interval = value
                    .parse::<f64>()
                    .map_err(|_| "--interval must be a number")?;
            }
            "--log" => {
                let value = iter.next().ok_or("--log requires a file path")?;
                parsed.log = Some(PathBuf::from(value));
            }
            other => {
                return Err(format!(
                    "unknown argument: {other}\n\nRun `healthstatus --help`."
                ))
            }
        }
    }
    if parsed.interval <= 0.0 {
        return Err("--interval must be > 0".to_string());
    }
    if parsed.details && parsed.sensors {
        return Err("--details and --sensors are mutually exclusive".to_string());
    }
    Ok(parsed)
}

fn print_help() {
    println!(
        "healthstatus {}\n\nUSAGE:\n    healthstatus [OPTIONS]\n\nOPTIONS:\n    --once              Render one snapshot and exit\n    --details           Show system details with --once or --json\n    --sensors           Show GPU sensors with --once or --json\n    --json              Print metrics as JSON and exit\n    --interval <SEC>    Refresh interval for live dashboard (default: 1.0)\n    --log <FILE>        Append sampled metrics to a CSV file\n    --version           Print version\n    --help              Print help",
        env!("CARGO_PKG_VERSION")
    );
}
