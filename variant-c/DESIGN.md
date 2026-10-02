# Variant C — Go dogma engine (pull-based modifier registry)

Part of the EX-CT engine bake-off (`EX-CT/eve-dogma-lab`). Same contract as `eve-dogma-rs`
(stateless `FitRequest` JSON → `FitStats` JSON, same dataset `exct-eve-dataset` v1), different architecture.

## Goals (in order)
1. **Correctness** — identical numbers to the Pyfa oracle (`testdata/oracle/pyfa_expected.json`, 297 fits / 19 103 values; bench 1.6.0 297/297, EFT export 297/297).
2. **Speed** — beat eve-dogma-rs per calculation, not just Pyfa.
3. **Maintainability / embeddability** — idiomatic Go, zero dependencies (stdlib only), library + CLI + HTTP.

## Architecture

```
dataset.json.gz ──load once (parallel)──► Dataset (immutable, shared by goroutines)
                                 • types: sorted attr slices (no per-item copies), req skills, slot, effects
                                 • effects: pre-decoded modifier tuples
                                 • indices: name→id, group→types, published skills
FitRequest ──► Fit (one per calculation, cheap)
                 items[]  (ship, char, skills, modules, charges, drones, fighters, implants, boosters, mode, beacons, projected)
                 base overlay per item (only for mutated/overridden/skill-level attrs; everything else read from Dataset)
                 Registry: attr → ordered list of modifiers, each tagged with its *selector*
                     (not expanded to target items):
                     item · shipLoc · shipLoc+group · shipLoc+skill · owner+skill · charLoc · charLoc+group · char+skill
                 Skill template: skill modifiers for all-published-skills characters, built once per Dataset
                 Cache: (item,attr) → value, memoised on first read; explicit cycle stack
```

### Pull, not push
eve-dogma-rs (variant A) resolves every modifier to its concrete target items at registration
(`O(modifiers × items)`), creating attribute nodes for many attributes that are never read
(≈500 skills × their modifiers × every ship item). Variant C stores each modifier once, under the selector
it was declared with (location / group / required skill / owner). When `(item, attr)` is first read, the
engine walks the modifier list for `attr` and keeps the entries whose selector matches the item (its own
index, its location, its group, one of its required skills, its owner). Work is proportional to the
*attributes the stats layer actually reads*.

The list for each attribute is kept in registration order and split into three phases: pre-skill, skill and
post-skill. That makes the fold order, and so the floating-point summation order, identical to
eve-dogma-rs. Those are bit-identical results, not just results within 1e-6. Stacking-penalty factors are
precomputed constants equal to glibc `exp`. Go's `math.Exp` differs in the last ulp.

### Shared skill template
About 500 skills × their modifiers make up most of the registrations in a fit. When the character uses
the canonical skill set (every published skill, no extra ids) and the ship is not a structure, the skill
phase uses a per-Dataset template (`template.go`, built once under `sync.Once`). The template holds
selector-tagged modifiers whose source is "skill S, attribute A". Each fit supplies only its own skill
levels as an overlay. The fit's skill items share one backing array for their level overlay. Registering
skills becomes O(1) per fit instead of O(skills × modifiers).

### Dataset loading
A byte scanner splits the 7 MB JSON document into top-level sections without decoding them. The
`types` section (10 746 types) is split per type and decoded and indexed by a `GOMAXPROCS` worker pool.
Load time fell from ~300 ms to ~125 ms wall clock.

**Binary cache (v4, `cache.go`).** `LoadPathCached` keeps a flat little-endian copy of the derived tables:
types, attributes, effects, groups, categories, names, dbuffs and mutaplasmids. It is keyed by the sha256
of the dataset file, guarded by a crc32c trailer, and lives in `$EVE_DOGMA_CACHE_DIR` or the user cache dir
(`EVE_DOGMA_CACHE=off` disables it). Strings point into the cache buffer (`unsafe.String`, zero copy).
Type attributes are slab-allocated, and GC is paused while loading. Name and group indices are built lazily
(`sync.Once`), and the T3D mode lists are precomputed. Startup plus one calc takes 27 ms (min of 20), and the
first calc in a process ≈4.5 ms.

### Evaluation
CCP operator order (PreAssign, PreMul, PreDiv, ModAdd, ModSub, PostMul, PostDiv, PostPercent, PostAssign),
per-operator stacking-penalty buckets (exempt source categories Ship/Charge/Skill/Implant/Subsystem/Structure,
`e^-(i/2.67)^2`), assign = max/min by highIsGood, min/max attribute caps, cpu/power rounding, cycle guard.

### Dirty tracking
`Fit` supports incremental use (what-if loops, optimisers, MCP sessions). `EnableDepTracking()` records
reverse-dependency edges `(source item,attr) → (target item,attr)` while evaluating.
`SetBase(item, attr, v)` then invalidates only the transitive dependents. `Invalidate()` clears the
whole cache in O(1); structural changes such as fleet buffs and RAH adaptation use it. The stateless CLI
never mutates, so it pays nothing for this.

