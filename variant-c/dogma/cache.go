package dogma

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"fmt"
	"hash/crc32"
	"math"
	"os"
	"path/filepath"
	"runtime/debug"
	"sort"
	"unsafe"
)

// ---- derived binary cache ----
//
// Parsing the 7 MB dataset JSON dominates process start-up (~125 ms even when parallel). The cache stores
// the primary tables (types, attributes, effects, groups, names; buffs and mutaplasmids as their JSON
// sections) in a flat little-endian format keyed by the sha256 of the dataset file, so later processes
// start in ~20 ms. It is purely derived: delete it at any time, or disable it with EVE_DOGMA_CACHE=off.
// Location: $EVE_DOGMA_CACHE_DIR, else <user cache dir>/eve-dogma-go.

const cacheMagic = "EXCTDGC\x00"
const cacheVersion = 4 // 2: categories; 3: crc32c trailer; 4: binary dbuffs/mutaplasmids

// LoadPathCached loads a dataset file, using (and refreshing) the binary cache when enabled.
func LoadPathCached(path string) (*Dataset, error) {
	// the dataset is built once and lives for the whole process: collecting while it is being built only
	// rescans live data, so the collector is paused during loading (restored afterwards)
	defer debug.SetGCPercent(debug.SetGCPercent(-1))
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	dir := cacheDir()
	if dir == "" {
		return LoadBytes(b)
	}
	key := sha256.Sum256(b)
	file := filepath.Join(dir, hex.EncodeToString(key[:12])+".bin")
	if cb, err := os.ReadFile(file); err == nil {
		if ds, err := decodeCache(cb, key); err == nil {
			return ds, nil
		}
	}
	ds, err := LoadBytes(b)
	if err != nil {
		return nil, err
	}
	_ = writeFileAtomic(dir, file, encodeCache(ds, key)) // best effort
	return ds, nil
}

func cacheDir() string {
	if v := os.Getenv("EVE_DOGMA_CACHE"); v == "off" || v == "0" || v == "false" {
		return ""
	}
	if d := os.Getenv("EVE_DOGMA_CACHE_DIR"); d != "" {
		return d
	}
	d, err := os.UserCacheDir()
	if err != nil {
		return ""
	}
	return filepath.Join(d, "eve-dogma-go")
}

func writeFileAtomic(dir, file string, data []byte) error {
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, ".tmp-*")
	if err != nil {
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		os.Remove(tmp.Name())
		return err
	}
	if err := tmp.Close(); err != nil {
		os.Remove(tmp.Name())
		return err
	}
	return os.Rename(tmp.Name(), file) // atomic: concurrent processes see the old or the new file, never a partial one
}

// ---- encoding ----

type cw struct{ b []byte }

func (w *cw) u8(v uint8)    { w.b = append(w.b, v) }
func (w *cw) u32(v uint32)  { w.b = binary.LittleEndian.AppendUint32(w.b, v) }
func (w *cw) f64(v float64) { w.b = binary.LittleEndian.AppendUint64(w.b, math.Float64bits(v)) }
func (w *cw) str(s string)  { w.u32(uint32(len(s))); w.b = append(w.b, s...) }
func (w *cw) bool(v bool) {
	if v {
		w.u8(1)
	} else {
		w.u8(0)
	}
}
func (w *cw) optU32(p *uint32) {
	if p == nil {
		w.u8(0)
		return
	}
	w.u8(1)
	w.u32(*p)
}
func (w *cw) blob(b []byte) { w.u32(uint32(len(b))); w.b = append(w.b, b...) }

func sortedKeys[T any](m map[uint32]T) []uint32 {
	ks := make([]uint32, 0, len(m))
	for k := range m {
		ks = append(ks, k)
	}
	sort.Slice(ks, func(i, j int) bool { return ks[i] < ks[j] })
	return ks
}

