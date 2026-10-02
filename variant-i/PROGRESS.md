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

## 2026-10-03 ~06:40 CST — bench 1.6.0 + speed round 2
- Ported projected Remote Tracking Computers (assistance-gated), npcEntityWeaponDisruptor (TD drones), and
  ChainLightning/salvage counted as damage effects (A c53d333 + working tree): 297/297, 19,103/19,103.
- Skill pruning as in A: a skill whose modifiers can reach nothing in the fit is not instantiated. The reach summary is
  cached per skill. Output is byte-identical over the corpus.
- Vendored salsa 0.28.2 with one patch (`vendor/salsa/src/active_query.rs`): query-stack frames that once held a very
  wide query (~550 deps: index, plan, ...) memset their whole IndexSet table on every later drain (11% of all
  instructions). Wide frames now hand over their set instead of draining it. About -7% Ir.
- Tried without success (reverted or off by default):
  - one merged query for all skills' modifiers: +8% Ir, because per-skill memos verify cheaper than they recompute;
  - `--pool N`, an open-fit workspace with one session per hull: slower on the corpus, because per-session cold
    caches cost more than the diff against the previous same-hull fit saves. It is kept as an option with default 1.

## 2026-10-03 ~06:10 CST — bench 1.7.0 (306 cases)
- Ported from A aa46025: local special module handlers (superweapon/lance speed + warp status, EHE, entosis, MJFG,
  local WDFG), doomsday sub-cycle DPS, Python round(v,2) for cpu/pg, TD drones. Result: 306/306, 19,621/19,621.
- Perf: modifiers that target sets (location/group/skill) are now stored as deferred set references in `outgoing`, so
  an item's outgoing modifiers depend only on the item. `incoming` expands them through the index. That gives -11% Ir
  on the corpus (1.73G -> 1.54G for 295 fits) with byte-identical output.