### Specials (no modifierInfo in the SDE)
Same list as eve-dogma-rs, implemented as explicit registry entries with computed sources:
AB/MWD (mass add + speed boost from thrust/mass), MWD/MJD signature bloom, slot/hardpoint modifiers (T3C),
bastion hull resists unpenalised, structure skill rules, booster side effects, fighter abilities, projected
webs/TPs/damps/sebos with range factor and resistance (sebos also scale scan strengths), warfare buffs (the
strongest source per buff id across own bursts and `fleet.booster_fits`; explicit `fleet.buffs` override),
RAH adaptation, projected fits (source fit computed separately, the active modules' and drones' attributes
frozen into overlays, × amount), charges on projected modules, incoming remote reps (Pyfa's
diminishing-returns formula) and neut/nos/cap-transfer drains in the capacitor sim, Pyfa missile range
(acceleration phase, floor/ceil blend, FoF cap).

### Hot path (per calculation)
Pyfa parity fixes the arithmetic, so the speed work removes overhead around it:
* **Fit reuse.** `Fit` objects (items, registry, caches) come from a `sync.Pool` and are reset rather than
  reallocated. Working memory is reused across calcs.
* **Dense registry and node cache.** Attribute ids are remapped to a dense index per Dataset, so registry
  and cache lookups are slice indexing, not maps. `attrSet.get` is a branch-free lower bound with an
  arithmetic mask.
* **Request decoder (`fastdec.go`).** A strict, reflection-free parser for `FitRequest`. It falls back to
  encoding/json on anything unusual and is fuzzed against it (`FuzzFastDecoder`, 6 M execs).
  `DecodeRequest` is used everywhere. Decode went from 35 to 7 µs/fit.
* **Output encoder (`jsonenc.go`, `rows.go`).** A hand-written encoder for the contract format (sorted
  keys, 1e-6 rounding, non-finite → null). Stats objects are typed rows: `modRow` for modules, `fobj` for
  fixed-key float objects, `kobj` for small unboxed key/value objects. `Tidy` turns them back into
  generic maps for library users, and `TestFastEncoder` checks byte equality with the generic path.
  Allocations per corpus run went from 98.5 k to 72.5 k.
* **Capsim.** Pyfa's heapq order is reproduced exactly, with the same iteration counts (Vexor 20 320). The
  reschedule is folded into the next pop (`pushPop`), and repeated exp/penalty terms are memoised. Vexor
  went from 980 to 795 µs. `math.Min`/`Max` were replaced by builtins with the same NaN/±0 behaviour
  (−10 % instructions).
* **Pipelines.** `batch` decodes, computes and encodes on N goroutines with order-preserving output.
  `serve-stdio` (JSONL) and `serve-http` are long-running and pay the dataset load once.
* **Serve-mode memo (`cmd/eve-dogma-go/memo.go`).** Long-running modes only: a bounded map of exact
  request bytes → response bytes. It holds up to `EVE_DOGMA_MEMO` entries (default 4096, `0` = off) and
  is cleared when full. `calc` and `batch`, and
  therefore the bench throughput figures, never use it.

### Layers
| package | file | role |
|---|---|---|
| `dogma` | `data.go` | dataset loader (gzip JSON → indexed structs) |
| | `request.go` | FitRequest v1 with serde-compatible defaults |
| | `engine.go` | item graph, registry, evaluation, specials, RAH, projected fits, buffs |
| | `template.go` | shared per-Dataset skill-modifier template |
| | `ids.go` | well-known attribute/effect/group ids resolved once per Dataset |
| | `stats.go` | resources/offense/defense/capacitor/navigation/targeting/validation |
| | `capsim.go` | Pyfa-compatible event-driven capacitor simulation |
| | `eft.go` | EFT import, Pyfa-exact EFT export (incl. mutations) |
| | `cache.go` | binary dataset cache v4 |
| | `fastdec.go` | reflection-free FitRequest decoder |
| | `jsonenc.go`, `rows.go` | contract JSON encoder, typed output rows (`modRow`, `fobj`, `kobj`) |
| `cmd/eve-dogma-go` | `main.go` | CLI: calc, batch, serve-stdio, serve-http, eft, search, type, meta, bench |
| | `memo.go` | serve-mode response memo |
| `tools/` | `diff_vs_rs.py`, `eft_export_check.py`, `instr.sh` | full-output diff against A, EFT export parity, callgrind instructions/calc |

### Concurrency
`Dataset` is immutable after load → any number of goroutines can `Calc` in parallel (HTTP server, batch
`-j N`). A `Fit` is single-goroutine.

## Trade-offs
* **Pull vs push.** Registering a modifier costs nothing, but each read scans the attribute's modifier list
  and tests selectors. Lists are short in practice because they are per attribute. A modifier that hits
  many items is not copied per item, and that copying is the main cost in A.
* **Exact parity with Rust over idiomatic floats.** The fold order and penalty constants follow
  eve-dogma-rs bit for bit. That adds some ceremony (the phase split, hardcoded constants), but full-output
  diffs against A are clean and easy to read.
* **Typed rows, generic API.** Hot objects are typed rows (`modRow`/`fobj`/`kobj`) that the encoder writes
  directly. Library callers still get `map[string]any` through `Tidy`. Two representations cost some code,
  and `TestFastEncoder` keeps them byte-identical.
* **No cgo, no dependencies; `unsafe` only for zero-copy cache strings.** Embeds in any Go service; `go build` is the whole toolchain.

## Contract
CLI flags, JSON shapes and error codes mirror eve-dogma-rs (`calc`, `batch`, `serve-stdio`, `eft`, `search`,
`type`, `meta`, `bench`). Output keys are sorted, floats rounded to 1e-6, non-finite → null.

## Licence
LGPL-3.0-or-later (algorithms for RAH / capsim follow Pyfa eos, LGPL). EVE data © CCP hf.
