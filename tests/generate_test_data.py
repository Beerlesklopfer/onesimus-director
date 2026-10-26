#!/usr/bin/env python3
"""
Generiert 10 Millionen Mockup-Dateien die ein Home-Verzeichnis simulieren.

Ziel: ~200GB Rohdaten, ~10% Kompressionsrate (→ ~20GB komprimiert).

Verzeichnisstruktur:
  /home/testuser/
    ├── Documents/         (Office-artige Dateien, gut komprimierbar)
    ├── Pictures/          (JPEG-Header + Random, schlecht komprimierbar)
    ├── Music/             (Pseudo-Audio, schlecht komprimierbar)
    ├── Videos/            (Pseudo-Video, schlecht komprimierbar)
    ├── Downloads/         (Mix aus allem)
    ├── Projects/          (Source-Code, sehr gut komprimierbar)
    ├── .config/           (kleine Config-Dateien)
    ├── .cache/            (viele kleine Dateien)
    ├── .local/            (App-Daten)
    └── Mail/              (Text, gut komprimierbar)

Kompressionsrate ~10%:
  - 30% der Dateien: gut komprimierbar (Text, Code, Configs) → ~5:1 Ratio
  - 70% der Dateien: schlecht komprimierbar (Random/Binary) → ~1.05:1 Ratio
  - Gewichteter Durchschnitt: ~10% Einsparung
"""

import os
import sys
import random
import string
import struct
import hashlib
import time
import multiprocessing
from pathlib import Path
from collections import defaultdict

# ============================================================================
# Konfiguration
# ============================================================================

TARGET_FILES = 10_000_000       # 10 Millionen Dateien
TARGET_SIZE_GB = 200            # ~200 GB Gesamtgroesse
BASE_DIR = "/home/INTRANET.BERNAU.FAMILY/Joerg.Bernau/testdata"
NUM_WORKERS = os.cpu_count() or 8

# Dateigroessen-Verteilung (realistische Home-Verzeichnis Simulation)
SIZE_DISTRIBUTION = [
    # (min_bytes, max_bytes, weight, compressible, category)
    (64,         512,        25,  True,  "config"),      # .config, dotfiles
    (512,        4096,       20,  True,  "cache"),        # .cache, kleine Dateien
    (1024,       8192,       15,  True,  "source"),       # Source Code
    (4096,       65536,      10,  True,  "document"),     # Dokumente, Mail
    (8192,       131072,      8,  True,  "text"),         # groessere Textdateien
    (1024,       16384,       5,  False, "thumbnail"),    # Thumbnails
    (65536,      524288,      5,  False, "image"),        # Bilder (JPEG)
    (131072,     1048576,     4,  False, "audio_small"),  # kleine Audio
    (524288,     4194304,     3,  False, "download"),     # Downloads
    (1048576,    8388608,     2,  False, "audio"),        # Audio
    (4194304,    33554432,    2,  False, "video_small"),  # kleine Videos
    (33554432,   134217728,   1,  False, "video"),        # Videos
]

# Verzeichnisstruktur
DIR_STRUCTURE = {
    "Documents": {
        "Work": ["Reports", "Presentations", "Spreadsheets", "Letters"],
        "Personal": ["Finance", "Insurance", "Taxes", "Receipts"],
        "Templates": [],
    },
    "Pictures": {
        "2024": ["January", "February", "March", "April", "May", "June",
                 "July", "August", "September", "October", "November", "December"],
        "2025": ["January", "February", "March", "April", "May", "June",
                 "July", "August", "September", "October", "November", "December"],
        "Wallpapers": [],
        "Screenshots": [],
    },
    "Music": {
        "Artist_01": ["Album_01", "Album_02"],
        "Artist_02": ["Album_01", "Album_02", "Album_03"],
        "Playlists": [],
    },
    "Videos": {
        "Movies": [],
        "Series": ["Season_01", "Season_02"],
        "Recordings": [],
    },
    "Downloads": {},
    "Projects": {
        f"project_{i:03d}": ["src", "tests", "docs", "build"]
        for i in range(50)
    },
    ".config": {
        app: [] for app in [
            "autostart", "dconf", "gtk-3.0", "pulse", "systemd",
            "chromium", "firefox", "libreoffice", "Code", "vlc",
        ]
    },
    ".cache": {
        app: [] for app in [
            "chromium", "mozilla", "thumbnails", "pip", "yarn",
            "fontconfig", "mesa_shader_cache", "gstreamer-1.0",
        ]
    },
    ".local": {
        "share": ["applications", "icons", "fonts", "Trash", "recently-used"],
        "lib": ["python3.13", "node_modules"],
        "state": [],
    },
    "Mail": {
        box: ["cur", "new", "tmp"]
        for box in ["INBOX", "Sent", "Drafts", "Archive", "Spam", "Trash"]
    },
}

