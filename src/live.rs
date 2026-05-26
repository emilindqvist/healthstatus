use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table},
    Frame, Terminal,
};

use crate::collectors::{
    collect_all, gpu_telemetry, system_details, GpuTelemetry, Metrics, NetSnapshot, SystemDetails,
};
use crate::{fmt_bytes, fmt_duration};

const DETAILS_TTL: Duration = Duration::from_secs(15);
const GPU_TTL: Duration = Duration::from_secs(3);

pub fn run(interval: f64) {
    if let Err(err) = run_tui(Duration::from_secs_f64(interval)) {
        eprintln!("healthstatus: {err}");
    }
}

fn run_tui(interval: Duration) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_app(&mut terminal, interval);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    interval: Duration,
) -> io::Result<()> {
    let mut page = Page::Status;
    let mut prev_net: Option<NetSnapshot> = None;
    let mut data = collect_all(&mut prev_net);
    let mut cached_details: Option<SystemDetails> = None;
    let mut details_fetched = Instant::now() - Duration::from_secs(60);
    let mut details_pending = false;
    let (details_tx, details_rx) = mpsc::channel::<SystemDetails>();
    let mut cached_gpu: Option<GpuTelemetry> = None;
    let mut gpu_fetched = Instant::now() - Duration::from_secs(60);
    let mut gpu_pending = false;
    let (gpu_tx, gpu_rx) = mpsc::channel::<GpuTelemetry>();
    let mut next_refresh = Instant::now();
    let poll_rate = Duration::from_millis(50);

    loop {
        while let Ok(details) = details_rx.try_recv() {
            cached_details = Some(details);
            details_fetched = Instant::now();
            details_pending = false;
        }
        while let Ok(gpu) = gpu_rx.try_recv() {
            cached_gpu = Some(gpu);
            gpu_fetched = Instant::now();
            gpu_pending = false;
        }

        if next_refresh <= Instant::now() {
            data = collect_all(&mut prev_net);
            next_refresh = Instant::now() + interval;
        }

        if page == Page::Details
            && !details_pending
            && (cached_details.is_none() || details_fetched.elapsed() > DETAILS_TTL)
        {
            details_pending = true;
            let tx = details_tx.clone();
            thread::spawn(move || {
                let _ = tx.send(system_details());
            });
        }

        if page == Page::Sensors
            && !gpu_pending
            && (cached_gpu.is_none() || gpu_fetched.elapsed() > GPU_TTL)
        {
            gpu_pending = true;
            let tx = gpu_tx.clone();
            thread::spawn(move || {
                let _ = tx.send(gpu_telemetry());
            });
        }

        terminal.draw(|frame| match page {
            Page::Status => draw_status(frame, &data),
            Page::Details => draw_details(frame, &data, cached_details.as_ref(), details_pending),
            Page::Sensors => draw_sensors(frame, &data, cached_gpu.as_ref(), gpu_pending),
        })?;

        if event::poll(poll_rate)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Char('1') => page = Page::Status,
                    KeyCode::Char('2') => page = Page::Details,
                    KeyCode::Char('3') => page = Page::Sensors,
                    KeyCode::Tab => page = page.next(),
                    _ => {}
                }
            }
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Page {
    Status,
    Details,
    Sensors,
}

impl Page {
    fn next(self) -> Self {
        match self {
            Self::Status => Self::Details,
            Self::Details => Self::Sensors,
            Self::Sensors => Self::Status,
        }
    }
}

fn draw_status(frame: &mut Frame<'_>, data: &Metrics) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(11),
            Constraint::Length(7),
            Constraint::Min(7),
            Constraint::Length(3),
        ])
        .split(area);

    draw_top_bar(frame, rows[0], data, "Status", 1);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(48),
            Constraint::Percentage(34),
            Constraint::Percentage(18),
        ])
        .split(rows[1]);
    draw_cpu(frame, top[0], data);
    draw_memory(frame, top[1], data);
    draw_battery(frame, top[2], data);

    draw_disks(frame, rows[2], data);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(rows[3]);
    draw_network(frame, bottom[0], data);
    draw_processes(frame, bottom[1], data);

    draw_footer(frame, rows[4]);
}

