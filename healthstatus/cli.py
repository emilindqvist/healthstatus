"""Command-line entry point for healthstatus."""

from __future__ import annotations

import argparse
import json
import sys
import time

from healthstatus import __version__


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="healthstatus",
        description="Live TUI dashboard of your machine's health.",
    )
    parser.add_argument("--version", action="version", version=f"healthstatus {__version__}")
    parser.add_argument(
        "--once",
        action="store_true",
        help="Render one snapshot and exit (does not take over the screen).",
    )
    parser.add_argument(
        "--details",
        action="store_true",
        help="With --once, render the System details page instead of Status.",
    )
    parser.add_argument(
        "--sensors",
        action="store_true",
        help="With --once, render the Sensors (GPU telemetry) page instead of Status.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Print collected metrics as JSON and exit.",
    )
    parser.add_argument(
        "--interval",
        type=float,
        default=1.0,
        help="Refresh interval in seconds for the live dashboard (default: 1.0).",
    )
    args = parser.parse_args(argv)

    if args.interval <= 0:
        parser.error("--interval must be > 0")

    if args.details and args.sensors:
        parser.error("--details and --sensors are mutually exclusive")

    if args.json:
        return _run_json(include_details=args.details, include_sensors=args.sensors)
    if args.once:
        if args.details:
            from healthstatus.dashboard import render_details_once
            render_details_once()
        elif args.sensors:
            from healthstatus.dashboard import render_sensors_once
            render_sensors_once()
        else:
            from healthstatus.dashboard import render_once
            render_once()
        return 0

    from healthstatus.dashboard import run_live
    run_live(interval=args.interval)
    return 0


def _run_json(include_details: bool = False, include_sensors: bool = False) -> int:
    import psutil
    from healthstatus import collectors

    psutil.cpu_percent(interval=None, percpu=True)
    time.sleep(0.2)
    data = collectors.collect_all(None, 1.0)
    data["network"].pop("_snapshot", None)
    if include_details:
        data["system_details"] = collectors.system_details()
    if include_sensors:
        data["gpu_telemetry"] = collectors.gpu_telemetry()
    json.dump(data, sys.stdout, indent=2, default=str)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
