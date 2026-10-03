# G3 perf history

All runs use `bench/perf.py` (same harness for both trees) on the shared 8-core box: Python 3.13, NumPy 2.5.
Figures are CPU-time minima. Before and after were run alternately, three rounds each, and the best round is
shown. The box is noisy: ±30 % between rounds is normal, and the cold-start wall time (c) is noisier still.

| metric | f7aa4cb (round-2 report) | ad61502 (time-axis vectorisation) | change |
|---|---|---|---|
| a) corpus 111 req / 1274 pts, cold fit cache | 338 ms (3.8 k pts/s) | 254 ms (5.0 k pts/s) | −25 % |
| a) corpus, warm fit cache | 45.6 ms (27.9 k pts/s) | 28.6 ms (44.6 k pts/s) | −37 % |
| b) dense damage/distance 500 pts, cold fit | 3.2 ms | 3.2 ms | = (fit calc dominates) |
| b) dense damage/distance 500 pts, warm fit | 0.40 ms | 0.33 ms | −18 % |
| d) damage/distance 100 k pts, warm | 53 ms (1.87 M pts/s) | 31 ms (3.19 M pts/s) | −42 % |
| f) all 33 time-axis cases at 500 pts, warm | 105.5 ms (156 k pts/s) | 12.3 ms (1.34 M pts/s) | **8.6×** |
| f) slowest: kestrel breacher 500 pts | 63.2 ms | 2.2 ms | 29× |
| f) rifter / hyperion reload 500 pts | 12.4 / 8.0 ms | 1.2 / 1.4 ms | ~6–10× |
| c) cold start + 1 request (wall) | 176–196 ms | 181–252 ms | = (NumPy import ≈ 170 ms) |
| cold, per request (fresh engine each), sum over corpus | 501 ms | 402 ms | out_of_range 67.6 → 13.8 ms |

Correctness after every commit:
- 111/111 graph cases and 1843/1843 sample values.
- 326/326 stats cases from the 1.8.0 corpus (`variant-g/tests/run_tests.py`).
- `bench/identity_check.sh`: 3285 stress requests are byte-identical to f7aa4cb, cached and `--no-cache`.

Commits in this round:
- e154c27: prepared entry tables and breacher tick matrices for the time axis.
- cf04564: breacher ticks as one block, plus vectorised validation of x.
- 21568f6: segment memo in the time-cache build.
- bb43a90: tackle grouping, and a shared unerr of query times.
- c3b4d4f: batched application-profile bisections.
- e4e2a1b: per-request memo of the unscrammed target variant.
- ad61502: cumulative tables built incrementally.
