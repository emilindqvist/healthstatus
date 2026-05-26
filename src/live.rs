use std::io::{self, Read, Write};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::collectors::{collect_all, gpu_telemetry, system_details, NetSnapshot, SystemDetails};
use crate::render::{render_details_live, render_sensors_live, render_status_live};

pub fn run(interval: f64) {
    let interval = Duration::from_secs_f64(interval);
    let (tx, rx) = mpsc::channel::<u8>();
    let _guard = RawMode::new();
    thread::spawn(move || {
        let mut stdin = io::stdin();
        loop {
            let mut buf = [0_u8; 1];
            if stdin.read_exact(&mut buf).is_ok() {
                let _ = tx.send(buf[0]);
            }
        }
    });

    let mut page = 1_u8;
    let mut prev_net: Option<NetSnapshot> = None;
    let mut cached_details: Option<SystemDetails> = None;
    let mut details_fetched = Instant::now() - Duration::from_secs(60);

    print!("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
    let _ = io::stdout().flush();

    loop {
        let (width, height) = terminal_size();
        let data = collect_all(&mut prev_net);
        let frame = match page {
            2 => {
                if cached_details.is_none() || details_fetched.elapsed() > Duration::from_secs(15) {
                    cached_details = Some(system_details());
                    details_fetched = Instant::now();
                }
                render_details_live(
                    &data,
                    cached_details.as_ref().expect("details cached"),
                    width,
                    height,
                )
            }
            3 => render_sensors_live(&data, &gpu_telemetry(), width, height),
            _ => render_status_live(&data, width, height),
        };
        print!("\x1b[H{frame}\x1b[J");
        let _ = io::stdout().flush();

        let deadline = Instant::now() + interval;
        while Instant::now() < deadline {
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));
            match rx.recv_timeout(wait) {
                Ok(b'q' | b'Q' | 3 | 4) => return,
                Ok(b'1') => {
                    page = 1;
                    break;
                }
                Ok(b'2') => {
                    page = 2;
                    break;
                }
                Ok(b'3') => {
                    page = 3;
                    break;
                }
                Ok(b'\t') => {
                    page = page % 3 + 1;
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }
}

fn terminal_size() -> (usize, usize) {
    let output = Command::new("sh")
        .args(["-c", "stty size 2>/dev/null"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let mut parts = output.split_whitespace();
    let rows = parts
        .next()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(30);
    let cols = parts
        .next()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100);
    (cols.max(40), rows.max(12))
}

struct RawMode;

impl RawMode {
    fn new() -> Self {
        let _ = Command::new("sh")
            .args(["-c", "stty -echo -icanon min 1 time 0 2>/dev/null"])
            .status();
        Self
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = Command::new("sh")
            .args(["-c", "stty sane 2>/dev/null"])
            .status();
        print!("\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}
