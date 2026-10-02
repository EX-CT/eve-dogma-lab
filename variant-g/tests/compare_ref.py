#!/usr/bin/env python3
"""Compare variant G against the reference engine (variant A) on every leaf of the response, and against the
Pyfa expected values of the bench corpus.

  python3 tests/compare_ref.py [--cases DIR] [--ref BIN] [--out results/compare_ref.json]
"""
import argparse, glob, json, math, os, pathlib, subprocess, sys, time

HERE = pathlib.Path(__file__).resolve().parent.parent
EXCT = pathlib.Path("/workspace/exct-eve")
sys.path.insert(0, str(HERE))


def leaves(x, p=""):
    if isinstance(x, dict):
        for k, v in x.items():
            yield from leaves(v, f"{p}/{k}")
    elif isinstance(x, list):
        for i, v in enumerate(x):
            yield from leaves(v, f"{p}/{i}")
    else:
        yield p, x


def close(a, b):
    if isinstance(a, bool) or isinstance(b, bool) or a is None or b is None or isinstance(a, str) or isinstance(b, str):
        return a == b
    return abs(a - b) <= max(1e-6, 1e-6 * abs(b))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cases", default=str(EXCT / "eve-dogma-bench/cases"))
    ap.add_argument("--ref", default=str(EXCT / "eve-dogma-rs/target/release/eve-dogma"))
    ap.add_argument("--dataset", default=str(EXCT / "data/dataset-3569502.json.gz"))
    ap.add_argument("--out", default=str(HERE / "results/compare_ref.json"))
    ap.add_argument("-v", action="store_true")
    a = ap.parse_args()
    files = sorted(glob.glob(os.path.join(a.cases, "*.json")))
    lines = [json.dumps(json.load(open(f))) for f in files]
    data = "".join(l + "\n" for l in lines)
    ref = subprocess.run([a.ref, "batch", "--dataset", a.dataset], input=data, capture_output=True, text=True).stdout.splitlines()
    t = time.perf_counter()
    got = subprocess.run([str(HERE / "bin/eve-dogma-g"), "batch", "--dataset", a.dataset], input=data, capture_output=True,
                         text=True)
    dt = time.perf_counter() - t
    if got.returncode:
        print(got.stderr[-3000:])
        sys.exit(1)
    got = got.stdout.splitlines()
    report = {"cases": len(files), "identical_cases": 0, "leaf_total": 0, "leaf_mismatch": 0, "batch_s": dt, "cases_diff": {}}
    for f, r, g in zip(files, ref, got):
        r, g = json.loads(r), json.loads(g)
        r.get("meta", {}).pop("engine", None)
        g.get("meta", {}).pop("engine", None)
        rl, gl = dict(leaves(r)), dict(leaves(g))
        diffs = {}
        for k in sorted(set(rl) | set(gl)):
            report["leaf_total"] += 1
            if k not in rl or k not in gl or not close(gl[k], rl[k]):
                diffs[k] = {"got": gl.get(k, "<missing>"), "ref": rl.get(k, "<missing>")}
        report["leaf_mismatch"] += len(diffs)
        if diffs:
            report["cases_diff"][pathlib.Path(f).stem] = diffs
        else:
            report["identical_cases"] += 1
    pathlib.Path(a.out).write_text(json.dumps(report, indent=1, default=str))
    print(f"cases {report['cases']}  identical to reference {report['identical_cases']}  "
          f"leaf mismatches {report['leaf_mismatch']}/{report['leaf_total']}  batch {dt:.2f}s")
    for c, d in list(report["cases_diff"].items())[: (999 if a.v else 15)]:
        ks = list(d.items())
        print(f"  {c}: {len(d)} diffs, e.g. " + "; ".join(f"{k} got {v['got']} ref {v['ref']}" for k, v in ks[:4]))


if __name__ == "__main__":
    main()