# Dateiendungen pro Kategorie
EXTENSIONS = {
    "config":    [".conf", ".cfg", ".ini", ".yaml", ".yml", ".json", ".toml", ".xml", ".env", ""],
    "cache":     [".cache", ".tmp", ".dat", ".bin", ".db", ".journal", ""],
    "source":    [".py", ".rs", ".c", ".h", ".cpp", ".js", ".ts", ".go", ".java", ".sh", ".md", ".txt", ".html", ".css"],
    "document":  [".txt", ".md", ".csv", ".log", ".tex", ".rst", ".org"],
    "text":      [".txt", ".log", ".md", ".rst"],
    "thumbnail": [".png", ".jpg", ".webp"],
    "image":     [".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp"],
    "audio_small": [".mp3", ".ogg", ".flac"],
    "download":  [".zip", ".tar.gz", ".deb", ".pdf", ".iso"],
    "audio":     [".mp3", ".flac", ".ogg", ".m4a", ".wav"],
    "video_small": [".mp4", ".webm", ".mkv"],
    "video":     [".mp4", ".mkv", ".avi"],
}

# ============================================================================
# Content-Generatoren
# ============================================================================

# Vorgefertigte komprimierbare Bloecke (werden wiederverwendet → gut komprimierbar)
LOREM = (
    "Lorem ipsum dolor sit amet, consectetur adipiscing elit. "
    "Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. "
    "Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris. "
    "Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore. "
    "Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia. "
) * 10  # ~2KB Block

CODE_BLOCK = """
import os
import sys
from pathlib import Path

def process_data(input_file: str, output_dir: str) -> int:
    \"\"\"Process input data and write results to output directory.\"\"\"
    count = 0
    with open(input_file, 'r') as f:
        for line in f:
            if line.strip():
                result = transform(line)
                write_output(output_dir, count, result)
                count += 1
    return count

def transform(data: str) -> str:
    return data.upper().strip()

def write_output(directory: str, index: int, data: str):
    path = Path(directory) / f"output_{index:06d}.txt"
    path.write_text(data)

if __name__ == "__main__":
    process_data(sys.argv[1], sys.argv[2])
""" * 5  # ~2.5KB Block

CONFIG_BLOCK = """
[database]
host = localhost
port = 5432
name = bareos
user = bareos
password = secret
max_connections = 10
timeout = 30

[director]
name = bareos-dir
port = 9101
maximum_concurrent_jobs = 10
heartbeat_interval = 0

[logging]
level = info
file = /var/log/bareos/bareos-dir.log
max_size = 10M
rotate = 7
""" * 8  # ~2KB Block

COMPRESSIBLE_BLOCKS = [LOREM.encode(), CODE_BLOCK.encode(), CONFIG_BLOCK.encode()]


def generate_compressible(size: int) -> bytes:
    """Generiert gut komprimierbaren Content (repetitiv, Text-basiert)."""
    block = random.choice(COMPRESSIBLE_BLOCKS)
    result = bytearray()
    while len(result) < size:
        result.extend(block)
    return bytes(result[:size])


def generate_incompressible(size: int) -> bytes:
    """Generiert schlecht komprimierbaren Content (pseudo-random)."""
    # Verwende einen schnellen PRNG statt os.urandom (schneller)
    seed = random.randint(0, 2**32)
    chunks = []
    remaining = size
    while remaining > 0:
        # hashlib ist schneller als os.urandom fuer grosse Bloecke
        h = hashlib.sha256(struct.pack("<QQ", seed, remaining))
        chunk = h.digest()
        chunks.append(chunk[:min(32, remaining)])
        remaining -= 32
        seed += 1
    return b"".join(chunks)[:size]


# ============================================================================
# Verzeichnis-Erstellung
# ============================================================================

def create_directory_tree(base: str) -> list:
    """Erstellt Verzeichnisbaum und gibt Liste aller Verzeichnisse zurueck."""
    dirs = []

    def walk(parent: str, structure: dict):
        for name, children in structure.items():
            path = os.path.join(parent, name)
            os.makedirs(path, exist_ok=True)
            dirs.append(path)
            if isinstance(children, dict):
                walk(path, children)
            elif isinstance(children, list):
                for child in children:
                    child_path = os.path.join(path, child)
                    os.makedirs(child_path, exist_ok=True)
                    dirs.append(child_path)

    walk(base, DIR_STRUCTURE)
    return dirs


# ============================================================================
# Worker-Prozess
# ============================================================================

def pick_size_and_category() -> tuple:
    """Waehlt Dateigroesse und Kategorie basierend auf Gewichtung."""
    total_weight = sum(w for _, _, w, _, _ in SIZE_DISTRIBUTION)
    r = random.randint(1, total_weight)
    cumulative = 0
    for min_sz, max_sz, weight, compressible, category in SIZE_DISTRIBUTION:
        cumulative += weight
        if r <= cumulative:
            size = random.randint(min_sz, max_sz)
            return size, compressible, category
    # Fallback
    return 1024, True, "config"


