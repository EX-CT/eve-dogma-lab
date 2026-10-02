# PROGRESS — variant B

Updated: 2026-10-03 04:40 (Asia/Shanghai)

## Done
- New engine (`src/engine.rs`): flat raw-modifier list → packed-key sort → CSR node graph with compile-time constant
  folding → iterative on-demand DFS evaluation; staged recompilation for bursts and RAH.
- Skill folding with dataset-level proof of validity and automatic fallback; lazy per-(skill, level, structure) probes.
- Index-list target selection, dense attribute metadata, Pyfa-compatible everything (shared stats/capsim/eft).
- Feature parity with A @ 0e5a1ce: fleet booster fits, projected fits/modules/charges/drones, remote reps, neuts,
  nos, cap transfers, missile range.
- Fast dataset load (~65 ms vs ~140 ms), parallel ordered `batch`.
- Tests: oracle parity (249 cases, 13 812 values) green; full-output diff vs A: 249/249 byte-identical.
- Bench (`eve-dogma-bench/run.py`, shared noisy box): 249/249 cases, 13 812/13 812 values, cold start median 97 ms,
  batch 2 955 fits/s (all cores) / ~1 000–2 400 fits/s (1 thread, box load varies), rifter ≈ 0.29 ms/calc single
  thread (A ≈ 1.1–1.3 ms on the same run).

- Instruction counts vs A: see results/instructions.md (rifter 4.5×, hyperion 3.2×, fleet boosters 4.4× fewer).
- Robustness: packed sort key falls back to tuple sort for >65 535 items; errors per contract (BAD_REQUEST, UNKNOWN_TYPE, UNKNOWN_METHOD).

## Next
- `sweep`: multi-lane evaluation of K variants on one compiled plan.
- Cut remaining per-fit allocations (item effect lists, req_skills) and stats-layer string lookups.
- Track A's new features (diff tool flags them immediately).

## 2026-10-03 ~05:10 CST — snapshot cache
- bincode dataset snapshot keyed by SHA-256 of dataset bytes: process cold calc ~25 ms (was ~90 ms).
- Parity: 289/289 bench cases byte-identical vs A@0e5a1ce (with and without cache); oracle test passes.

## 2026-10-03 ~05:30 CST — ported A@086dcb4 + A@ae4bfb0
- sustainable tank, cap booster forced reload, ECM jam chance, projected fighters, fighter self abilities, drone/fighter application fields.
- tests/ synced from A@ae4bfb0. Parity: 289/289 bench cases byte-identical vs A@ae4bfb0; oracle test passes.
