#!/usr/bin/env python3
"""Regression guard for refactors: output of this tree vs a baseline tree (e.g. `git archive <rev>`) must be
byte-identical over the bench corpus, each case also with options.include_attributes = "all".
usage: compare_prev.py BASELINE_VARIANT_G_DIR [CASES_DIR]"""
import glob, json, os, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DATASET = os.environ.get("EVE_DATASET", "/workspace/exct-eve/data/dataset-3569502.json.gz")


def run(root, path):
    r = subprocess.run([os.path.join(root, "bin/eve-dogma-g"), "--dataset", DATASET, "batch"],
                       stdin=open(path), capture_output=True, text=True, check=True)
    return r.stdout.splitlines()


def main():
    base = sys.argv[1]
    cases = sys.argv[2] if len(sys.argv) > 2 else "/workspace/exct-eve/eve-dogma-bench/cases"
    names, lines = [], []
    for f in sorted(glob.glob(os.path.join(cases, "*.json"))):
        r = json.load(open(f))
        names.append(os.path.basename(f)); lines.append(json.dumps(r))
        r2 = json.loads(json.dumps(r)); r2.setdefault("options", {})["include_attributes"] = "all"
        names.append(os.path.basename(f) + "+all"); lines.append(json.dumps(r2))
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as t:
        t.write("\n".join(lines) + "\n")
    a, b = run(HERE, t.name), run(base, t.name)
    bad = [n for n, x, y in zip(names, a, b) if x != y]
    print(f"{len(names)} requests, {len(names) - len(bad)} byte-identical to baseline")
    for n in bad[:10]:
        print("  differs:", n)
    sys.exit(1 if bad or len(a) != len(b) else 0)


main()
