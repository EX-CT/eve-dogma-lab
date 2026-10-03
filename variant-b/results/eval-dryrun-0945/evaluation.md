# Engine round 1 evaluation (DRY RUN)

**DRY RUN — not final results.** Generated 2026-10-03 09:45:03 CST (Asia/Shanghai) by `tools/evaluate.py` (`32295d5`); bench pinned to 1.8.0 @ `3da9671` (326 cases); commits: current head; runs = 3; host 8 CPUs; total wall time 2.6 min. Perf numbers were measured on a shared, loaded machine: compare with the loadavg column.

## Ranking

| rank | variant | commit (CST) | cases | values | ms/calc | fits/s | cold ms | speed | maint | features | port | **total** | load (1m) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | B data-oriented Rust | `bcd80df` 10-03 09:42 | 326/326 | 21051/21051 | 0.099 | 21513 | 4.7 | 1.00 | 0.92 | 1.00 | 0.50 | **0.923** | 2.1–2.3 |

## Maintainability and features

| variant | core LOC (languages) | test LOC | tests (own suite) | runtime deps | build s | README/DESIGN/LICENSE | license | hard-coded effects (h) | data-driven ratio | EFT exp | EFT parse | RPC | search | type | portability |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| B | 6220 (Rust 6220) | 672 | passed (51✓/0✗, cargo, 74.7 s) | 6 | 72.5 (incremental) | ✓✓✓ | LGPL-3.0-or-later | 96 | 0.970 | 326/326 | 20/20 | 1.00 | 1.00 | 1.00 | docs-only |

## Licensing (mainline eve-dogma-rs is LGPL-3.0-or-later)

| variant | license | -only/-or-later from | LICENSE files (kind) | mergeable into LGPL-3.0-or-later mainline | reason |
|---|---|---|---|---|---|
| B | LGPL-3.0-or-later | package metadata | LICENSE (LGPL-3); LICENSE.GPL-3.0 (GPL-3); <branch root>/LICENSE (LGPL-3); <branch root>/LICENSE.GPL-3.0 (GPL-3) | **yes** | LGPL v3 LICENSE text + GPL companion text |

## Latency measurement (single CPU, own measurement)

| variant | ms/calc (median) | min | max | spread | valid/samples | N per sample | startup ms | flags |
|---|---|---|---|---|---|---|---|---|
| B | 0.0988 | 0.0847 | 0.1112 | 27% | 5/5 | 4120 | 7.68 | ok |

## Per-run measurements

| variant | run | wall s | cases ok | run.py ms/calc (info only) | fits/s | cold ms | loadavg before | loadavg after |
|---|---|---|---|---|---|---|---|---|
| B | 1 | 1.9 | 326 | 0.030 | 21513 | 4.7 | [2.27, 3.06, 3.56] | [2.09, 3.01, 3.54] |
| B | 2 | 2.0 | 326 | 0.035 | 19739 | 4.7 | [2.09, 3.01, 3.54] | [2.09, 3.01, 3.54] |
| B | 3 | 1.9 | 326 | 0.027 | 21970 | 4.5 | [2.09, 3.01, 3.54] | [2.08, 3.0, 3.53] |

## Scoring rules

- **Version rule:** each variant is evaluated at its branch HEAD as of the cutoff (`--as-of`; unified scoring 2026-10-03T10:15:00+08:00). Self-reported "final" versions are reference only. A commit that fails the gate is **disqualified**; no fallback to an older commit.
- **Runs:** one untimed warm-up batch, then `--runs` official-scorer runs per variant, one variant at a time; perf = median.
- **Gate (correctness):** ranked only if the variant built, ran, and passed **all** bench cases (cases fully correct = cases, no engine errors) in every run.
- **Total = 0.40·Speed + 0.35·Maintainability + 0.15·Features + 0.10·Portability** (each in [0, 1]).
- `L(x, best, span) = clamp(1 − log10(x/best)/log10(span), 0, 1)` for lower-is-better `x` (1 = best ranked variant, 0 = `span`× worse).
- **Bench pin:** cases/expected/run.py from bench `3da9671` (1.8.0, 326 cases; = 0969967 cases), whatever upstream main is.
- **Latency** = own measurement (not run.py's): batch command pinned to one CPU (taskset), (t_N − t_1)/(N − 1) with N sized for ≈0.5 s of calcs; ≥5 independent samples, median; samples ≤0, < 0.002 ms, or > t_N/N are invalid; spread > 50 % ⇒ re-measure (≤3 extra), then flagged.
- **Licensing** (at the evaluated commit): judged by the LICENSE texts in the variant dir and branch root (LGPL header ± companion GPL text ⇒ LGPL; GPL header without LGPL ⇒ GPL); SPDX metadata/README only refine -only/-or-later. Mergeable into LGPL-3.0-or-later mainline: LGPL-3/permissive yes, GPL no, no file / conflict unknown. Informational, not scored.
- **Speed** = 0.5·L(latency ms/calc, 100) + 0.3·L(1/batch fits·s⁻¹, 100) + 0.2·L(cold-start ms, 100); medians over runs.
- **Maintainability** = 0.25·Tests + 0.20·DataDriven + 0.20·Size + 0.15·Docs + 0.10·Deps + 0.10·Build.
  Tests: passed → 0.6 + 0.4·min(1, log10(1+n)/2); failed → 0.2; timed out → 0.3; none → 0.
  DataDriven: L(h+10, h_min+10, 10), h = distinct dataset effect names (camelCase, ≥8 chars) referenced in hand-written core source (heuristic for per-effect special-casing).
  Size: L(core LOC, min, 10). Docs: 0.4 README + 0.4 DESIGN + 0.2 LICENSE. Deps: 1/(1+n/5), n = direct runtime deps. Build: L(build s, best, 100), only if every ranked build was fresh (`--fresh-clones`), else dropped and the other weights renormalised.
- **Features** = mean(EFT, RPC, search, type); EFT = ½ export (Pyfa byte-exact) + ½ eft_parse round-trip; others = share of RPC probes passing.
- **Portability** = 1 if a WASM/browser build exists in code, 0.5 if only documented, else 0.

## Reproduce

```
python3 tools/evaluate.py --only B --dry-run --out /tmp/vbB-eval2 --work-dir /tmp/vbB-evalwork
```

