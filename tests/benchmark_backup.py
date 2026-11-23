#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
"""
Onesimus Director — Backup Throughput Benchmark

Runs a Full Backup via both C++ and Rust Director, monitors progress,
measures throughput over time, generates comparative PDF report.

Usage:
    python3 tests/benchmark_backup.py [--cpp-port 9101] [--rust-port 29101]

Auto-creates .venv with matplotlib if needed.
Requires: bareos-fd on port 9102, bareos-sd on port 9103, both Directors running.
"""

import os
import subprocess
import sys
from pathlib import Path

# =============================================================================
# Bootstrap venv (same as compare_directors.py)
# =============================================================================

VENV_DIR = Path(__file__).parent.parent / ".venv"
VENV_PYTHON = VENV_DIR / "bin" / "python3"
REQUIREMENTS = ["matplotlib"]

def ensure_venv():
    if sys.prefix == str(VENV_DIR):
        return
    if not VENV_PYTHON.exists():
        print(f"Creating venv at {VENV_DIR} ...")
        try:
            subprocess.check_call([sys.executable, "-m", "venv", str(VENV_DIR)])
        except subprocess.CalledProcessError:
            if Path("/etc/debian_version").exists():
                print("Install with: sudo apt install python3-venv")
            elif Path("/etc/redhat-release").exists():
                print("Install with: sudo dnf install python3-devel")
            elif Path("/etc/arch-release").exists():
                print("Install with: sudo pacman -S python")
            else:
                print("Install python3-venv for your distribution.")
            sys.exit(1)
    for pkg in REQUIREMENTS:
        try:
            subprocess.check_call([str(VENV_PYTHON), "-c", f"import {pkg}"],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except subprocess.CalledProcessError:
            print(f"Installing {pkg} ...")
            subprocess.check_call([str(VENV_DIR / "bin" / "pip"), "install", "-q", pkg])
    os.execv(str(VENV_PYTHON), [str(VENV_PYTHON), __file__] + sys.argv[1:])

ensure_venv()

# --- Inside venv ---

import argparse
import json
import re
import tempfile
import time
import threading
from dataclasses import dataclass, field

BCONSOLE = os.environ.get("BCONSOLE", "/usr/bin/bconsole")
PASSWORD = os.environ.get("PASSWORD", "SlC3mJTFz3DnQX9ZEJwWJgAIoxe5GbD/OmyQADlFrN9A")

# =============================================================================
# Data
# =============================================================================

@dataclass
class BackupSample:
    elapsed_s: float
    files: int
    bytes: int
    throughput_mbps: float  # MB/s at this point

@dataclass
class BackupRun:
    director: str       # "C++" or "Rust"
    port: int
    job_id: int = 0
    total_files: int = 0
    total_bytes: int = 0
    duration_s: float = 0
    avg_throughput_mbps: float = 0
    status: str = ""
    samples: list = field(default_factory=list)

# =============================================================================
# Helpers
# =============================================================================

STRIP_RE = re.compile(
    r"^(Connecting to|Enter a period|You are connected|"
    r" Encryption:|1000 OK:).*$", re.MULTILINE
)

def bconsole_cmd(conf_path: str, command: str, timeout: int = 10) -> str:
    try:
        proc = subprocess.run(
            [BCONSOLE, "-c", conf_path, "-u", str(timeout)],
            input=command + "\n", capture_output=True, text=True, timeout=timeout + 5
        )
        return STRIP_RE.sub("", proc.stdout).strip()
    except Exception as e:
        return f"ERROR: {e}"


def start_backup(conf_path: str, job_name: str) -> int:
    """Start a backup job, return JobId."""
    output = bconsole_cmd(conf_path, f"run job={job_name} level=Full yes", timeout=15)
    m = re.search(r"JobId=(\d+)", output)
    return int(m.group(1)) if m else 0


def poll_job_status(conf_path: str, job_id: int) -> dict:
    """Query job status via API mode 1 (tab-separated, same format for both Directors)."""
    # .api 1 → list jobid=N → .api 0  (all in one bconsole session)
    output = bconsole_cmd(conf_path,
        f".api 1\nlist jobid={job_id}\n.api 0", timeout=10)
    result = {"files": 0, "bytes": 0, "status": "R"}

    for line in output.split("\n"):
        if not line.strip() or "api:" in line or "jobid" in line.lower():
            continue
        # API mode 1: tab-separated fields
        fields = line.split("\t")
        if len(fields) < 5:
            # Fallback: whitespace split
            fields = line.split()

        # Find field with job_id
        if not any(f.strip() == str(job_id) for f in fields):
            continue

        # Parse: find jobstatus (single char), jobfiles, jobbytes
        for f in fields:
            f = f.strip().replace(",", "")
            if len(f) == 1 and f in "TREfACBDFSsmMjcdtpialLW":
                result["status"] = f
            elif f.isdigit():
                val = int(f)
                if val == job_id:
                    continue
                elif val < 10_000_000:
                    result["files"] = val  # jobfiles < 10M
                else:
                    result["bytes"] = val  # jobbytes is large

    return result


def poll_sd_status(conf_path: str) -> dict:
    """Poll SD status for live throughput data during backup."""
    output = bconsole_cmd(conf_path, "status storage=File", timeout=8)
    result = {"files": 0, "bytes": 0, "avg_bps": 0, "last_bps": 0, "writing": False}
    for line in output.split("\n"):
        line = line.strip().replace(",", "")
        if "Files=" in line and "Bytes=" in line:
            for part in line.split():
                if part.startswith("Files="):
                    try: result["files"] = int(part.split("=")[1])
                    except: pass
                elif part.startswith("Bytes=") and "AveBytes" not in part:
                    try: result["bytes"] = int(part.split("=")[1])
                    except: pass
                elif part.startswith("AveBytes/sec="):
                    try: result["avg_bps"] = int(part.split("=")[1])
                    except: pass
                elif part.startswith("LastBytes/sec="):
                    try: result["last_bps"] = int(part.split("=")[1])
                    except: pass
            if "Writing" in line:
                result["writing"] = True
    return result


def monitor_backup(conf_path: str, job_id: int, samples: list, stop_event: threading.Event):
    """Poll SD status + job status every 3 seconds."""
    start_time = time.monotonic()

    while not stop_event.is_set():
        elapsed = time.monotonic() - start_time

        # SD throughput data
        sd = poll_sd_status(conf_path)
        samples.append(BackupSample(
            elapsed_s=round(elapsed, 1),
            files=sd["files"],
            bytes=sd["bytes"],
            throughput_mbps=round(sd["last_bps"] / 1024 / 1024, 1) if sd["last_bps"] > 0 else 0,
        ))

        # Job status is the primary termination check
        js = poll_job_status(conf_path, job_id)
        if js["status"] in ("T", "E", "f", "A"):
            # Final sample with job's actual totals
            if js["bytes"] > 0:
                samples.append(BackupSample(
                    elapsed_s=round(time.monotonic() - start_time, 1),
                    files=js["files"], bytes=js["bytes"], throughput_mbps=0,
                ))
            break

        stop_event.wait(3.0)


def run_benchmark(conf_path: str, director_name: str, port: int, job_name: str) -> BackupRun:
    """Run a complete backup benchmark."""
    run = BackupRun(director=director_name, port=port)

    print(f"\n  Starting {director_name} backup (port {port})...")
    start = time.monotonic()

    run.job_id = start_backup(conf_path, job_name)
    if run.job_id == 0:
        run.status = "FAILED_TO_START"
        print(f"  {director_name}: failed to start job")
        return run

    print(f"  {director_name}: JobId={run.job_id}, monitoring...")

    stop = threading.Event()
    monitor = threading.Thread(target=monitor_backup, args=(conf_path, run.job_id, run.samples, stop))
    monitor.start()

    # Wait for job to finish (max 20 min)
    monitor.join(timeout=1200)
    stop.set()

    run.duration_s = round(time.monotonic() - start, 1)

    # Final status
    final = poll_job_status(conf_path, run.job_id)
    run.total_files = final["files"]
    run.total_bytes = final["bytes"]
    run.status = final["status"]
    run.avg_throughput_mbps = round(run.total_bytes / run.duration_s / 1024 / 1024, 2) if run.duration_s > 0 else 0

    print(f"  {director_name}: {run.status} — {run.total_files} files, "
          f"{run.total_bytes/1024/1024:.0f} MB in {run.duration_s}s "
          f"({run.avg_throughput_mbps} MB/s)")

    return run


# =============================================================================
# PDF Report
# =============================================================================

def gather_system_info() -> dict:
    import platform
    info = {"hostname": platform.node(), "os": platform.platform(),
            "kernel": platform.release(), "cpu": "unknown", "ram_gb": "unknown",
            "arch": platform.machine()}
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    info["cpu"] = line.split(":")[1].strip(); break
    except OSError: pass
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemTotal"):
                    info["ram_gb"] = f"{int(line.split()[1])/1024/1024:.1f} GB"; break
    except OSError: pass
    return info


def gather_fileset_stats(paths: list) -> dict:
    """Collect fileset statistics: total files, dirs, bytes."""
    total_files = 0
    total_dirs = 0
    total_bytes = 0
    for p in paths:
        try:
            for root, dirs, files in os.walk(p):
                total_dirs += len(dirs)
                for f in files:
                    fp = os.path.join(root, f)
                    try:
                        total_bytes += os.path.getsize(fp)
                        total_files += 1
                    except OSError:
                        pass
        except OSError:
            pass
    return {
        "paths": paths,
        "files": total_files,
        "dirs": total_dirs,
        "bytes": total_bytes,
        "human_size": f"{total_bytes/1024/1024/1024:.2f} GB",
    }


def generate_pdf(cpp_run: BackupRun, rust_run: BackupRun, path: Path,
                  fileset_stats: dict = None):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.backends.backend_pdf import PdfPages

    sysinfo = gather_system_info()

    with PdfPages(str(path)) as pdf:
        # --- Page 1: Summary ---
        fig, axes = plt.subplots(2, 1, figsize=(11, 8.5), gridspec_kw={"height_ratios": [1, 1.5]})

        # Top: System info + Summary table
        ax = axes[0]
        ax.axis("off")
        ax.text(0.5, 0.95, "Onesimus Director — Backup Throughput Benchmark",
                fontsize=18, ha="center", fontweight="bold", transform=ax.transAxes)

        # System + test conditions
        sys_text = (f"Host: {sysinfo['hostname']}   OS: {sysinfo['os']}\n"
                    f"CPU: {sysinfo['cpu']}   RAM: {sysinfo['ram_gb']}   Arch: {sysinfo.get('arch','')}")
        ax.text(0.5, 0.82, sys_text, fontsize=7, ha="center", color="gray",
                transform=ax.transAxes)

        # FileSet statistics
        if fileset_stats:
            fs_text = (f"FileSet: {', '.join(fileset_stats['paths'])}\n"
                       f"Files: {fileset_stats['files']:,}   "
                       f"Directories: {fileset_stats['dirs']:,}   "
                       f"Total size: {fileset_stats['human_size']}")
            ax.text(0.5, 0.74, fs_text, fontsize=8, ha="center",
                    fontfamily="monospace", transform=ax.transAxes,
                    bbox=dict(boxstyle="round,pad=0.3", facecolor="#E8F5E9", edgecolor="#2E7D32"))

        # Test conditions notice
        ax.text(0.5, 0.67,
                "Test conditions: Both Directors ran sequentially (never parallel) on the same host,\n"
                "same config, same catalog DB, same FD/SD. No other applications were running during the test.",
                fontsize=7, ha="center", color="#555", style="italic", transform=ax.transAxes)

        # Summary table
        col_labels = ["", "C++ Director", "Rust Director", "Delta"]
        cpp_mb = cpp_run.total_bytes / 1024 / 1024
        rust_mb = rust_run.total_bytes / 1024 / 1024
        speedup = (cpp_run.duration_s / rust_run.duration_s) if rust_run.duration_s > 0 else 0

        table_data = [
            ["JobId", str(cpp_run.job_id), str(rust_run.job_id), ""],
            ["Status", cpp_run.status, rust_run.status, ""],
            ["Files", f"{cpp_run.total_files:,}", f"{rust_run.total_files:,}", ""],
            ["Data (MB)", f"{cpp_mb:,.0f}", f"{rust_mb:,.0f}", ""],
            ["Duration (s)", f"{cpp_run.duration_s:.1f}", f"{rust_run.duration_s:.1f}",
             f"{speedup:.1f}x" if speedup > 0 else ""],
            ["Avg Throughput", f"{cpp_run.avg_throughput_mbps:.1f} MB/s",
             f"{rust_run.avg_throughput_mbps:.1f} MB/s",
             f"{speedup:.1f}x" if speedup > 0 else ""],
        ]
        colors = [["#ECEFF1", "#E3F2FD", "#FFF3E0", "#E8F5E9"]] * len(table_data)

        table = ax.table(cellText=table_data, colLabels=col_labels,
                         cellColours=colors, loc="center", cellLoc="center")
        table.auto_set_font_size(False)
        table.set_fontsize(9)
        table.scale(1.0, 1.8)
        for j in range(4):
            table[0, j].set_facecolor("#263238")
            table[0, j].set_text_props(color="white", fontweight="bold")

        # Bottom: Throughput over time
        ax2 = axes[1]

        if cpp_run.samples:
            cpp_t = [s.elapsed_s for s in cpp_run.samples]
            cpp_tp = [s.throughput_mbps for s in cpp_run.samples]
            ax2.plot(cpp_t, cpp_tp, label=f"C++ (avg {cpp_run.avg_throughput_mbps:.1f} MB/s)",
                     color="#2196F3", linewidth=2)
            # Cumulative bytes as area
            cpp_cum = [s.bytes / 1024 / 1024 for s in cpp_run.samples]

        if rust_run.samples:
            rust_t = [s.elapsed_s for s in rust_run.samples]
            rust_tp = [s.throughput_mbps for s in rust_run.samples]
            ax2.plot(rust_t, rust_tp, label=f"Rust (avg {rust_run.avg_throughput_mbps:.1f} MB/s)",
                     color="#FF9800", linewidth=2)

        ax2.set_xlabel("Elapsed Time (seconds)")
        ax2.set_ylabel("Throughput (MB/s) — from SD LastBytes/sec")
        ax2.set_title("Backup Throughput Over Time (SD Device Stats)")
        ax2.legend(loc="upper right")
        ax2.grid(True, alpha=0.3)

        plt.tight_layout()
        pdf.savefig(fig)
        plt.close(fig)

        # --- Page 2: Cumulative data transferred ---
        fig, ax = plt.subplots(figsize=(11, 8.5))

        if cpp_run.samples:
            cpp_t = [s.elapsed_s for s in cpp_run.samples]
            cpp_cum = [s.bytes / 1024 / 1024 for s in cpp_run.samples]
            ax.fill_between(cpp_t, cpp_cum, alpha=0.3, color="#2196F3")
            ax.plot(cpp_t, cpp_cum, label=f"C++ ({cpp_mb:.0f} MB total)",
                    color="#2196F3", linewidth=2)

        if rust_run.samples:
            rust_t = [s.elapsed_s for s in rust_run.samples]
            rust_cum = [s.bytes / 1024 / 1024 for s in rust_run.samples]
            ax.fill_between(rust_t, rust_cum, alpha=0.3, color="#FF9800")
            ax.plot(rust_t, rust_cum, label=f"Rust ({rust_mb:.0f} MB total)",
                    color="#FF9800", linewidth=2)

        ax.set_xlabel("Elapsed Time (seconds)")
        ax.set_ylabel("Cumulative Data Transferred (MB)")
        ax.set_title("Backup Progress: Cumulative Data Over Time")
        ax.legend(loc="upper left")
        ax.grid(True, alpha=0.3)

        plt.tight_layout()
        pdf.savefig(fig)
        plt.close(fig)

    print(f"  PDF report: {path}")


# =============================================================================
# Main
# =============================================================================

def main():
    parser = argparse.ArgumentParser(description="Backup Throughput Benchmark")
    parser.add_argument("--cpp-port", type=int, default=9101)
    parser.add_argument("--rust-port", type=int, default=29101)
    parser.add_argument("--password", default=PASSWORD)
    parser.add_argument("--job", default="benchmark-1gb", help="Job name to run")
    args = parser.parse_args()

    timestamp = time.strftime("%Y%m%d-%H%M%S")
    result_dir = Path(f"tests/results/benchmark_{timestamp}")
    result_dir.mkdir(parents=True, exist_ok=True)

    # bconsole configs
    cpp_conf = tempfile.NamedTemporaryFile(mode="w", suffix=".conf", delete=False)
    rust_conf = tempfile.NamedTemporaryFile(mode="w", suffix=".conf", delete=False)
    for conf, port in [(cpp_conf, args.cpp_port), (rust_conf, args.rust_port)]:
        conf.write(f'Director {{\n  Name = bareos-dir\n  address = localhost\n  Port = {port}\n  Password = "{args.password}"\n}}\n')
        conf.close()

    print(f"\n{'='*70}")
    print(f" Onesimus — Backup Throughput Benchmark ({timestamp})")
    print(f" Job: {args.job}")
    print(f" C++ Director: localhost:{args.cpp_port}")
    print(f" Rust Director: localhost:{args.rust_port}")
    print(f"{'='*70}")

    # Run C++ benchmark
    cpp_run = run_benchmark(cpp_conf.name, "C++", args.cpp_port, args.job)

    # Run Rust benchmark
    rust_run = run_benchmark(rust_conf.name, "Rust", args.rust_port, args.job)

    # Save JSON
    json_path = result_dir / "benchmark.json"
    with open(json_path, "w") as f:
        json.dump({
            "timestamp": timestamp,
            "job": args.job,
            "cpp": {"port": cpp_run.port, "job_id": cpp_run.job_id,
                    "files": cpp_run.total_files, "bytes": cpp_run.total_bytes,
                    "duration_s": cpp_run.duration_s, "throughput_mbps": cpp_run.avg_throughput_mbps,
                    "status": cpp_run.status,
                    "samples": [{"t": s.elapsed_s, "files": s.files, "bytes": s.bytes, "mbps": s.throughput_mbps}
                                for s in cpp_run.samples]},
            "rust": {"port": rust_run.port, "job_id": rust_run.job_id,
                     "files": rust_run.total_files, "bytes": rust_run.total_bytes,
                     "duration_s": rust_run.duration_s, "throughput_mbps": rust_run.avg_throughput_mbps,
                     "status": rust_run.status,
                     "samples": [{"t": s.elapsed_s, "files": s.files, "bytes": s.bytes, "mbps": s.throughput_mbps}
                                 for s in rust_run.samples]},
        }, f, indent=2)

    # FileSet statistics
    fileset_paths = ["/usr/bin", "/usr/sbin", "/usr/share"]  # from Benchmark1GB.conf
    print(f"\n  Scanning FileSet ({', '.join(fileset_paths)})...")
    fileset_stats = gather_fileset_stats(fileset_paths)
    print(f"  FileSet: {fileset_stats['files']:,} files, {fileset_stats['dirs']:,} dirs, "
          f"{fileset_stats['human_size']}")

    # Generate PDF
    generate_pdf(cpp_run, rust_run, result_dir / "benchmark.pdf", fileset_stats)

    # Summary
    speedup = (cpp_run.duration_s / rust_run.duration_s) if rust_run.duration_s > 0 else 0
    print(f"\n{'='*70}")
    print(f" C++:  {cpp_run.total_files} files, {cpp_run.total_bytes/1024/1024:.0f} MB, "
          f"{cpp_run.duration_s}s, {cpp_run.avg_throughput_mbps:.1f} MB/s")
    print(f" Rust: {rust_run.total_files} files, {rust_run.total_bytes/1024/1024:.0f} MB, "
          f"{rust_run.duration_s}s, {rust_run.avg_throughput_mbps:.1f} MB/s")
    if speedup > 0:
        print(f" Speedup: {speedup:.1f}x")
    print(f" Report: {result_dir}/benchmark.pdf")
    print(f"{'='*70}\n")

    os.unlink(cpp_conf.name)
    os.unlink(rust_conf.name)


if __name__ == "__main__":
    main()