func encodeCache(ds *Dataset, key [32]byte) []byte {
	w := &cw{b: make([]byte, 0, 8<<20)}
	w.b = append(w.b, cacheMagic...)
	w.u32(cacheVersion)
	w.b = append(w.b, key[:]...)
	w.u32(uint32(ds.Build >> 32))
	w.u32(uint32(ds.Build))
	if ds.ReleaseDate != nil {
		w.u8(1)
		w.str(*ds.ReleaseDate)
	} else {
		w.u8(0)
	}
	w.str(ds.SHA256)

	ks := sortedKeys(ds.Attrs)
	w.u32(uint32(len(ks)))
	for _, k := range ks {
		a := ds.Attrs[k]
		w.u32(a.ID)
		w.str(a.Name)
		w.f64(a.Default)
		w.bool(a.Stackable)
		w.bool(a.HighIsGood)
		w.u32(a.MinAttr)
		w.u32(a.MaxAttr)
		w.u32(a.Unit)
		w.str(a.Display)
	}
	ks = sortedKeys(ds.Effects)
	w.u32(uint32(len(ks)))
	for _, k := range ks {
		e := ds.Effects[k]
		w.u32(e.ID)
		w.str(e.Name)
		w.u8(e.Category)
		for _, v := range [7]uint32{e.DurationAttr, e.DischargeAttr, e.RangeAttr, e.FalloffAttr, e.TrackingAttr, e.ResistanceAttr, e.FittingChance} {
			w.u32(v)
		}
		w.bool(e.IsOffensive)
		w.bool(e.IsAssistance)
		w.u32(uint32(len(e.Mods)))
		for _, m := range e.Mods {
			w.u8(uint8(m.Func))
			w.u8(uint8(m.Domain))
			w.u32(uint32(m.Op))
			w.u32(m.Modified)
			w.u32(m.Modifying)
			w.u32(m.Extra)
		}
	}
	ks = sortedKeys(ds.Groups)
	w.u32(uint32(len(ks)))
	for _, k := range ks {
		w.u32(k)
		w.str(ds.Groups[k].Name)
		w.u32(ds.Groups[k].Category)
	}
	ks = sortedKeys(ds.Types)
	w.u32(uint32(len(ks)))
	for _, k := range ks {
		t := ds.Types[k]
		w.u32(t.ID)
		w.str(t.Name)
		w.u32(t.Group)
		w.u32(t.Category)
		w.bool(t.Published)
		w.f64(t.Mass)
		w.f64(t.Volume)
		w.f64(t.Capacity)
		w.f64(t.Radius)
		w.optU32(t.MarketGroup)
		w.optU32(t.MetaGroup)
		if t.MetaLevel != nil {
			w.u8(1)
			w.u32(uint32(*t.MetaLevel))
		} else {
			w.u8(0)
		}
		w.optU32(t.VariationParent)
		w.u32(uint32(len(t.raw.ids)))
		for i, a := range t.raw.ids {
			w.u32(a)
			w.f64(t.raw.vals[i])
		}
		w.u32(uint32(len(t.Effects)))
		for _, e := range t.Effects {
			w.u32(e.ID)
			w.bool(e.Default)
		}
	}
	ks = sortedKeys(ds.NamesZh)
	w.u32(uint32(len(ks)))
	for _, k := range ks {
		w.u32(k)
		w.str(ds.NamesZh[k])
	}
	ks = sortedKeys(ds.Categories)
	w.u32(uint32(len(ks)))
	for _, k := range ks {
		w.u32(k)
		w.str(ds.Categories[k])
	}
	ks = sortedKeys(ds.Dbuffs)
	w.u32(uint32(len(ks)))
	for _, k := range ks {
		d := ds.Dbuffs[k]
		w.u32(k)
		w.optStr(d.Name)
		w.optStr(d.Aggregate)
		w.u32(uint32(d.Op))
		w.u32s(d.Item)
		w.u32s(d.Location)
		w.pairs(d.LocationGroup)
		w.pairs(d.LocationSkill)
	}
	ks = sortedKeys(ds.Mutaplasmids)
	w.u32(uint32(len(ks)))
	for _, k := range ks {
		m := ds.Mutaplasmids[k]
		w.u32(k)
		ak := make([]string, 0, len(m.Attrs))
		for a := range m.Attrs {
			ak = append(ak, a)
		}
		sort.Strings(ak)
		w.u32(uint32(len(ak)))
		for _, a := range ak {
			w.str(a)
			w.f64(m.Attrs[a][0])
			w.f64(m.Attrs[a][1])
		}
		w.u32(uint32(len(m.Mapping)))
		for _, mp := range m.Mapping {
			w.u32s(mp.Inputs)
			w.u32(mp.Output)
		}
	}
	sum := trailerSum(w.b)
	w.b = append(w.b, sum[:]...) // integrity trailer
	return w.b
}

// ---- decoding ----

type cr struct {
	b   []byte
	s   string // the same bytes as one string: names are substrings of it (no per-string allocation)
	off int
	err bool
}