fn draw_details(
    frame: &mut Frame<'_>,
    data: &Metrics,
    details: Option<&SystemDetails>,
    pending: bool,
) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(8),
            Constraint::Length(3),
        ])
        .split(area);
    draw_top_bar(frame, rows[0], data, "Details", 2);
    let Some(details) = details else {
        draw_loading(frame, rows[1], "Host", pending);
        draw_loading(frame, rows[2], "Wi-Fi", pending);
        draw_footer(frame, rows[3]);
        return;
    };

    let host_rows = vec![
        row_pair("Distro", details.distro.as_deref().unwrap_or("-")),
        row_pair("Kernel", &details.kernel),
        row_pair("Arch", &details.arch),
        row_pair("CPU", details.cpu_model.as_deref().unwrap_or("-")),
        row_pair("WSL RAM", fmt_bytes(details.vm_ram_total as f64)),
    ]
    .into_iter()
    .chain(
        details
            .windows
            .iter()
            .map(|(key, value)| row_pair(key, value)),
    )
    .collect::<Vec<_>>();
    frame.render_widget(
        Table::new(host_rows, [Constraint::Length(14), Constraint::Min(20)])
            .block(panel("Host", Color::Cyan))
            .column_spacing(2),
        rows[1],
    );

    let wifi_rows = if details.wifi.is_empty() {
        vec![Row::new(vec![
            Cell::from("no Wi-Fi interface").style(Style::new().fg(Color::DarkGray))
        ])]
    } else {
        details
            .wifi
            .iter()
            .map(|(key, value)| row_pair(key, value))
            .collect()
    };
    frame.render_widget(
        Table::new(wifi_rows, [Constraint::Length(14), Constraint::Min(20)])
            .block(panel("Wi-Fi", Color::Green))
            .column_spacing(2),
        rows[2],
    );
    draw_footer(frame, rows[3]);
}

fn draw_sensors(frame: &mut Frame<'_>, data: &Metrics, gpu: Option<&GpuTelemetry>, pending: bool) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(8),
            Constraint::Length(3),
        ])
        .split(area);
    draw_top_bar(frame, rows[0], data, "Sensors", 3);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(rows[1]);
    if let Some(gpu) = gpu {
        draw_gpu(frame, top[0], gpu);
        draw_gpu_processes(frame, rows[2], gpu);
    } else {
        draw_loading(frame, top[0], "GPU", pending);
        draw_loading(frame, rows[2], "GPU processes", pending);
    }
    draw_temperatures(frame, top[1], data);
    draw_footer(frame, rows[3]);
}

fn draw_top_bar(frame: &mut Frame<'_>, area: Rect, data: &Metrics, page: &str, page_num: u8) {
    let text = Line::from(vec![
        Span::styled(
            "healthstatus",
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  {}@{}  {}  {}  uptime {}  {page} ({page_num}/3)",
            data.host.user,
            data.host.hostname,
            data.host.distro.as_deref().unwrap_or(&data.host.os),
            data.host.arch,
            fmt_duration(data.host.uptime_s)
        )),
    ]);
    frame.render_widget(Paragraph::new(text).block(panel("", Color::White)), area);
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect) {
    let text = Line::from(vec![
        Span::styled("[1]", Style::new().fg(Color::Cyan)),
        Span::raw(" Status   "),
        Span::styled("[2]", Style::new().fg(Color::Cyan)),
        Span::raw(" Details   "),
        Span::styled("[3]", Style::new().fg(Color::Cyan)),
        Span::raw(" Sensors   "),
        Span::styled("[tab]", Style::new().fg(Color::Cyan)),
        Span::raw(" next   "),
        Span::styled("[q]", Style::new().fg(Color::Cyan)),
        Span::raw(" quit"),
    ]);
    frame.render_widget(Paragraph::new(text).block(panel("", Color::White)), area);
}