def worker(args):
    """Worker-Prozess: erstellt einen Batch von Dateien."""
    worker_id, start_idx, count, dirs = args
    random.seed(worker_id * 1000000 + start_idx)

    stats = defaultdict(lambda: {"count": 0, "bytes": 0})
    errors = 0

    for i in range(count):
        file_idx = start_idx + i
        size, compressible, category = pick_size_and_category()

        # Verzeichnis waehlen
        target_dir = random.choice(dirs)

        # Dateiname
        ext = random.choice(EXTENSIONS.get(category, [""]))
        filename = f"file_{file_idx:08d}{ext}"
        filepath = os.path.join(target_dir, filename)

        try:
            # Content generieren
            if compressible:
                content = generate_compressible(size)
            else:
                content = generate_incompressible(size)

            # Schreiben
            with open(filepath, "wb") as f:
                f.write(content)

            stats[category]["count"] += 1
            stats[category]["bytes"] += size

        except Exception as e:
            errors += 1
            if errors < 5:
                print(f"  Worker {worker_id}: Error writing {filepath}: {e}", file=sys.stderr)

    return dict(stats), errors


# ============================================================================
# Hauptprogramm
# ============================================================================

def main():
    print(f"=== baReos Test-Daten Generator ===")
    print(f"Ziel: {TARGET_FILES:,} Dateien, ~{TARGET_SIZE_GB}GB")
    print(f"Basis: {BASE_DIR}")
    print(f"Worker: {NUM_WORKERS}")
    print()

    # Pruefen ob genug Platz
    stat = os.statvfs(os.path.dirname(BASE_DIR))
    free_gb = (stat.f_bavail * stat.f_bsize) / (1024**3)
    print(f"Verfuegbarer Platz: {free_gb:.1f} GB")
    if free_gb < TARGET_SIZE_GB * 1.1:
        print(f"WARNUNG: Nur {free_gb:.1f}GB frei, brauche ~{TARGET_SIZE_GB}GB")
        resp = input("Fortfahren? (y/n) ")
        if resp.lower() != "y":
            sys.exit(1)

    # Verzeichnisbaum erstellen
    print("Erstelle Verzeichnisstruktur...")
    os.makedirs(BASE_DIR, exist_ok=True)
    dirs = create_directory_tree(BASE_DIR)
    print(f"  {len(dirs)} Verzeichnisse erstellt")
    print()

    # Dateien in Batches aufteilen
    batch_size = TARGET_FILES // NUM_WORKERS
    remainder = TARGET_FILES % NUM_WORKERS

    tasks = []
    offset = 0
    for w in range(NUM_WORKERS):
        count = batch_size + (1 if w < remainder else 0)
        tasks.append((w, offset, count, dirs))
        offset += count

    # Parallel generieren
    print(f"Generiere {TARGET_FILES:,} Dateien mit {NUM_WORKERS} Workern...")
    start_time = time.time()

    total_stats = defaultdict(lambda: {"count": 0, "bytes": 0})
    total_errors = 0

    with multiprocessing.Pool(NUM_WORKERS) as pool:
        for i, (stats, errors) in enumerate(pool.imap_unordered(worker, tasks)):
            total_errors += errors
            for cat, data in stats.items():
                total_stats[cat]["count"] += data["count"]
                total_stats[cat]["bytes"] += data["bytes"]

            # Fortschritt
            done = sum(s["count"] for s in total_stats.values())
            elapsed = time.time() - start_time
            rate = done / elapsed if elapsed > 0 else 0
            total_bytes = sum(s["bytes"] for s in total_stats.values())
            eta = (TARGET_FILES - done) / rate if rate > 0 else 0
            print(f"\r  {done:>10,} / {TARGET_FILES:,} Dateien "
                  f"({total_bytes / 1024**3:.1f} GB) "
                  f"[{rate:.0f} files/s, ETA {eta/60:.0f}min]", end="", flush=True)

    elapsed = time.time() - start_time
    print()
    print()

    # Statistiken
    total_files = sum(s["count"] for s in total_stats.values())
    total_bytes = sum(s["bytes"] for s in total_stats.values())
    compressible_bytes = sum(
        s["bytes"] for cat, s in total_stats.items()
        if cat in ("config", "cache", "source", "document", "text")
    )

    print(f"=== Ergebnis ===")
    print(f"Dateien:        {total_files:,}")
    print(f"Groesse:        {total_bytes / 1024**3:.1f} GB")
    print(f"Fehler:         {total_errors}")
    print(f"Dauer:          {elapsed:.0f}s ({elapsed/60:.1f}min)")
    print(f"Rate:           {total_files/elapsed:.0f} files/s")
    print()
    print(f"Komprimierbar:  {compressible_bytes / 1024**3:.1f} GB ({compressible_bytes*100/total_bytes:.0f}%)")
    print(f"Nicht-kompr.:   {(total_bytes-compressible_bytes) / 1024**3:.1f} GB ({(total_bytes-compressible_bytes)*100/total_bytes:.0f}%)")
    print(f"Geschaetzte Kompressionsrate: ~{100 - (compressible_bytes*20 + (total_bytes-compressible_bytes)*95)/total_bytes:.0f}%")
    print()
    print(f"Kategorie-Verteilung:")
    for cat in sorted(total_stats.keys()):
        s = total_stats[cat]
        print(f"  {cat:<15} {s['count']:>10,} Dateien  {s['bytes']/1024**3:>8.1f} GB")


if __name__ == "__main__":
    main()
