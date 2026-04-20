#!/usr/bin/env python3
"""Fail the build when any criterion benchmark's PR mean is more than
THRESHOLD_PCT percent slower than its baseline mean.

Reads criterion's saved baseline JSON directly from target/criterion, so no
external crate (critcmp, bencher, ...) is needed at CI time. Usage:

    scripts/bench-compare.py <base-name> <pr-name> [threshold-pct]

Defaults: base-name=base, pr-name=pr, threshold-pct=10.

Criterion's per-baseline layout is:

    target/criterion/<group>/<bench>/<baseline>/estimates.json

`estimates.json` contains `mean.point_estimate` in nanoseconds. We compare
means (not p99) because criterion doesn't serialize p99 separately; mean
tracks p99 tightly for the cold-cache, short-run benches this gate targets.
"""

from __future__ import annotations

import json
import pathlib
import sys

ROOT = pathlib.Path("target/criterion")


def read_mean(baseline_dir: pathlib.Path) -> float | None:
    est = baseline_dir / "estimates.json"
    if not est.is_file():
        return None
    data = json.loads(est.read_text())
    try:
        return float(data["mean"]["point_estimate"])
    except (KeyError, TypeError, ValueError):
        return None


def main() -> int:
    base_name = sys.argv[1] if len(sys.argv) > 1 else "base"
    pr_name = sys.argv[2] if len(sys.argv) > 2 else "pr"
    threshold = float(sys.argv[3]) if len(sys.argv) > 3 else 10.0

    if not ROOT.is_dir():
        print(f"error: {ROOT} not found — did criterion run?", file=sys.stderr)
        return 2

    rows: list[tuple[str, float, float, float]] = []
    for pr_est in ROOT.rglob(f"{pr_name}/estimates.json"):
        pr_dir = pr_est.parent
        bench_dir = pr_dir.parent
        base_dir = bench_dir / base_name
        pr_mean = read_mean(pr_dir)
        base_mean = read_mean(base_dir)
        if pr_mean is None or base_mean is None:
            continue
        name = str(bench_dir.relative_to(ROOT))
        delta_pct = (pr_mean - base_mean) / base_mean * 100.0
        rows.append((name, base_mean, pr_mean, delta_pct))

    if not rows:
        print(
            f"error: no matching baselines found under {ROOT} "
            f"(looked for {base_name!r} and {pr_name!r})",
            file=sys.stderr,
        )
        return 2

    rows.sort(key=lambda r: r[3], reverse=True)
    width = max(len(r[0]) for r in rows)
    print(f"{'bench':{width}}  {'base':>12}  {'pr':>12}  {'Δ':>8}")
    print("-" * (width + 38))
    failed: list[tuple[str, float]] = []
    for name, b, p, pct in rows:
        marker = " !" if pct > threshold else ""
        print(f"{name:{width}}  {b/1000:>9.1f}µs  {p/1000:>9.1f}µs  {pct:>+7.1f}%{marker}")
        if pct > threshold:
            failed.append((name, pct))

    if failed:
        print(f"\nFAIL: {len(failed)} bench(es) regressed >{threshold:.1f}%:")
        for name, pct in failed:
            print(f"  {name}: {pct:+.1f}%")
        return 1

    print(f"\nOK: no regressions above {threshold:.1f}%")
    return 0


if __name__ == "__main__":
    sys.exit(main())