fn draw_cpu(frame: &mut Frame<'_>, area: Rect, data: &Metrics) {
    let title = format!(
        "CPU ({}c/{}t) - load {:.2}",
        data.cpu.physical_cores.unwrap_or(data.cpu.logical_cores),
        data.cpu.logical_cores,
        data.cpu.load_avg.0
    );
    let lines = data
        .cpu
        .percent_per_core
        .chunks(2)
        .take(area.height.saturating_sub(2) as usize)
        .enumerate()
        .map(|(row_idx, chunk)| {
            let left_idx = row_idx * 2;
            let mut spans = core_spans(left_idx, chunk[0]);
            if let Some(pct) = chunk.get(1) {
                spans.push(Span::raw("   "));
                spans.extend(core_spans(left_idx + 1, *pct));
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).block(panel(&title, Color::Cyan)),
        area,
    );
}

fn draw_memory(frame: &mut Frame<'_>, area: Rect, data: &Metrics) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(area);
    frame.render_widget(
        Block::default().borders(Borders::ALL).title(" Memory "),
        area,
    );
    draw_gauge(
        frame,
        chunks[0],
        "RAM",
        data.memory.ram_percent,
        format!(
            "{} / {}",
            fmt_bytes(data.memory.ram_used as f64),
            fmt_bytes(data.memory.ram_total as f64)
        ),
    );
    draw_gauge(
        frame,
        chunks[1],
        "Swap",
        data.memory.swap_percent,
        format!(
            "{} / {}",
            fmt_bytes(data.memory.swap_used as f64),
            fmt_bytes(data.memory.swap_total as f64)
        ),
    );
}

fn draw_battery(frame: &mut Frame<'_>, area: Rect, data: &Metrics) {
    let (text, color) = match &data.battery {
        Some(battery) => {
            let state = battery
                .charging
                .map(|charging| if charging { "charging" } else { "on battery" })
                .unwrap_or("unknown");
            (
                format!(
                    "{}\n{}\nsource: {}",
                    battery
                        .percent
                        .map(|pct| format!("{pct:.0}%"))
                        .unwrap_or_else(|| "unknown".to_string()),
                    state,
                    battery.source
                ),
                usage_color(battery.percent.unwrap_or(0.0)),
            )
        }
        None => ("no battery\ndetected".to_string(), Color::DarkGray),
    };
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::new().fg(color))
            .block(panel("Battery", Color::Yellow)),
        area,
    );
}

fn draw_disks(frame: &mut Frame<'_>, area: Rect, data: &Metrics) {
    let rows = data.disks.iter().take(4).map(|disk| {
        Row::new(vec![
            Cell::from(disk.mount.clone()),
            Cell::from(progress_bar(disk.percent, 16))
                .style(Style::new().fg(usage_color(disk.percent))),
            Cell::from(format!("{:>5.1}%", disk.percent)),
            Cell::from(format!(
                "{} / {}",
                fmt_bytes(disk.used as f64),
                fmt_bytes(disk.total as f64)
            )),
        ])
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(18),
                Constraint::Length(18),
                Constraint::Length(8),
                Constraint::Min(18),
            ],
        )
        .block(panel("Disks", Color::Blue))
        .column_spacing(1),
        area,
    );
}

fn draw_network(frame: &mut Frame<'_>, area: Rect, data: &Metrics) {
    let rows = data.network.interfaces.iter().take(6).map(|iface| {
        Row::new(vec![
            Cell::from(iface.name.clone()).style(Style::new().add_modifier(Modifier::BOLD)),
            Cell::from(format!("{}/s", fmt_bytes(iface.up_bps))),
            Cell::from(format!("{}/s", fmt_bytes(iface.down_bps))),
        ])
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Length(14),
                Constraint::Length(14),
            ],
        )
        .header(Row::new(vec!["iface", "up", "down"]).style(Style::new().fg(Color::Green)))
        .block(panel("Network", Color::Green))
        .column_spacing(2),
        area,
    );
}

fn draw_processes(frame: &mut Frame<'_>, area: Rect, data: &Metrics) {
    let rows = data.processes.iter().take(8).map(|proc| {
        Row::new(vec![
            Cell::from(proc.pid.to_string()),
            Cell::from(proc.name.clone()),
            Cell::from(fmt_bytes(proc.memory_bytes as f64)),
        ])
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Min(16),
                Constraint::Length(10),
            ],
        )
        .header(Row::new(vec!["pid", "process", "rss"]).style(Style::new().fg(Color::Red)))
        .block(panel("Top processes", Color::Red))
        .column_spacing(2),
        area,
    );
}