func (r *cr) need(n int) bool {
	if r.err || r.off+n > len(r.b) {
		r.err = true
		return false
	}
	return true
}
func (r *cr) u8() uint8 {
	if !r.need(1) {
		return 0
	}
	r.off++
	return r.b[r.off-1]
}
func (r *cr) u32() uint32 {
	if !r.need(4) {
		return 0
	}
	r.off += 4
	return binary.LittleEndian.Uint32(r.b[r.off-4:])
}
func (r *cr) f64() float64 {
	if !r.need(8) {
		return 0
	}
	r.off += 8
	return math.Float64frombits(binary.LittleEndian.Uint64(r.b[r.off-8:]))
}
func (r *cr) str() string {
	n := int(r.u32())
	if !r.need(n) {
		return ""
	}
	r.off += n
	return r.s[r.off-n : r.off]
}
func (r *cr) bool() bool { return r.u8() != 0 }
func (r *cr) optU32() *uint32 {
	if r.u8() == 0 {
		return nil
	}
	v := r.u32()
	return &v
}
func (r *cr) blob() []byte {
	n := int(r.u32())
	if !r.need(n) {
		return nil
	}
	r.off += n
	return r.b[r.off-n : r.off]
}
func (r *cr) count(elemMin int) int {
	n := int(r.u32())
	if n < 0 || n*elemMin > len(r.b)-r.off {
		r.err = true
		return 0
	}
	return n
}

var errCache = errors.New("dataset cache: invalid or stale")

func decodeCache(b []byte, key [32]byte) (*Dataset, error) {
	hdr := len(cacheMagic) + 4 + 32
	if len(b) < hdr+32 || string(b[:len(cacheMagic)]) != cacheMagic ||
		binary.LittleEndian.Uint32(b[len(cacheMagic):]) != cacheVersion || !bytes.Equal(b[len(cacheMagic)+4:hdr], key[:]) {
		return nil, errCache
	}
	body := b[:len(b)-32]
	if sum := trailerSum(body); !bytes.Equal(sum[:], b[len(b)-32:]) {
		return nil, errCache
	}
	// zero-copy: names are substrings of the cache bytes themselves. decodeCache takes ownership of b,
	// which must never be modified afterwards (LoadPathCached reads it fresh from disk).
	r := &cr{b: body, s: unsafe.String(unsafe.SliceData(body), len(body)), off: hdr}
	ds := &Dataset{}
	ds.Build = uint64(r.u32())<<32 | uint64(r.u32())
	if r.u8() == 1 {
		s := r.str()
		ds.ReleaseDate = &s
	}
	ds.SHA256 = r.str()

	n := r.count(30)
	ds.Attrs = make(map[uint32]*AttrInfo, n)
	attrs := make([]AttrInfo, n)
	for i := range attrs {
		a := &attrs[i]
		a.ID, a.Name, a.Default = r.u32(), r.str(), r.f64()
		a.Stackable, a.HighIsGood = r.bool(), r.bool()
		a.MinAttr, a.MaxAttr, a.Unit = r.u32(), r.u32(), r.u32()
		a.Display = r.str()
		ds.Attrs[a.ID] = a
	}
	n = r.count(40)
	ds.Effects = make(map[uint32]*EffectInfo, n)
	effs := make([]EffectInfo, n)
	for i := range effs {
		e := &effs[i]
		e.ID, e.Name, e.Category = r.u32(), r.str(), r.u8()
		e.DurationAttr, e.DischargeAttr, e.RangeAttr, e.FalloffAttr = r.u32(), r.u32(), r.u32(), r.u32()
		e.TrackingAttr, e.ResistanceAttr, e.FittingChance = r.u32(), r.u32(), r.u32()
		e.IsOffensive, e.IsAssistance = r.bool(), r.bool()
		if m := r.count(18); m > 0 {
			e.Mods = make([]Mod, m)
			for k := range e.Mods {
				e.Mods[k] = Mod{Func: int8(r.u8()), Domain: int8(r.u8()), Op: int32(r.u32()), Modified: r.u32(), Modifying: r.u32(), Extra: r.u32()}
			}
		}
		ds.Effects[e.ID] = e
	}
	n = r.count(12)
	ds.Groups = make(map[uint32]*GroupInfo, n)
	groups := make([]GroupInfo, n)
	for i := range groups {
		id := r.u32()
		groups[i] = GroupInfo{Name: r.str(), Category: r.u32()}
		ds.Groups[id] = &groups[i]
	}
	n = r.count(60)
	ds.Types = make(map[uint32]*TypeInfo, n)
	types := make([]TypeInfo, n)
	var sl slab
	for i := range types {
		t := &types[i]
		t.ID, t.Name, t.Group, t.Category, t.Published = r.u32(), r.str(), r.u32(), r.u32(), r.bool()
		t.Mass, t.Volume, t.Capacity, t.Radius = r.f64(), r.f64(), r.f64(), r.f64()
		t.MarketGroup, t.MetaGroup = r.optU32(), r.optU32()
		if r.u8() == 1 {
			v := int32(r.u32())
			t.MetaLevel = &v
		}
		t.VariationParent = r.optU32()
		na := r.count(12)
		ri, rv := sl.take(na, na)
		t.raw = attrSet{ri, rv}
		for k := 0; k < na; k++ {
			t.raw.ids[k], t.raw.vals[k] = r.u32(), r.f64()
		}
		if ne := r.count(5); ne > 0 {
			t.Effects = make([]TypeEffect, ne)
			for k := range t.Effects {
				t.Effects[k] = TypeEffect{ID: r.u32(), Default: r.bool()}
			}
		}
		if r.err {
			return nil, errCache
		}
		t.deriveA(&sl)
		ds.Types[t.ID] = t
	}
	n = r.count(8)
	ds.NamesZh = make(map[uint32]string, n)
	for i := 0; i < n; i++ {
		id := r.u32()
		ds.NamesZh[id] = r.str()
	}
	n = r.count(8)
	ds.Categories = make(map[uint32]string, n)
	for i := 0; i < n; i++ {
		id := r.u32()
		ds.Categories[id] = r.str()
	}
	n = r.count(4)
	ds.Dbuffs = make(map[uint32]*DbuffInfo, n)
	for i := 0; i < n && !r.err; i++ {
		id := r.u32()
		d := &DbuffInfo{Name: r.optStr(), Aggregate: r.optStr(), Op: int32(r.u32())}
		d.Item, d.Location = r.u32s(), r.u32s()
		d.LocationGroup, d.LocationSkill = r.pairs(), r.pairs()
		ds.Dbuffs[id] = d
	}
	n = r.count(4)
	ds.Mutaplasmids = make(map[uint32]*MutaInfo, n)
	for i := 0; i < n && !r.err; i++ {
		id := r.u32()
		m := &MutaInfo{}
		na := r.count(20)
		m.Attrs = make(map[string][2]float64, na)
		for j := 0; j < na && !r.err; j++ {
			a := r.str()
			m.Attrs[a] = [2]float64{r.f64(), r.f64()}
		}
		nm := r.count(8)
		for j := 0; j < nm && !r.err; j++ {
			m.Mapping = append(m.Mapping, MutaMapping{Inputs: r.u32s(), Output: r.u32()})
		}
		ds.Mutaplasmids[id] = m
	}
	if r.err || r.off != len(body) {
		return nil, errCache
	}
	ds.index()
	return ds, nil
}

