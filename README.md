# healthstatus

Live terminal dashboard for machine health, now implemented in Rust.

It shows CPU, memory, disk, network, uptime, battery, top processes, system
details, and NVIDIA GPU telemetry from a single `healthstatus` command.

## Install

From the repo root:

```bash
cargo install --path .
```

From GitHub:

```bash
cargo install --git https://github.com/emilindqvist/healthstatus.git
```

For local development:

```bash
cargo run -- --once
cargo run -- --sensors
cargo test
```

Package name: `healthstatus`  
Command name: `healthstatus`

## Use

```bash
healthstatus                     # live dashboard, q / Ctrl-C to quit
healthstatus --once              # one status snapshot, then exit
healthstatus --once --details    # one system details snapshot
healthstatus --once --sensors    # one sensors snapshot
healthstatus --json              # raw metrics as JSON
healthstatus --json --details    # include system details in JSON
healthstatus --json --sensors    # include GPU telemetry in JSON
healthstatus --interval 0.5      # refresh every 0.5s
healthstatus --version           # print version
```

Live mode keys:

```text
1      Status
2      Details
3      Sensors
tab    Next page
q      Quit
```

## Screenshots

Screenshots/GIFs should live in `docs/` or `assets/` and be linked here once
captured:

```md
![healthstatus status view](docs/status.png)
![healthstatus sensors view](docs/sensors.png)
```

## WSL notes

- CPU and RAM reflect the WSL2 VM, not the full Windows host.
- Disks include Linux and mounted Windows drives when `df` reports them.
- Battery info in WSL is read from Windows through `powershell.exe`.
- System details in WSL use `powershell.exe` and Wi-Fi details use `netsh.exe`.
- GPU telemetry is primarily NVIDIA-focused and requires `nvidia-smi` or
  `nvidia-smi.exe` on `PATH`.
- CPU/board temperatures are best-effort from Linux thermal sysfs and are often
  unavailable inside WSL.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The GitHub Actions workflow runs those same checks on push and pull request.
