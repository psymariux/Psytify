#!/usr/bin/env python3
"""Campiona un processo Psytify da /proc: CPU per thread, RSS e memoria GPU di calcolo.

Uso: scripts/perf-sample.py PID [SECONDI]   (default 30)

Non richiede privilegi: legge solo /proc e nvidia-smi. Il risultato va riportato in
docs/PERFORMANCE.md con lo stato dell'app (fermo, in riproduzione, video, lyrics, formato).
"""

import os
import subprocess
import sys
import time

HZ = os.sysconf("SC_CLK_TCK")


def thread_ticks(pid):
    """CPU (utente + sistema, in tick) di ogni thread, per tid."""
    ticks = {}
    for tid in os.listdir(f"/proc/{pid}/task"):
        try:
            with open(f"/proc/{pid}/task/{tid}/stat") as stat:
                raw = stat.read()
        except OSError:
            continue  # il thread è terminato mentre lo leggevamo
        name = raw[raw.index("(") + 1 : raw.rindex(")")]
        fields = raw[raw.rindex(")") + 2 :].split()
        ticks[tid] = (name, int(fields[11]) + int(fields[12]))
    return ticks


def rss_mb(pid):
    with open(f"/proc/{pid}/status") as status:
        for line in status:
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) / 1024
    return 0.0


def compute_mib(pid):
    """Memoria di calcolo sulla GPU NVIDIA usata dal processo (0 se non c'è o non c'è NVIDIA)."""
    try:
        out = subprocess.run(
            ["nvidia-smi", "--query-compute-apps=pid,used_memory", "--format=csv,noheader,nounits"],
            capture_output=True,
            text=True,
            check=False,
        ).stdout
    except FileNotFoundError:
        return 0
    for line in out.splitlines():
        owner, used = (field.strip() for field in line.split(","))
        if owner == str(pid):
            return int(used)
    return 0


def main():
    pid = int(sys.argv[1])
    seconds = int(sys.argv[2]) if len(sys.argv) > 2 else 30
    before = thread_ticks(pid)
    started = time.monotonic()
    time.sleep(seconds)
    elapsed = time.monotonic() - started
    after = thread_ticks(pid)

    per_thread = []
    for tid, (name, now) in after.items():
        used = now - before.get(tid, (name, 0))[1]
        per_thread.append((used / HZ / elapsed * 100, name))
    per_thread.sort(reverse=True)
    total = sum(share for share, _ in per_thread)

    print(f"CPU totale: {total:.1f}% di un core (campione di {elapsed:.0f} s)")
    top = ", ".join(f"{name}={share:.1f}%" for share, name in per_thread[:8] if share >= 0.05)
    print(f"Thread più attivi: {top or 'nessuno'}")
    print(f"RSS: {rss_mb(pid):.0f} MB   GPU (calcolo): {compute_mib(pid)} MiB   thread: {len(after)}")


if __name__ == "__main__":
    main()
