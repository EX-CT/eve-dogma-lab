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
