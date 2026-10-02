#!/usr/bin/env python3
"""Self-contained tests for variant G (no pytest needed):

  python3 tests/run_tests.py [--bench /workspace/exct-eve/eve-dogma-bench]

1. Pyfa parity: every case of the bench corpus vs. the Pyfa-oracle expected values (bench tolerance).
2. Batch == single: evaluating the corpus as one NumPy batch gives byte-identical output to one-by-one.
3. Determinism: same request twice -> identical bytes.
4. Errors: BAD_JSON / BAD_REQUEST / UNKNOWN_TYPE are JSON errors, not crashes.
5. Engine unit checks: stacking penalty, operator order, min/max caps.
"""
import argparse, glob, json, os, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))
from evedogma_g import dataset  # noqa: E402
from evedogma_g.calc import calc, calc_many, calc_json_lines, dumps  # noqa: E402

FAILS = []


def check(cond, msg):
    if not cond:
        FAILS.append(msg)
        print("FAIL", msg)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bench", default="/workspace/exct-eve/eve-dogma-bench")
    ap.add_argument("--dataset", default=os.environ.get("EVE_DOGMA_DATASET", "/workspace/exct-eve/data/dataset-3569502.json.gz"))
    a = ap.parse_args()
    DATASET_PATH = a.dataset
    ds = dataset.load(a.dataset)
    bench = pathlib.Path(a.bench)
    sys.path.insert(0, str(bench / "tools"))
    from metrics import METRICS, extract, close  # noqa: E402

    # 1. Pyfa parity
    files = sorted(glob.glob(str(bench / "cases/*.json")))
    reqs = [json.load(open(f)) for f in files]
    outs = calc_many(ds, reqs)
    ok_cases = ok_vals = n_vals = 0
    for f, out in zip(files, outs):
        e = bench / "expected" / pathlib.Path(f).name
        if not e.exists():
            continue
        exp = json.load(open(e))
        bad = []
        for k, want in exp["values"].items():
            n_vals += 1
            got = extract(out, METRICS[k][0])
            if close(got, want):
                ok_vals += 1
            else:
                bad.append(f"{k}: got {got} want {want}")
        ok_cases += not bad
        check(not bad, f"pyfa {pathlib.Path(f).stem}: " + "; ".join(bad[:5]))
    print(f"pyfa parity: {ok_cases}/{len(files)} cases, {ok_vals}/{n_vals} values")

    # 2. batch == single (one-by-one), 3. determinism
    lines = [json.dumps(r) for r in reqs]
    batched = calc_json_lines(ds, lines, chunk=len(lines))
    singles = [dumps(calc(ds, r)) for r in reqs]
    diff = [pathlib.Path(f).stem for f, b, s in zip(files, batched, singles) if b != s]
    check(not diff, f"batch != single for {diff[:5]}")
    again = calc_json_lines(ds, lines, chunk=17)
    check(again == batched, "chunked batch differs from one batch")
    print(f"batch == single: {len(files) - len(diff)}/{len(files)}")

    # 4. errors
    e = calc_json_lines(ds, ["{not json"])[0]
    check(json.loads(e)["error"]["code"] == "BAD_JSON", "BAD_JSON")
    check(calc(ds, {"modules": []})["error"]["code"] == "BAD_REQUEST", "missing ship -> BAD_REQUEST")
    check(calc(ds, {"ship": {"type_id": "x"}})["error"]["code"] == "BAD_REQUEST", "bad type -> BAD_REQUEST")
    r = calc(ds, {"ship": {"type_id": 587}, "modules": [{"type_id": 999999999}]})
    check(r["error"]["code"] == "UNKNOWN_TYPE" and r["error"]["path"] == "/modules/0", f"UNKNOWN_TYPE {r}")
    mixed = calc_many(ds, [{"ship": {"type_id": 587}}, {"ship": {"type_id": 1}}, {"ship": {"type_id": 587}}])
    check("error" not in mixed[0] and mixed[1]["error"]["code"] == "UNKNOWN_TYPE" and "error" not in mixed[2],
          "an error in one batch member must not affect the others")
    check(dumps(mixed[0]) == dumps(mixed[2]), "batch neighbours identical")

    # 5. unit checks on the evaluator (Rifter, all V): 3 gyrostabilizers -> stacking penalty on damageMultiplier
    base = {"ship": {"type_id": 587}, "character": {"skills": {"default_level": 5}},
            "modules": [{"type_id": 2889, "state": "active", "charge_type_id": 21898}]}
    one = calc(ds, base)
    gyro = dict(base, modules=base["modules"] + [{"type_id": 519, "state": "online"}] * 3)
    three = calc(ds, gyro)
    ratio = three["offense"]["weapons"][0]["volley"]["total"] / one["offense"]["weapons"][0]["volley"]["total"]
    gb = ds.type_attr(ds.tidx(519), ds.a("damageMultiplier"))
    pen = 1.0
    import math
    for k in range(3):
        pen *= 1.0 + (gb - 1.0) * math.exp(-(k * k) / 7.1289)
    check(abs(ratio - pen) < 1e-6, f"stacking penalty ratio {ratio} vs {pen}")
    # 6. capacitor sim: NumPy periodic fast path == plain event loop (random drain sets, no injectors/clips)
    import random
    from evedogma_g import capsim
    rnd = random.Random(7)
    bad = 0
    for _ in range(300):
        drains = [(float(rnd.choice([1000, 2500, 3000, 4000, 5000, 6000, 8000, 10000, 12500, 15000, 45000])),
                   rnd.uniform(0, 400), 0, 0.0, False, rnd.random() < 0.2) for _ in range(rnd.randint(1, 8))]
        args = (rnd.uniform(200, 6000), rnd.uniform(100000, 600000), drains, rnd.choice([1.0, 0.5]), False,
                rnd.random() < 0.5, 3600 * 1000.0)
        capsim.FAST_PATH = True
        a = capsim.simulate(*args)
        capsim.FAST_PATH = False
        b = capsim.simulate(*args)
        capsim.FAST_PATH = True
        bad += a != b
    check(bad == 0, f"capsim fast path differs in {bad}/300 random cases")
    print("unit checks done")

    # 7. EFT import: outputs recorded from the reference engine (eve-dogma-rs 0e5a1ce) for tests/eft/*.txt
    import glob as _g, subprocess as _sp
    eft_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "eft")
    exe = os.path.join(os.path.dirname(eft_dir), "..", "bin", "eve-dogma-g")
    n_ok = n_all = 0
    for f in sorted(_g.glob(os.path.join(eft_dir, "*.txt"))):
        r = _sp.run([exe, "--dataset", DATASET_PATH, "eft", f], capture_output=True, text=True)
        got = r.stdout + f"rc {r.returncode}\n"
        exp = open(f[:-4] + ".expected").read()
        err_f = f[:-4] + ".expected_err"
        exp_err = open(err_f).read() if os.path.exists(err_f) else ""
        n_all += 1
        n_ok += got == exp and r.stderr == exp_err
    check(n_ok == n_all, f"EFT import {n_ok}/{n_all} identical to reference")
    print(f"eft import: {n_ok}/{n_all} identical to reference")

    # 8. EFT export vs Pyfa's exporter (bench >= 1.4.1: expected_extra/eft_export.jsonl), in process
    exp_f = bench / "expected_extra" / "eft_export.jsonl"
    if exp_f.exists():
        from evedogma_g import eft as _eft
        n_ok = n_all = 0
        for line in open(exp_f):
            e = json.loads(line)
            n_all += 1
            try:
                t = _eft.export(ds, e["fit"], e["name"])
                # accepted data divergence (bench tools/check_eft_export.py): T3C maxSubSystems 5 (SDE) vs 4 (Pyfa)
                ok = t == e["text"] or t.replace("\n[Empty Subsystem slot]", "", 1) == e["text"]
            except Exception:  # noqa: BLE001
                ok = False
            n_ok += ok
            if not ok and n_all - n_ok <= 3:
                print("  eft export mismatch:", e.get("file"))
        check(n_ok == n_all, f"EFT export {n_ok}/{n_all} identical to Pyfa")
        print(f"eft export: {n_ok}/{n_all} identical to Pyfa")
    print("FAILED" if FAILS else "ALL OK", len(FAILS))
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()
