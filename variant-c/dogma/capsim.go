package dogma

import "math"

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
	reload               float64
	shot, clip           uint32
	seq                  uint32 // < 5M + #drains (iteration cap), so 32 bits suffice
	inj                  bool
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

// evHeap is a typed binary min-heap (no interface boxing: zero allocations per push/pop).
type evHeap []capEv

// push/pop move a "hole" instead of swapping (half the 64-byte copies of a swap-based heap).
func (h *evHeap) push(e capEv) {
	*h = append(*h, e)
	a := *h
	i := len(a) - 1
	for i > 0 {
		p := (i - 1) / 2
		if !evLess(&e, &a[p]) {
			break
		}
		a[i] = a[p]
		i = p
	}
	a[i] = e
}

func (h *evHeap) pop(top *capEv) {
	a := *h
	n := len(a) - 1
	*top = a[0]
	last := a[n]
	a = a[:n]
	i := 0
	for {
		l := 2*i + 1
		if l >= n {
			break
		}
		m := l
		if r := l + 1; r < n && evLess(&a[r], &a[l]) {
			m = r
		}
		if !evLess(&a[m], &last) {
			break
		}
		a[i] = a[m]
		i = m
	}
	if n > 0 {
		a[i] = last
	}
	*h = a
}

// pushPop is push(e) followed by pop(top) in one sift: when e precedes the current minimum it is
// returned directly without touching the heap (the common case for a periodic module).
func (h *evHeap) pushPop(e capEv, top *capEv) {
	a := *h
	n := len(a)
	if n == 0 || evLess(&e, &a[0]) {
		*top = e
		return
	}
	*top = a[0]
	i := 0
	for {
		l := 2*i + 1
		if l >= n {
			break
		}
		m := l
		if r := l + 1; r < n && evLess(&a[r], &a[l]) {
			m = r
		}
		if !evLess(&a[m], &e) {
			break
		}
		a[i] = a[m]
		i = m
	}
	a[i] = e
}

func gcd(a, b uint64) uint64 {
	for b != 0 {
		a, b = b, a%b
	}
	return a
}

// SimulateCap is an event-driven capacitor simulation, behaviour-compatible with Pyfa eos/capSim.py.
func SimulateCap(capacity, rechargeMs float64, drains []Drain, startFrac float64, reload, stagger bool, tMaxMs float64) CapResult {
	tau := rechargeMs / 5
	hh := make(evHeap, 0, 2*len(drains)+4)
	h := &hh
	var seq uint32
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
				h.push(capEv{0, d.Duration, d.CapNeed, d.ReloadMs, 0, d.ClipSize, seq, true})
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
					h.push(capEv{float64(k) * st, d.Duration, d.CapNeed, d.ReloadMs, 0, d.ClipSize, seq, false})
					seq++
				}
			}
		} else {
			d.CapNeed *= float64(n)
		}
		dur := uint64(max(math.Round(d.Duration), 1))
		period = period / gcd(period, dur) * dur
		h.push(capEv{0, d.Duration, d.CapNeed, d.ReloadMs, 0, d.ClipSize, seq, false})
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
	var keyBuf [][2]uint64
	key := func(v []capEv) [][2]uint64 {
		k := keyBuf[:0]
		for _, e := range v {
			k = append(k, [2]uint64{math.Float64bits(e.duration), math.Float64bits(e.capNeed)})
		}
		for i := 1; i < len(k); i++ {
			for j := i; j > 0 && (k[j-1][0] > k[j][0] || (k[j-1][0] == k[j][0] && k[j-1][1] > k[j][1])); j-- {
				k[j-1], k[j] = k[j], k[j-1]
			}
		}
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
	var lastEv capEv
	haveLast := false
	next := func(inj *capEv, tNow float64) {
		inj.t = tNow + inj.duration
		inj.shot++
		if inj.clip > 0 && inj.shot%inj.clip == 0 {
			inj.shot = 0
			inj.t += inj.reload
		}
		inj.seq = seq
		seq++
	}
	reschedule := func(inj capEv, tNow float64) {
		next(&inj, tNow)
		h.push(inj)
	}
	// exp((tLast-tNow)/tau) memo: event spacings repeat (periodic modules), exp is pure, so results are identical
	var expK [256]uint64
	var expV [256]float64
	expOf := func(dt float64) float64 {
		bits := math.Float64bits(dt)
		slot := (bits * 0x9E3779B97F4A7C15) >> 56
		if expK[slot] == bits && bits != 0 {
			return expV[slot]
		}
		v := math.Exp(dt / tau)
		expK[slot], expV[slot] = bits, v
		return v
	}
	var ev capEv
	// the popped event is rescheduled at the end of most iterations; its heap insertion is deferred
	// into the next iteration's pushPop (seq is still assigned in program order, so ordering is unchanged)
	pending := false
	for pending || len(*h) > 0 {
		if pending {
			h.pushPop(ev, &ev)
			pending = false
		} else {
			h.pop(&ev)
		}
		tNow := ev.t
		if tNow >= tMaxMs {
			lastEv, haveLast = ev, true
			break
		}
		if tNow > tLast && capMax > 0 && tau > 0 {
			x := math.Sqrt(max(cap/capMax, 0))
			y := 1 + (x-1)*expOf(tLast-tNow)
			cap = y * y * capMax
		}
		if tNow != tLast {
			if cap < capLowestPre {
				capLowestPre = cap
			}
			if tNow == tWrap {
				k := key(awaiting)
				if cap >= capWrap && eqKey(k, awaitingWrap) {
					lastEv, haveLast = ev, true
					break
				}
				capWrap = math.Round(cap*10) / 10
				keyBuf, awaitingWrap = awaitingWrap, k
				tWrap += periodF
			}
		}
		tLast = tNow
		iterations++
		if iterations > 5_000_000 {
			lastEv, haveLast = ev, true
			break
		}
		if ev.inj && cap-ev.capNeed > capMax {
			awaiting = append(awaiting, ev)
			continue
		}
		if ev.capNeed > cap && cap < capMax {
			for len(awaiting) > 0 && ev.capNeed > cap && capMax > cap {
				need := min(ev.capNeed-cap, capMax-cap)
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
				cap = min(cap-inj.capNeed, capMax)
				reschedule(inj, tNow)
			}
		}
		cap = min(cap-ev.capNeed, capMax)
		if cap < capLowest {
			if cap < 0 {
				ranOut = true
				lastEv, haveLast = ev, true
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
			cap = min(cap-inj.capNeed, capMax)
			reschedule(inj, tNow)
		}
		next(&ev, tNow)
		pending = true
	}
	if pending {
		h.push(ev)
	}
	all := []capEv(*h)
	if haveLast {
		all = append(all, lastEv)
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
