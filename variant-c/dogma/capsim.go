package dogma

import (
	"container/heap"
	"math"
	"sort"
)

// Drain is one capacitor consumer/injector for the simulation.
type Drain struct {
	Duration       float64 // ms
	CapNeed        float64 // per cycle (negative = injected)
	ClipSize       uint32  // 0 = infinite
	ReloadMs       float64
	IsInjector     bool
	DisableStagger bool
}

type CapResult struct {
	Stable     bool
	StableLow  float64
	StableHigh float64
	TS         float64
	EveStable  float64
	Iterations uint64
}

type capEv struct {
	t, duration, capNeed float64
	shot, clip           uint32
	reload               float64
	inj                  bool
	seq                  uint64
}

func evLess(a, b *capEv) bool {
	if a.t != b.t {
		return a.t < b.t
	}
	if a.duration != b.duration {
		return a.duration < b.duration
	}
	if a.capNeed != b.capNeed {
		return a.capNeed < b.capNeed
	}
	if a.shot != b.shot {
		return a.shot < b.shot
	}
	if a.clip != b.clip {
		return a.clip < b.clip
	}
	if a.reload != b.reload {
		return a.reload < b.reload
	}
	if a.inj != b.inj {
		return !a.inj
	}
	return a.seq < b.seq
}

type evHeap []capEv

func (h evHeap) Len() int           { return len(h) }
func (h evHeap) Less(i, j int) bool { return evLess(&h[i], &h[j]) }
func (h evHeap) Swap(i, j int)      { h[i], h[j] = h[j], h[i] }
func (h *evHeap) Push(x any)        { *h = append(*h, x.(capEv)) }
func (h *evHeap) Pop() any          { o := *h; n := len(o); x := o[n-1]; *h = o[:n-1]; return x }

func gcd(a, b uint64) uint64 {
	for b != 0 {
		a, b = b, a%b
	}
	return a
}

