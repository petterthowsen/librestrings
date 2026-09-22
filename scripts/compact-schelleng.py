#!/usr/bin/env python3
"""Convert an extracted mdw Schelleng archive from CSV to 4-channel FLAC.

Input:  <dataset>/<archive>/<date>_r_v<k>/{beta,timestamp,whole}_<n>.csv
Output: <dataset>/vb<speed>/<n>.flac and <dataset>/index.csv

Each FLAC holds the four whole_<n>.csv columns (bow force N, bow velocity m/s,
bridge force N, nut force N) at 50 kHz as 24-bit integers scaled so that
FULL_SCALE maps to 2^23 (LSB about 1.9e-6, far below the sensor noise).
Decode: int24 * FULL_SCALE / 2^23. Each file is checked to decode bit-exactly
before its CSVs are deleted, so this can be rerun after an interruption.

Requires numpy and ffmpeg.
"""
import csv
import multiprocessing
import subprocess
import sys
from pathlib import Path

import numpy as np

FULL_SCALE = 16.0
SAMPLE_RATE = 50_000
BOW_SPEEDS = (0.05, 0.1, 0.2)
INDEX_COLUMNS = [
    "file", "vb_nominal", "n", "beta", "win_start", "win_end", "samples",
    "fb_mean", "vb_mean", "bridge_rms",
]


def encode(q, path):
    subprocess.run(
        ["ffmpeg", "-v", "error", "-y", "-f", "s32le", "-ar", str(SAMPLE_RATE),
         "-ch_layout", "quad", "-i", "-", "-c:a", "flac",
         "-bits_per_raw_sample", "24", "-compression_level", "8", str(path)],
        input=(q << 8).tobytes(), check=True)


def decode(path):
    raw = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), "-f", "s32le", "-"],
                         capture_output=True, check=True).stdout
    return np.frombuffer(raw, np.int32).reshape(-1, 4) >> 8


def nominal_speed(v):
    return min(BOW_SPEEDS, key=lambda s: abs(s - v))


def convert(job):
    src_dir, n, out_root = job
    whole = src_dir / f"whole_{n}.csv"
    beta_f = src_dir / f"beta_{n}.csv"
    ts_f = src_dir / f"timestamp_{n}.csv"
    x = np.loadtxt(whole, delimiter=",", dtype=np.float64, ndmin=2)
    if x.shape[1] != 4:
        raise ValueError(f"{whole}: expected 4 columns, got {x.shape[1]}")
    peak = np.abs(x).max()
    if peak >= FULL_SCALE:
        raise ValueError(f"{whole}: |x| = {peak} exceeds full scale {FULL_SCALE}")
    beta = float(beta_f.read_text().strip())
    a, b = (int(t) for t in ts_f.read_text().strip().split(","))
    w = x[a:b]
    vb = nominal_speed(float(w[:, 1].mean()))

    out = out_root / f"vb{vb}" / f"{n}.flac"
    out.parent.mkdir(exist_ok=True)
    q = np.round(x / FULL_SCALE * 2**23).astype(np.int32)
    encode(q, out)
    if not np.array_equal(decode(out), q):
        out.unlink()
        raise RuntimeError(f"{out}: round trip mismatch")
    for f in (whole, beta_f, ts_f):
        f.unlink()
    return {
        "file": str(out.relative_to(out_root)), "vb_nominal": vb, "n": n,
        "beta": beta, "win_start": a, "win_end": b, "samples": len(x),
        "fb_mean": round(float(w[:, 0].mean()), 6),
        "vb_mean": round(float(w[:, 1].mean()), 6),
        "bridge_rms": round(float(w[:, 2].std()), 6),
    }


def main():
    if len(sys.argv) != 2:
        sys.exit(f"usage: {sys.argv[0]} <dataset dir>")
    root = Path(sys.argv[1])
    index_path = root / "index.csv"
    rows = []
    if index_path.exists():
        with open(index_path) as f:
            rows = list(csv.DictReader(f))

    jobs = [(p.parent, int(p.stem.split("_")[1]), root)
            for p in sorted(root.glob("*/*_r_v*/whole_*.csv"))]
    print(f"{root}: {len(jobs)} files to convert, {len(rows)} already done")
    # Append each row as it completes: its CSVs are already deleted.
    with open(index_path, "a", newline="") as f, multiprocessing.Pool() as pool:
        writer = csv.DictWriter(f, fieldnames=INDEX_COLUMNS)
        if not rows:
            writer.writeheader()
        for i, row in enumerate(pool.imap_unordered(convert, jobs, chunksize=4), 1):
            writer.writerow(row)
            f.flush()
            rows.append(row)
            if i % 200 == 0:
                print(f"  {i}/{len(jobs)}", flush=True)

    # A re-extracted archive re-converts points; keep the newest row per file.
    rows = list({r["file"]: r for r in rows}.values())
    rows.sort(key=lambda r: (float(r["vb_nominal"]), int(r["n"])))
    with open(index_path, "w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=INDEX_COLUMNS)
        writer.writeheader()
        writer.writerows(rows)

    # Remove the now-empty extracted archive folders.
    for d in sorted(root.glob("*/*_r_v*")):
        if not any(d.iterdir()):
            d.rmdir()
    for d in root.iterdir():
        if d.is_dir() and not d.name.startswith("vb") and not any(d.iterdir()):
            d.rmdir()
    print(f"{root}: {len(rows)} points in {index_path}")


if __name__ == "__main__":
    main()
