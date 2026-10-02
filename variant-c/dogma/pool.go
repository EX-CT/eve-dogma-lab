package dogma

import "sync"

// ---- per-calculation working memory reuse ----
//
// A Fit owns a few large buffers (items, the dense modifier registry, the value cache, skill-level
// overlays). Build takes a Fit from a sync.Pool and Release hands it back, so a steady stream of
// calculations (batch, serve-stdio, serve-http) reuses them instead of allocating ~250 KB per fit.
// Release is optional: an unreleased Fit is simply garbage collected.

var fitPool sync.Pool

func acquireFit(ds *Dataset, nItems int) *Fit {
	f, _ := fitPool.Get().(*Fit)
	if f == nil || f.DS != ds {
		f = &Fit{}
	}
	items, reg, used, cache, stack, lvI, lvV := f.Items[:0], f.reg, f.regUsed[:0], f.cache, f.stack[:0], f.lvIDs, f.lvVals
	*f = Fit{DS: ds, Items: items, regUsed: used, stack: stack, lvIDs: lvI, lvVals: lvV}
	if cap(f.Items) < nItems {
		f.Items = make([]Item, 0, nItems)
	}
	if n := int(ds.maxAttr) + 1; len(reg) < n {
		reg = make([]attrMods, n)
	}
	f.reg = reg
	cache.reset()
	f.cache = cache
	return f
}

// Release returns the Fit's buffers for reuse by a later Build. The Fit (and any slice obtained from
// it, e.g. Items) must not be used afterwards. Values already returned by Get/ComputeStats stay valid.
func (f *Fit) Release() {
	if f == nil || f.noPool {
		return
	}
	for _, a := range f.regUsed {
		am := &f.reg[a]
		clear(am.mods) // drop pointers held by stale entries
		am.mods, am.split, am.used = am.mods[:0], 0, false
	}
	f.regUsed = f.regUsed[:0]
	clear(f.Items[:cap(f.Items)][:len(f.Items)])
	f.Items = f.Items[:0]
	f.Warnings, f.ProjSpecials, f.rdeps, f.skillTpl = nil, nil, nil, nil
	fitPool.Put(f)
}

// nodeCache is an open-addressing (linear probing) map from nodeKey to value. Compared with a Go map it
// avoids hashing overhead, keeps the table across calculations, and resets with one memclr.
type nodeCache struct {
	keys  []uint64 // 0 = empty, else key | occupied
	vals  []float64
	n     int
	shift uint
}

const occupied = 1 << 63

func (c *nodeCache) reset() {
	if c.keys == nil {
		c.init(1 << 11)
		return
	}
	if c.n > 0 {
		clear(c.keys)
		c.n = 0
	}
}

func (c *nodeCache) init(size int) {
	c.keys, c.vals, c.n = make([]uint64, size), make([]float64, size), 0
	c.shift = 64
	for s := size; s > 1; s >>= 1 {
		c.shift--
	}
}

func (c *nodeCache) slot(k uint64) int { return int((k * 0x9E3779B97F4A7C15) >> c.shift) }

func (c *nodeCache) get(k uint64) (float64, bool) {
	if c.keys == nil {
		return 0, false
	}
	k |= occupied
	mask := len(c.keys) - 1
	for i := c.slot(k); ; i = (i + 1) & mask {
		switch c.keys[i] {
		case k:
			return c.vals[i], true
		case 0:
			return 0, false
		}
	}
}

func (c *nodeCache) put(k uint64, v float64) {
	if c.keys == nil {
		c.init(1 << 11)
	}
	if 2*(c.n+1) > len(c.keys) {
		oldK, oldV := c.keys, c.vals
		c.init(2 * len(oldK))
		for i, ok := range oldK {
			if ok != 0 {
				c.put(ok&^occupied, oldV[i])
			}
		}
	}
	k |= occupied
	mask := len(c.keys) - 1
	for i := c.slot(k); ; i = (i + 1) & mask {
		switch c.keys[i] {
		case k:
			c.vals[i] = v
			return
		case 0:
			c.keys[i], c.vals[i] = k, v
			c.n++
			return
		}
	}
}

// del removes k with backward-shift deletion (no tombstones).
func (c *nodeCache) del(k uint64) {
	if c.keys == nil {
		return
	}
	k |= occupied
	mask := len(c.keys) - 1
	i := c.slot(k)
	for c.keys[i] != k {
		if c.keys[i] == 0 {
			return
		}
		i = (i + 1) & mask
	}
	for {
		c.keys[i] = 0
		j := i
		for {
			j = (j + 1) & mask
			if c.keys[j] == 0 {
				c.n--
				return
			}
			h := c.slot(c.keys[j])
			// move j back to i if its home slot h is not in the cyclic range (i, j]
			if (i <= j && (h <= i || h > j)) || (i > j && h <= i && h > j) {
				c.keys[i], c.vals[i] = c.keys[j], c.vals[j]
				i = j
				break
			}
		}
	}
}

func (c *nodeCache) clearAll() { c.reset() }
