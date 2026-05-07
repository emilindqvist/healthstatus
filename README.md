# healthstatus

Live terminal dashboard showing the health of your machine — CPU, memory, disk,
network, uptime, battery, and top processes — in one refreshing view.

## Install

From the repo root:

```bash
pipx install --editable .
# or, inside a venv:
pip install -e .
```

## Use

```bash
healthstatus                 # live dashboard, Ctrl-C / q to quit
healthstatus --once          # one snapshot, then exit (pipe-friendly)
healthstatus --json          # dump raw metrics as JSON
healthstatus --interval 0.5  # refresh every 0.5s
```

## WSL notes

You run this from a WSL terminal, so a few things are worth knowing:

- **CPU and RAM** reflect the WSL2 VM, not the full Windows host. WSL2 usually
  allocates about half of the host's RAM dynamically, so the numbers here show
  what is available to your Linux environment — which is typically what you
  actually care about for dev work.
- **Disks**: `/mnt/c`, `/mnt/d`, etc. show the real Windows drive usage.
- **Battery**: psutil can't read battery info from inside WSL2, so the tool
  shells out to `powershell.exe` to read `Win32_Battery` from Windows.
- **Temperatures**: usually unavailable in WSL. If `psutil.sensors_temperatures`
  returns nothing, the temperature line is hidden.