fn draw_gpu(frame: &mut Frame<'_>, area: Rect, gpu: &GpuTelemetry) {
    let Some(g) = gpu.gpus.first() else {
        let text = gpu
            .error
            .as_deref()
            .filter(|error| !error.is_empty())
            .unwrap_or("GPU telemetry unavailable");
        frame.render_widget(
            Paragraph::new(text)
                .style(Style::new().fg(Color::Yellow))
                .block(panel("GPU", Color::Green)),
            area,
        );
        return;
    };

    let rows = vec![
        row_pair("Name", &g.name),
        row_pair("Temp", opt_num(g.temp_c, " C", 0)),
        row_pair("GPU util", opt_num(g.gpu_util_pct, "%", 0)),
        row_pair("Mem I/O", opt_num(g.mem_util_pct, "%", 0)),
        row_pair(
            "VRAM",
            &match (g.vram_used_mib, g.vram_total_mib) {
                (Some(used), Some(total)) => {
                    format!("{:.1} / {:.1} GiB", used / 1024.0, total / 1024.0)
                }
                _ => "-".to_string(),
            },
        ),
        row_pair("Fan", opt_num(g.fan_pct, "%", 0)),
        row_pair("Power", opt_num(g.power_w, " W", 1)),
        row_pair("Core clock", opt_num(g.clock_core_mhz, " MHz", 0)),
        row_pair("Mem clock", opt_num(g.clock_mem_mhz, " MHz", 0)),
    ];
    frame.render_widget(
        Table::new(rows, [Constraint::Length(12), Constraint::Min(20)])
            .block(panel("GPU", Color::Green))
            .column_spacing(2),
        area,
    );
}

fn draw_temperatures(frame: &mut Frame<'_>, area: Rect, data: &Metrics) {
    if data.temperatures.is_empty() {
        frame.render_widget(
            Paragraph::new("no sensors available")
                .style(Style::new().fg(Color::DarkGray))
                .block(panel("CPU / board sensors", Color::Yellow)),
            area,
        );
        return;
    }
    let rows = data.temperatures.iter().take(8).map(|temp| {
        Row::new(vec![
            Cell::from(temp.label.clone()),
            Cell::from(format!("{:.1} C", temp.current))
                .style(Style::new().fg(temp_color(temp.current))),
        ])
    });
    frame.render_widget(
        Table::new(rows, [Constraint::Min(18), Constraint::Length(10)])
            .block(panel("CPU / board sensors", Color::Yellow)),
        area,
    );
}

fn draw_gpu_processes(frame: &mut Frame<'_>, area: Rect, gpu: &GpuTelemetry) {
    let rows = gpu.processes.iter().take(8).map(|proc| {
        Row::new(vec![
            Cell::from(proc.pid.clone()),
            Cell::from(proc.name.clone()),
            Cell::from(opt_num(proc.mem_mib, " MiB", 0)),
        ])
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Min(20),
                Constraint::Length(12),
            ],
        )
        .header(Row::new(vec!["pid", "process", "vram"]).style(Style::new().fg(Color::Cyan)))
        .block(panel("GPU processes", Color::Cyan)),
        area,
    );
}

fn draw_loading(frame: &mut Frame<'_>, area: Rect, title: &str, pending: bool) {
    let text = if pending {
        "loading cached data..."
    } else {
        "waiting for data..."
    };
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::new().fg(Color::DarkGray))
            .block(panel(title, Color::DarkGray)),
        area,
    );
}

fn draw_gauge(frame: &mut Frame<'_>, area: Rect, label: &str, percent: f64, detail: String) {
    let pct = percent.clamp(0.0, 100.0);
    let title = format!("{label}  {pct:>5.1}%  {detail}");
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::new().fg(usage_color(pct)))
            .ratio(pct / 100.0)
            .label(title),
        area,
    );
}

fn core_spans(idx: usize, pct: f64) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("c{idx:02} "), Style::new().fg(Color::DarkGray)),
        Span::styled(progress_bar(pct, 8), Style::new().fg(usage_color(pct))),
        Span::raw(format!(" {:>4.1}%", pct)),
    ]
}

fn progress_bar(percent: f64, width: usize) -> String {
    let pct = percent.clamp(0.0, 100.0);
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(width.saturating_sub(filled))
    )
}

fn panel(title: &str, color: Color) -> Block<'_> {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(color));
    if title.is_empty() {
        block
    } else {
        block.title(format!(" {title} "))
    }
}

fn row_pair(label: impl Into<String>, value: impl Into<String>) -> Row<'static> {
    Row::new(vec![
        Cell::from(label.into()).style(Style::new().fg(Color::DarkGray)),
        Cell::from(value.into()),
    ])
}

fn usage_color(percent: f64) -> Color {
    if percent >= 90.0 {
        Color::Red
    } else if percent >= 75.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn temp_color(temp: f64) -> Color {
    if temp >= 85.0 {
        Color::Red
    } else if temp >= 75.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn opt_num(value: Option<f64>, unit: &str, digits: usize) -> String {
    match value {
        Some(value) if digits == 0 => format!("{value:.0}{unit}"),
        Some(value) => format!("{value:.digits$}{unit}"),
        None => "-".to_string(),
    }
}
