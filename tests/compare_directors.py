#!/usr/bin/env python3
"""
Onesimus Director — Quantitative Comparison Test Suite

Runs identical commands against C++ bareos-dir and Rust odir,
compares outputs, measures response times, produces JSON + PDF report.

Usage:
    python3 tests/compare_directors.py [--cpp-port 9101] [--rust-port 29101]
    # Auto-creates .venv, installs matplotlib if needed.
"""

import os
import subprocess
import sys
from pathlib import Path

# =============================================================================
# Bootstrap: ensure venv with matplotlib exists, re-exec if needed
# =============================================================================

VENV_DIR = Path(__file__).parent.parent / ".venv"
VENV_PYTHON = VENV_DIR / "bin" / "python3"
REQUIREMENTS = ["matplotlib"]

def ensure_venv():
    """Create venv and install dependencies if missing, then re-exec."""
    if sys.prefix == str(VENV_DIR):
        return  # Already inside our venv

    if not VENV_PYTHON.exists():
        print(f"Creating venv at {VENV_DIR} ...")
        try:
            subprocess.check_call([sys.executable, "-m", "venv", str(VENV_DIR)])
        except subprocess.CalledProcessError:
            # venv module missing — try to install it
            # Detect distro and install
            if Path("/etc/debian_version").exists():
                pkg = f"python3-venv"  # Debian/Ubuntu
                print(f"venv not available. Install with: sudo apt install {pkg}")
            elif Path("/etc/redhat-release").exists():
                print("venv not available. Install with: sudo dnf install python3-devel")
            elif Path("/etc/arch-release").exists():
                print("venv not available. Install with: sudo pacman -S python")
            elif Path("/etc/SUSE-brand").exists() or Path("/etc/SuSE-release").exists():
                print("venv not available. Install with: sudo zypper install python3-devel")
            else:
                print("venv not available. Install python3-venv for your distribution.")
            sys.exit(1)

    # Install missing packages
    for pkg in REQUIREMENTS:
        try:
            subprocess.check_call(
                [str(VENV_PYTHON), "-c", f"import {pkg}"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
            )
        except subprocess.CalledProcessError:
            print(f"Installing {pkg} into venv ...")
            subprocess.check_call(
                [str(VENV_DIR / "bin" / "pip"), "install", "-q", pkg]
            )

    # Re-exec this script inside the venv
    os.execv(str(VENV_PYTHON), [str(VENV_PYTHON), __file__] + sys.argv[1:])

ensure_venv()

# --- From here on, we're inside the venv with all deps ---

import argparse
import json
import re
import tempfile
import time
from dataclasses import dataclass, field, asdict

# =============================================================================
# Config
# =============================================================================

BCONSOLE = os.environ.get("BCONSOLE", "/usr/bin/bconsole")
PASSWORD = os.environ.get("PASSWORD", "SlC3mJTFz3DnQX9ZEJwWJgAIoxe5GbD/OmyQADlFrN9A")
TIMEOUT = int(os.environ.get("TIMEOUT", "10"))

# =============================================================================
# Data types
# =============================================================================

@dataclass
class TestResult:
    name: str
    command: str
    mode: str
    match: str          # "yes", "no", "partial", "structure_diff", "skip"
    cpp_ms: int = 0
    rust_ms: int = 0
    cpp_lines: int = 0
    rust_lines: int = 0
    diff: str = ""

@dataclass
class Report:
    timestamp: str
    cpp_port: int
    rust_port: int
    tests: list = field(default_factory=list)
    total: int = 0
    passed: int = 0
    failed: int = 0
    skipped: int = 0

# =============================================================================
# Runner
# =============================================================================

# Strip bconsole chrome
STRIP_RE = re.compile(
    r"^(Connecting to|Enter a period|You are connected|"
    r" Encryption:|1000 OK:).*$", re.MULTILINE
)

def run_cmd(conf_path: str, command: str) -> tuple[str, int]:
    """Send command to bconsole, return (output, ms)."""
    start = time.monotonic()
    try:
        proc = subprocess.run(
            [BCONSOLE, "-c", conf_path, "-u", str(TIMEOUT)],
            input=command + "\n",
            capture_output=True, text=True, timeout=TIMEOUT + 5
        )
        output = proc.stdout
    except (subprocess.TimeoutExpired, FileNotFoundError) as e:
        return (f"ERROR: {e}", 0)
    elapsed_ms = int((time.monotonic() - start) * 1000)

    # Strip bconsole chrome
    output = STRIP_RE.sub("", output).strip()
    # Remove the echoed command line
    lines = output.split("\n")
    cleaned = []
    for line in lines:
        stripped = line.strip()
        if stripped == command.strip():
            continue
        if stripped:
            cleaned.append(line)
    return ("\n".join(cleaned), elapsed_ms)


# Normalize for comparison
NORM_RE = [
    (re.compile(r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}"), "TIMESTAMP"),
    (re.compile(r"JobId=\d+"), "JobId=N"),
    (re.compile(r"Version: [0-9]+\.[0-9]+\.[0-9]+\S*"), "Version: X.Y.Z"),
    (re.compile(r"\d+ (files|bytes|jobs|errors)"), "N \\1"),
    (re.compile(r"\x1b\[[0-9;]*m"), ""),  # strip ANSI
    (re.compile(r"\s+$", re.MULTILINE), ""),
]

def normalize(text: str) -> str:
    for pattern, repl in NORM_RE:
        text = pattern.sub(repl, text)
    lines = [l for l in text.split("\n") if l.strip()]
    return "\n".join(sorted(lines))


def compare(cpp: str, rust: str, mode: str) -> tuple[str, str]:
    """Compare outputs. Returns (match_status, diff_text)."""
    if mode == "exact":
        cpp_n = normalize(cpp)
        rust_n = normalize(rust)
        if cpp_n == rust_n:
            return ("yes", "")
        return ("no", make_diff(cpp, rust))

    elif mode == "contains":
        cpp_n = normalize(cpp)
        rust_n = normalize(rust)
        # Every non-empty normalized C++ line should appear in Rust
        cpp_lines = [l for l in cpp_n.split("\n") if l.strip()]
        missing = [l for l in cpp_lines if l not in rust_n]
        if not missing:
            return ("yes", "")
        return ("partial", make_diff(cpp, rust))

    elif mode == "structure":
        cpp_count = len([l for l in cpp.split("\n") if l.strip()])
        rust_count = len([l for l in rust.split("\n") if l.strip()])
        if cpp_count == rust_count:
            return ("yes", "")
        diff = f"C++ lines: {cpp_count}, Rust lines: {rust_count}\n"
        diff += make_diff(cpp, rust)
        return ("structure_diff", diff)

    return ("yes", "")


def make_diff(a: str, b: str) -> str:
    """Simple line diff."""
    a_lines = a.split("\n")
    b_lines = b.split("\n")
    diff = []
    for i, (al, bl) in enumerate(zip(a_lines, b_lines)):
        if al != bl:
            diff.append(f"  line {i+1}:")
            diff.append(f"    C++:  {al}")
            diff.append(f"    Rust: {bl}")
    if len(a_lines) != len(b_lines):
        diff.append(f"  C++ has {len(a_lines)} lines, Rust has {len(b_lines)} lines")
    return "\n".join(diff[:30])  # limit


# =============================================================================
# Test definitions
# =============================================================================

TESTS = [
    # (name, command, mode)
    # Basic
    ("version",         "version",              "contains"),
    ("time",            "time",                 "contains"),
    ("help",            "help",                 "structure"),
    (".help_all",       ".help all",            "structure"),

    # Status
    ("status_dir",      "status dir",           "structure"),
    ("status_client",   "status client=bareos-fd", "contains"),
    ("status_storage",  "status storage=File",  "contains"),

    # List
    ("list_jobs",       "list jobs",            "structure"),
    ("list_jobs_5",     "list jobs limit=5",    "structure"),
    ("list_clients",    "list clients",         "structure"),
    ("list_pools",      "list pools",           "structure"),
    ("list_volumes",    "list volumes",         "structure"),
    ("list_storages",   "list storages",        "structure"),
    ("list_filesets",   "list filesets",         "structure"),
    ("list_jobtotals",  "list jobtotals",       "structure"),

    # Dot Commands
    (".clients",        ".clients",             "exact"),
    (".jobs",           ".jobs",                "exact"),
    (".pools",          ".pools",               "exact"),
    (".storage",        ".storage",             "exact"),
    (".filesets",       ".filesets",            "exact"),
    (".schedule",       ".schedule",            "exact"),
    (".levels",         ".levels",              "exact"),
    (".types",          ".types",               "exact"),
    (".volstatus",      ".volstatus",           "exact"),
    (".media",          ".media",               "exact"),
    (".catalogs",       ".catalogs",            "exact"),
    (".jobdefs",        ".jobdefs",             "exact"),
    (".mediatypes",     ".mediatypes",          "exact"),
    (".actiononpurge",  ".actiononpurge",       "exact"),
    (".jobstatus",      ".jobstatus",           "exact"),

    # SQL
    ("sql_count",       "sqlquery SELECT COUNT(*) FROM job", "contains"),
    ("sql_clients",     "sqlquery SELECT name FROM client ORDER BY name", "exact"),
    (".sql_tables",     ".sql tables",          "structure"),

    # Show
    ("show_director",   "show director",        "contains"),
    ("show_clients",    "show clients",         "structure"),
    ("show_jobs",       "show jobs",            "structure"),
    ("show_pools",      "show pools",           "structure"),

    # Session
    ("whoami",          "whoami",               "exact"),
    ("messages",        "messages",             "contains"),

    # Admin
    ("configure_export", "configure export",    "structure"),
    ("setdebug_0",      "setdebug level=0",    "contains"),
]

# =============================================================================
# Colors
# =============================================================================

GREEN  = "\033[32m"
RED    = "\033[31m"
YELLOW = "\033[33m"
CYAN   = "\033[36m"
NC     = "\033[0m"

# =============================================================================
# Main
# =============================================================================

def main():
    parser = argparse.ArgumentParser(description="Compare C++ and Rust Directors")
    parser.add_argument("--cpp-port", type=int, default=9101)
    parser.add_argument("--rust-port", type=int, default=29101)
    parser.add_argument("--password", default=PASSWORD)
    parser.add_argument("--json", action="store_true", help="JSON output only")
    args = parser.parse_args()

    timestamp = time.strftime("%Y%m%d-%H%M%S")
    result_dir = Path(f"tests/results/compare_{timestamp}")
    diff_dir = result_dir / "diffs"
    diff_dir.mkdir(parents=True, exist_ok=True)

    # Create bconsole configs
    cpp_conf = tempfile.NamedTemporaryFile(mode="w", suffix=".conf", delete=False)
    rust_conf = tempfile.NamedTemporaryFile(mode="w", suffix=".conf", delete=False)
    for conf, port in [(cpp_conf, args.cpp_port), (rust_conf, args.rust_port)]:
        conf.write(f'Director {{\n  Name = bareos-dir\n  address = localhost\n  Port = {port}\n  Password = "{args.password}"\n}}\n')
        conf.close()

    report = Report(timestamp=timestamp, cpp_port=args.cpp_port, rust_port=args.rust_port)

    if not args.json:
        print(f"\n{'='*73}")
        print(f" Onesimus Director — Quantitative Comparison ({timestamp})")
        print(f" C++ Director: localhost:{args.cpp_port}")
        print(f" Rust Director: localhost:{args.rust_port}")
        print(f"{'='*73}\n")

    current_section = ""
    for name, command, mode in TESTS:
        # Section headers
        section = name.split("_")[0] if "_" in name else name[0:4]
        if section != current_section and not args.json:
            current_section = section
            print(f"\n{CYAN}--- {section} ---{NC}")

        report.total += 1

        cpp_out, cpp_ms = run_cmd(cpp_conf.name, command)
        rust_out, rust_ms = run_cmd(rust_conf.name, command)

        match_status, diff_text = compare(cpp_out, rust_out, mode)

        result = TestResult(
            name=name, command=command, mode=mode, match=match_status,
            cpp_ms=cpp_ms, rust_ms=rust_ms,
            cpp_lines=len(cpp_out.split("\n")),
            rust_lines=len(rust_out.split("\n")),
        )

        if match_status == "yes":
            report.passed += 1
            if not args.json:
                print(f"{GREEN}PASS{NC} {name:<40s} cpp={cpp_ms:4d}ms rust={rust_ms:4d}ms  {GREEN}match{NC}")
        elif match_status == "skip":
            report.skipped += 1
            if not args.json:
                print(f"{YELLOW}SKIP{NC} {name:<40s}")
        else:
            report.failed += 1
            result.diff = diff_text
            # Save diff
            diff_path = diff_dir / f"{name}.diff"
            diff_path.write_text(f"--- C++ (port {args.cpp_port})\n+++ Rust (port {args.rust_port})\n\n{diff_text}\n")
            if not args.json:
                print(f"{RED}FAIL{NC} {name:<40s} cpp={cpp_ms:4d}ms rust={rust_ms:4d}ms  {RED}{match_status}{NC}")

        report.tests.append(asdict(result))

    # Summary
    if not args.json:
        print(f"\n{'='*73}")
        print(f" {GREEN}{report.passed} PASS{NC}  {RED}{report.failed} FAIL{NC}  {YELLOW}{report.skipped} SKIP{NC}  ({report.total} total)")
        avg_cpp = sum(t["cpp_ms"] for t in report.tests) // max(len(report.tests), 1)
        avg_rust = sum(t["rust_ms"] for t in report.tests) // max(len(report.tests), 1)
        if avg_rust > 0:
            speedup = avg_cpp / avg_rust
            print(f" Avg response: C++={avg_cpp}ms  Rust={avg_rust}ms  ({speedup:.1f}x faster)")
        print(f" Report: {result_dir}/report.json")
        print(f" Diffs:  {diff_dir}/")
        print(f"{'='*73}\n")

    # Write JSON report
    json_path = result_dir / "report.json"
    with open(json_path, "w") as f:
        json.dump({
            "timestamp": report.timestamp,
            "cpp_port": report.cpp_port,
            "rust_port": report.rust_port,
            "summary": {
                "total": report.total,
                "pass": report.passed,
                "fail": report.failed,
                "skip": report.skipped,
            },
            "tests": report.tests,
        }, f, indent=2)

    if args.json:
        with open(json_path) as f:
            print(f.read())

    # Generate PDF report
    pdf_path = result_dir / "report.pdf"
    generate_pdf(report, pdf_path)
    if not args.json:
        print(f" PDF:    {pdf_path}")
        print()

    # Cleanup
    os.unlink(cpp_conf.name)
    os.unlink(rust_conf.name)

    sys.exit(0 if report.failed == 0 else 1)


# =============================================================================
# PDF Report Generator
# =============================================================================

def gather_system_info() -> dict:
    """Collect system information for the report."""
    import platform
    info = {
        "hostname": platform.node(),
        "os": platform.platform(),
        "kernel": platform.release(),
        "arch": platform.machine(),
        "python": platform.python_version(),
        "cpu": "unknown",
        "ram_gb": "unknown",
        "cpp_version": "unknown",
        "rust_version": "unknown",
    }
    # CPU
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    info["cpu"] = line.split(":")[1].strip()
                    break
    except OSError:
        pass
    # RAM
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemTotal"):
                    kb = int(line.split()[1])
                    info["ram_gb"] = f"{kb / 1024 / 1024:.1f} GB"
                    break
    except OSError:
        pass
    # Bareos C++ version
    try:
        out = subprocess.check_output(["/usr/sbin/bareos-dir", "--version"],
                                       stderr=subprocess.STDOUT, text=True, timeout=3)
        for line in out.split("\n"):
            if "Version:" in line:
                info["cpp_version"] = line.strip()
                break
    except Exception:
        pass
    # Rust odir version
    try:
        out = subprocess.check_output(["target/debug/odir", "--version"],
                                       stderr=subprocess.STDOUT, text=True, timeout=3)
        info["rust_version"] = out.strip()
    except Exception:
        pass
    return info


def classify_failure(name: str, diff: str) -> str:
    """Explain WHY a test failed — config diff, format diff, or real bug."""
    if not diff:
        return ""
    lower = diff.lower()
    if "port" in lower and ("9101" in diff or "29101" in diff):
        return "Port difference (C++ on 9101, Rust on 29101) — expected when running side-by-side."
    if "bareos-repo" in lower or "baReosRepo" in lower:
        return "Extra resource in Rust config (baReosRepo test job/fileset not in C++ config)."
    if "version" in lower:
        return "Version string difference — different binaries, expected."
    if "lines:" in lower:
        return "Output format difference — Rust shows more/fewer detail lines than C++."
    if "Job {" in diff or "JobDefs" in diff:
        return "Rust show jobs uses multi-line Bareos config format, C++ uses single-line summary."
    return "Output content mismatch — check diff for details."


def generate_pdf(report: Report, path: Path):
    """Generate a PDF report with system info, summary, timing chart, detail table."""
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.backends.backend_pdf import PdfPages

    tests = report.tests
    sysinfo = gather_system_info()

    with PdfPages(str(path)) as pdf:
        # --- Page 1: System Info ---
        fig, ax = plt.subplots(figsize=(11, 8.5))
        ax.axis("off")

        ax.text(0.5, 0.96, "Onesimus Director — Comparison Report",
                fontsize=22, ha="center", fontweight="bold")
        ax.text(0.5, 0.93, f"Date: {report.timestamp}",
                fontsize=11, ha="center", color="gray")

        # System info — left column
        sys_text = (
            f"Host:     {sysinfo['hostname']}\n"
            f"OS:       {sysinfo['os']}\n"
            f"Kernel:   {sysinfo['kernel']}\n"
            f"CPU:      {sysinfo['cpu']}\n"
            f"RAM:      {sysinfo['ram_gb']}\n"
            f"Python:   {sysinfo['python']}"
        )
        ax.text(0.03, 0.88, sys_text, fontsize=8, va="top",
                fontfamily="monospace",
                bbox=dict(boxstyle="round,pad=0.4", facecolor="#E3F2FD", edgecolor="#1565C0"))

        # Directors — right column
        dir_text = (
            f"C++ Director:\n"
            f"  Port: {report.cpp_port}\n"
            f"  {sysinfo['cpp_version']}\n\n"
            f"Rust Director:\n"
            f"  Port: {report.rust_port}\n"
            f"  {sysinfo['rust_version']}"
        )
        ax.text(0.55, 0.88, dir_text, fontsize=8, va="top",
                fontfamily="monospace",
                bbox=dict(boxstyle="round,pad=0.4", facecolor="#FFF3E0", edgecolor="#E65100"))

        # Summary — centered below system info
        total = report.total
        passed = report.passed
        failed = report.failed
        pct = (passed / total * 100) if total else 0
        avg_cpp = sum(t["cpp_ms"] for t in tests) // max(len(tests), 1)
        avg_rust = sum(t["rust_ms"] for t in tests) // max(len(tests), 1)
        speedup = (avg_cpp / avg_rust) if avg_rust > 0 else 0

        summary_text = (
            f"Total: {total}    "
            f"Passed: {passed} ({pct:.0f}%)    "
            f"Failed: {failed}    "
            f"Skipped: {report.skipped}\n"
            f"Avg Response:  C++ = {avg_cpp} ms    Rust = {avg_rust} ms    "
            f"Speedup: {speedup:.1f}x"
        )
        ax.text(0.5, 0.58, summary_text, fontsize=12, ha="center", va="center",
                fontfamily="monospace",
                bbox=dict(boxstyle="round,pad=0.6", facecolor="#f0f0f0", edgecolor="gray"))

        # Test method
        ax.text(0.5, 0.49, "Method: bconsole pipes command → stdout captured → "
                "ANSI stripped → timestamps/versions/IDs normalized → sorted line comparison",
                fontsize=8, ha="center", color="gray", style="italic")

        # Pass/Fail donut — lower half, centered
        ax_donut = fig.add_axes([0.15, 0.05, 0.3, 0.38])
        colors = ["#4CAF50", "#F44336", "#FFC107"]
        sizes = [passed, failed, report.skipped]
        labels = [f"PASS ({passed})", f"FAIL ({failed})", f"SKIP ({report.skipped})"]
        wedges, texts, autotexts = ax_donut.pie(
            [s for s in sizes if s > 0],
            labels=[l for l, s in zip(labels, sizes) if s > 0],
            colors=[c for c, s in zip(colors, sizes) if s > 0],
            autopct="%1.0f%%", startangle=90, pctdistance=0.75
        )
        centre_circle = plt.Circle((0, 0), 0.50, fc="white")
        ax_donut.add_artist(centre_circle)

        # Failure reasons — right of donut
        fail_tests = [t for t in tests if t["match"] != "yes" and t["match"] != "skip"]
        if fail_tests:
            reasons = []
            for t in fail_tests:
                comment = classify_failure(t["name"], t.get("diff", ""))
                reasons.append(f"  {t['name']}: {comment[:55]}")
            reason_text = "Failure Analysis:\n" + "\n".join(reasons[:8])
            ax.text(0.55, 0.35, reason_text, fontsize=7, va="top",
                    fontfamily="monospace",
                    bbox=dict(boxstyle="round,pad=0.4", facecolor="#FFEBEE", edgecolor="#C62828"))

        pdf.savefig(fig)
        plt.close(fig)

        # --- Page 2: Timing Comparison Chart ---
        fig, ax = plt.subplots(figsize=(11, 8.5))

        names = [t["name"] for t in tests if t["match"] != "skip"]
        cpp_times = [t["cpp_ms"] for t in tests if t["match"] != "skip"]
        rust_times = [t["rust_ms"] for t in tests if t["match"] != "skip"]

        y = range(len(names))
        bar_h = 0.35
        bars_cpp = ax.barh([i + bar_h for i in y], cpp_times, bar_h,
                           label=f"C++ (avg {avg_cpp}ms)", color="#2196F3", alpha=0.8)
        bars_rust = ax.barh(list(y), rust_times, bar_h,
                            label=f"Rust (avg {avg_rust}ms)", color="#FF9800", alpha=0.8)

        # Value labels on bars
        for bar, val in zip(bars_cpp, cpp_times):
            if val > 0:
                ax.text(bar.get_width() + 2, bar.get_y() + bar.get_height()/2,
                        f"{val}ms", va="center", fontsize=6, color="#1565C0")
        for bar, val in zip(bars_rust, rust_times):
            if val > 0:
                ax.text(bar.get_width() + 2, bar.get_y() + bar.get_height()/2,
                        f"{val}ms", va="center", fontsize=6, color="#E65100")

        ax.set_yticks([i + bar_h/2 for i in y])
        ax.set_yticklabels(names, fontsize=7)
        ax.set_xlabel("Response Time (ms)")
        ax.set_title(f"Command Response Time: C++ vs Rust  (Speedup: {speedup:.1f}x)")
        ax.legend(loc="lower right")
        ax.invert_yaxis()
        plt.tight_layout()

        pdf.savefig(fig)
        plt.close(fig)

        # --- Page 3+: Results Table with Comments ---
        col_labels = ["Test", "Match", "C++", "Rust", "Comment"]
        rows_per_page = 30
        all_rows = []
        all_colors = []
        for t in tests:
            comment = ""
            if t["match"] != "yes":
                comment = classify_failure(t["name"], t.get("diff", ""))
            row = [t["name"], t["match"],
                   f"{t['cpp_ms']}ms", f"{t['rust_ms']}ms",
                   comment[:60]]
            all_rows.append(row)
            if t["match"] == "yes":
                all_colors.append(["#E8F5E9"] * 5)
            elif t["match"] == "skip":
                all_colors.append(["#FFF9C4"] * 5)
            else:
                all_colors.append(["#FFEBEE"] * 5)

        # Paginate
        for page_start in range(0, len(all_rows), rows_per_page):
            page_rows = all_rows[page_start:page_start + rows_per_page]
            page_colors = all_colors[page_start:page_start + rows_per_page]

            fig, ax = plt.subplots(figsize=(11, 8.5))
            ax.axis("off")
            page_num = page_start // rows_per_page + 1
            ax.set_title(f"Test Results Detail (Page {page_num})",
                         fontsize=14, fontweight="bold", pad=20)

            table = ax.table(cellText=page_rows, colLabels=col_labels,
                             cellColours=page_colors, loc="center", cellLoc="left")
            table.auto_set_font_size(False)
            table.set_fontsize(7)
            table.scale(1.0, 1.2)

            # Column widths
            for j, w in enumerate([0.15, 0.08, 0.06, 0.06, 0.45]):
                for i in range(len(page_rows) + 1):
                    table[i, j].set_width(w)

            # Header
            for j in range(len(col_labels)):
                table[0, j].set_facecolor("#263238")
                table[0, j].set_text_props(color="white", fontweight="bold")

            plt.tight_layout()
            pdf.savefig(fig)
            plt.close(fig)


if __name__ == "__main__":
    main()