// SimulateCap is an event-driven capacitor simulation, behaviour-compatible with Pyfa eos/capSim.py.
func SimulateCap(capacity, rechargeMs float64, drains []Drain, startFrac float64, reload, stagger bool, tMaxMs float64) CapResult {
	tau := rechargeMs / 5
	h := &evHeap{}
	var seq uint64
	period := uint64(1)
	disablePeriod := false
	type grp struct {
		d Drain
		n uint32
	}
	var groups []grp
	for _, d := range drains {
		if !reload && !d.IsInjector {
			d.ClipSize = 0
			d.ReloadMs = 0
		}
		if d.Duration <= 0 {
			continue
		}
		found := false
		for gi := range groups {
			if groups[gi].d == d {
				groups[gi].n++
				found = true
				break
			}
		}
		if !found {
			groups = append(groups, grp{d, 1})
		}
	}
	for _, g := range groups {
		d, n := g.d, g.n
		if d.ClipSize > 0 {
			disablePeriod = true
		}
		if d.IsInjector {
			for k := uint32(0); k < n; k++ {
				heap.Push(h, capEv{0, d.Duration, d.CapNeed, 0, d.ClipSize, d.ReloadMs, true, seq})
				seq++
			}
			continue
		}
		if stagger && !d.DisableStagger {
			if d.ClipSize == 0 {
				d.Duration = math.Floor(d.Duration / float64(n))
			} else {
				st := (d.Duration*float64(d.ClipSize) + d.ReloadMs) / (float64(n) * float64(d.ClipSize))
				for k := uint32(1); k < n; k++ {
					heap.Push(h, capEv{float64(k) * st, d.Duration, d.CapNeed, 0, d.ClipSize, d.ReloadMs, false, seq})
					seq++
				}
			}
		} else {
			d.CapNeed *= float64(n)
		}
		dur := uint64(math.Max(math.Round(d.Duration), 1))
		period = period / gcd(period, dur) * dur
		heap.Push(h, capEv{0, d.Duration, d.CapNeed, 0, d.ClipSize, d.ReloadMs, false, seq})
		seq++
	}
	periodF := float64(period)
	if disablePeriod || periodF > tMaxMs {
		periodF = tMaxMs
	}
	capMax := capacity
	cap := capacity * startFrac
	capWrap, capLowest, capLowestPre := cap, cap, cap
	tWrap := periodF
	tLast := 0.0
	var iterations uint64
	var awaiting []capEv
	var awaitingWrap [][2]uint64
	ranOut := false
	key := func(v []capEv) [][2]uint64 {
		k := make([][2]uint64, len(v))
		for i, e := range v {
			k[i] = [2]uint64{math.Float64bits(e.duration), math.Float64bits(e.capNeed)}
		}
		sort.Slice(k, func(a, b int) bool {
			if k[a][0] != k[b][0] {
				return k[a][0] < k[b][0]
			}
			return k[a][1] < k[b][1]
		})
		return k
	}
	eqKey := func(a, b [][2]uint64) bool {
		if len(a) != len(b) {
			return false
		}
		for i := range a {
			if a[i] != b[i] {
				return false
			}
		}
		return true
	}
	var lastEv *capEv
	reschedule := func(inj capEv, tNow float64) {
		inj.t = tNow + inj.duration
		inj.shot++
		if inj.clip > 0 && inj.shot%inj.clip == 0 {
			inj.shot = 0
			inj.t += inj.reload
		}
		inj.seq = seq
		seq++
		heap.Push(h, inj)
	}
	for h.Len() > 0 {
		ev := heap.Pop(h).(capEv)
		tNow := ev.t
		if tNow >= tMaxMs {
			lastEv = &ev
			break
		}
		if tNow > tLast && capMax > 0 && tau > 0 {
			x := math.Sqrt(math.Max(cap/capMax, 0))
			y := 1 + (x-1)*math.Exp((tLast-tNow)/tau)
			cap = y * y * capMax
		}
		if tNow != tLast {
			if cap < capLowestPre {
				capLowestPre = cap
			}
			if tNow == tWrap {
				k := key(awaiting)
				if cap >= capWrap && eqKey(k, awaitingWrap) {
					lastEv = &ev
					break
				}
				capWrap = math.Round(cap*10) / 10
				awaitingWrap = k
				tWrap += periodF
			}
		}
		tLast = tNow
		iterations++
		if iterations > 5_000_000 {
			lastEv = &ev
			break
		}
		if ev.inj && cap-ev.capNeed > capMax {
			awaiting = append(awaiting, ev)
			continue
		}
		if ev.capNeed > cap && cap < capMax {
			for len(awaiting) > 0 && ev.capNeed > cap && capMax > cap {
				need := math.Min(ev.capNeed-cap, capMax-cap)
				pick := -1
				for i := range awaiting {
					if -awaiting[i].capNeed >= need && (pick < 0 || -awaiting[i].capNeed < -awaiting[pick].capNeed) {
						pick = i
					}
				}
				if pick < 0 {
					for i := range awaiting {
						if pick < 0 || -awaiting[i].capNeed >= -awaiting[pick].capNeed {
							pick = i
						}
					}
				}
				inj := awaiting[pick]
				awaiting = append(awaiting[:pick], awaiting[pick+1:]...)
				cap = math.Min(cap-inj.capNeed, capMax)
				reschedule(inj, tNow)
			}
		}
		cap = math.Min(cap-ev.capNeed, capMax)
		if cap < capLowest {
			if cap < 0 {
				ranOut = true
				lastEv = &ev
				break
			}
			capLowest = cap
		}
		for len(awaiting) > 0 && cap < capMax {
			need := capMax - cap
			pick := -1
			for i := range awaiting {
				// max_by keeps the last maximum
				if -awaiting[i].capNeed <= need && (pick < 0 || -awaiting[i].capNeed >= -awaiting[pick].capNeed) {
					pick = i
				}
			}
			if pick < 0 {
				break
			}
			inj := awaiting[pick]
			awaiting = append(awaiting[:pick], awaiting[pick+1:]...)
			cap = math.Min(cap-inj.capNeed, capMax)
			reschedule(inj, tNow)
		}
		reschedule(ev, tNow)
	}
	all := []capEv(*h)
	if lastEv != nil {
		all = append(all, *lastEv)
	}
	avgDrain := 0.0
	for _, e := range all {
		avgDrain += e.capNeed / e.duration
	}
	inner := -(2*avgDrain*tau - capMax) / capMax
	eve := 0.0
	if inner >= 0 && capMax > 0 {
		s := 1 + math.Sqrt(inner)
		eve = 0.25 * s * s
	}
	r := CapResult{Stable: !ranOut, TS: tLast / 1000, EveStable: eve, Iterations: iterations}
	if r.Stable && capMax > 0 {
		r.StableLow = capLowest / capMax
		r.StableHigh = capLowestPre / capMax
	}
	return r
}
