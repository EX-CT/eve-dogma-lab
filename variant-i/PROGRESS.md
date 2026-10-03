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

## 2026-10-03 ~06:30 CST — bench 1.8.0 (326 cases, frozen until 10:20)
- Ported from A 9f8579c:
  - weather / AoE cloud beacon buffs in the fleet-buff pool, including the unpenalised weather buffs and drone targets;
  - incursion system effects (OffensiveDefensiveReduction);
  - full-strength burst projectors (web/paint/damp/track, plus neut/ECM as stats sources);
  - the Standup weapon disruptor and Breach Pod damage control.
- Result: 326/326, 21,051/21,051, EFT export 326/326.
- Perf: salsa durability. Skill and character specs, the slot ids, and the fit core are HIGH durability, so the
  ~400 skill memos are shallow-verified when only modules or the ship change. That is -10% Ir on the corpus.

## 2026-10-03 06:20–08:20 CST — speed round 3 (bench 1.8.0, final)
Every commit below was checked before it was pushed. The checks were: 326/326 and 21,051/21,051 values; batch output
byte-identical to the golden for the corpus x5; the corpus run forward and reverse in one session matching a fresh
database per request (INC_OK, FRESH_OK); EFT export 326/326; deterministic output.
- 59d93dc:
  - `item_mods` carries the spec, so an attribute records one dependency.
  - A `roles` query means bursts and RAH skip skill specs.
  - The skill level list is memoised.
  - Facade layer and modmap caches.
- 7c192b1:
  - Floats are rounded in a serde Formatter at serialisation.
  - Batch output goes through a BufWriter.
  - Attr-id caches in `validate` and `raw_cycle_ms`; `Consts::special` is a bitmap.
- 3928254: `load` diffs specs against a shadow Vec with no salsa reads; `validate` scans canFit attributes in one pass.
- 84cd44a: **attributes are evaluated inline from the memoised modifier graph** (`engine::VCache`).
  - The per-attribute salsa memo is used automatically when a request keeps the previous request's hull, which is
    what an edit looks like.
  - `EVE_I_ATTR_MEMO=0/1` forces inline or memo mode.
  - The inline cache is reused while the salsa revision is unchanged. See DESIGN.md.
- 6759b31: per-target maps and view caches are pre-sized, and the index skips copying required skills for skills.
- 71027e5: SmallVec in `attr_body`. Fast path for `py_round2` away from ties, validated on 2M random values against
  the exact path.
- c434df7: resolved skill specs are memoised with the level list; `validate` looks skills up in a sorted Vec.
- e0f3f89: `raw_cycle_ms` is memoised per item per view.
- 9a4844a: `validate` reads required skills in one pass (`req_bits`).
- Tried and reverted:
  - a `has_effect_named` memo (no gain);
  - a new DB per hull or per request ("soft reset"): 2.1–2.3G vs 1.45G Ir on the single corpus.
- Not done: PGO, which would have to be trained on a representative workload rather than the scored corpus.
- Results:
  - Callgrind Ir for the corpus x5 went from 6.54G to **4.61G** (-30%).
  - An edit recomputes in ~83–90 µs at best, down from ~211 µs.

### Official bench 1.8.0+33db85a, head 9a4844a (08:18 CST, `bench.py --only I`)
- Accuracy: 326/326 cases, 21,051/21,051 values. Deterministic. EFT export 326/326.
- Batch: 1799 fits/s. Cold start: 19.8 ms per process.
- Latency: **0.017 ms/fit as printed, but treat it as a measurement artefact.** run.py computes
  (500 identical fits - 1 fit) / 499. On this run the single-fit reference took 72.3 ms (cold start median 19.8 ms),
  which pulls the result down. An identical re-request is genuinely cheap in I, because the revision is unchanged
  and the cache is reused. Still, two dev reruns of the same binary straight afterwards gave 0.146 and 0.194 ms/fit.
  Use ~0.15–0.2 ms for comparisons.
- Copied to `bench/official/` (scorecard and combined.md).

## 2026-10-03 08:50 CST — bench 1.9.0 (staged branch bench-1.9.0 @ 29c1f2b), parked
- Head 9a4844a scored in a temporary bench clone: 329/331 cases, 22,037/22,046 values (`bench/bench-1.9.0/`).
- Two cases are missing:
  - `breacher_kestrel`: breacher pod `pure` damage (one `dotMaxDamagePerTick` tick, strongest pod only).
  - `overheat_order_tengu`: per-module overheat order. A hardener reads `overloadHardeningBonus` before a Defensive
    subsystem listed after it has boosted it.
- No code has been ported yet. This was parked when I was reassigned to the mutated-modules suite.
