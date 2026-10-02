# PROGRESS — variant I
- 2026-10-03 04:10 CST: salsa engine core (spec / engine / session), stats+capsim+EFT shared with variant A.
- 2026-10-03 04:30 CST: projected fits/charges, remote reps/neuts/nos/cap transfer, fleet booster fits, missile range
  → eve-dogma-bench: 249/249 cases, 13 812/13 812 values, deterministic; incremental == fresh (byte-identical).
- Next: speed (cheaper per-get path, memoised stats sections, spec build reuse).

## 2026-10-03 05:15 CST — bench 1.4.0 (contract 1.4.1/1.5)
- Ported sustained tank, cap-booster forced reload, jam chance, warp scramble, projected fighters,
  drone/fighter application fields, and fighter self abilities (MWD/AB/evasive) from A.
- Contract 1.4.1: a calc error exits 2 and still writes JSON; missing `options` ⇒ validate=true.
- Perf: shared skill/type spec cache, value cache, capsim memo, indexed target sets that backdate,
  and a no-modifier fast path in `value()`.
- Bench 1.4.0+65fab29: 289/289 cases, 18,591/18,591 values; batch 514 fits/s, latency 0.296 ms/calc, cold 157 ms.

## 2026-10-03 ~06:00 CST — bench 1.5.0 (contract 1.4.2) + speed round 1
- Projected Tracking/Guidance Disruptors (Pyfa Effect6424/6423), ported from A e552cb9: 295/295, 18,978 values.
- Contract 1.4.1 extras: interim `search` spec (limit 20, kinds, exact>prefix>substring, typeID ties) and Pyfa-exact
  `eft_export` (A's exporter; ship slot totals come from the salsa session). `rpc_cmd` is in bench.yaml; eft export 295/295.
- **Derived binary dataset cache** (`src/bincache.rs`): the first run decodes the JSON.gz (~140 ms) and writes
  `$TMPDIR/eve-dogma-i-<ver>-<fxhash of file bytes>.bin` (5 MB). Later runs load it in ~10 ms, so cold start dropped
  from ~157 ms to ~20 ms. The key covers the exact dataset bytes; disable with `EVE_I_NO_CACHE=1`, relocate with `EVE_I_CACHE_DIR`.
  Note that every per-process bench run after the first one hits the cache.
- mimalloc as the global allocator.