var crcTable = crc32.MakeTable(crc32.Castagnoli)

// trailerSum is the 32-byte integrity trailer: CRC-32C (hardware accelerated, ~10x cheaper than sha256 on
// the 5 MB cache; it guards against truncation/corruption, the key already binds the dataset content).
func trailerSum(b []byte) [32]byte {
	var t [32]byte
	binary.LittleEndian.PutUint32(t[:], crc32.Checksum(b, crcTable))
	binary.LittleEndian.PutUint64(t[4:], uint64(len(b)))
	return t
}

func (w *cw) optStr(p *string) {
	if p == nil {
		w.u8(0)
		return
	}
	w.u8(1)
	w.str(*p)
}

func (w *cw) u32s(v []uint32) {
	if v == nil {
		w.u32(math.MaxUint32)
		return
	}
	w.u32(uint32(len(v)))
	for _, x := range v {
		w.u32(x)
	}
}

func (w *cw) pairs(v [][2]uint32) {
	if v == nil {
		w.u32(math.MaxUint32)
		return
	}
	w.u32(uint32(len(v)))
	for _, x := range v {
		w.u32(x[0])
		w.u32(x[1])
	}
}

func (r *cr) optStr() *string {
	if r.u8() == 0 {
		return nil
	}
	s := r.str()
	return &s
}

// lenOrNil reads a length where MaxUint32 encodes a nil slice.
func (r *cr) lenOrNil(elem int) (int, bool) {
	if r.need(4) && binary.LittleEndian.Uint32(r.b[r.off:]) == math.MaxUint32 {
		r.off += 4
		return 0, true
	}
	return r.count(elem), false
}

func (r *cr) u32s() []uint32 {
	n, isNil := r.lenOrNil(4)
	if isNil || r.err {
		return nil
	}
	v := make([]uint32, n)
	for i := range v {
		v[i] = r.u32()
	}
	return v
}

func (r *cr) pairs() [][2]uint32 {
	n, isNil := r.lenOrNil(8)
	if isNil || r.err {
		return nil
	}
	v := make([][2]uint32, n)
	for i := range v {
		v[i] = [2]uint32{r.u32(), r.u32()}
	}
	return v
}
