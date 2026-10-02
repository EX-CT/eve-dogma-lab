# Instructions per calculation (callgrind, deterministic; Fit::build + compute_stats, all-V skills)

Measured 2026-10-03 04:45 (Asia/Shanghai) on the shared box; A = eve-dogma-rs @ 0e5a1ce, B = variant-b.

| case | A (M instr) | B (M instr) | A/B |
|---|---|---|---|
| exct_rifter | 6.89 | 1.53 | 4.5× |
| exct_hyperion | 7.98 | 2.53 | 3.2× |
| esf_vexor (cap sim heavy) | 19.00 | 13.57 | 1.4× |
| exct_nidhoggur | 9.59 | 4.02 | 2.4× |
| fleet_two_boosters_hyperion | 20.91 | 4.72 | 4.4× |

Wall-clock numbers from `run.py` vary 2–3× with box load (other workers compiling); instruction counts don't.
