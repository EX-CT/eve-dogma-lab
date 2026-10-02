// Package dogma is a stateless EVE Online fitting engine (EX-CT variant C, Go).
//
// Calc(dataset, request) is a pure function: no I/O, clocks or globals. The Dataset is immutable after
// loading and may be shared by any number of goroutines.
package dogma

import (
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"runtime"
	"runtime/debug"
	"slices"
	"sort"
	"strconv"
	"strings"
	"sync"
)

// Modifier functions (dataset codes).
const (
	FuncItem = iota
	FuncLocation
	FuncLocationGroup
	FuncLocationRequiredSkill
	FuncOwnerRequiredSkill
	FuncEffectStopper
)

// Modifier domains (dataset codes).
const (
	DomItem = iota
	DomShip
	DomChar
	DomOther
	DomStructure
	DomTargetID
	DomTarget
	DomNone = -1
)

type AttrInfo struct {
	ID         uint32
	Name       string
	Default    float64
	Stackable  bool
	HighIsGood bool
	MinAttr    uint32 // 0 = none
	MaxAttr    uint32
	Unit       uint32
	Display    string
	round2     bool // cpu/power values are rounded to 0.01
}

type Mod struct {
	Func      int8
	Domain    int8
	Op        int32
	Modified  uint32
	Modifying uint32
	Extra     uint32 // group id or skill type id
}

type EffectInfo struct {
	ID             uint32
	Name           string
	Category       uint8
	DurationAttr   uint32
	DischargeAttr  uint32
	RangeAttr      uint32
	FalloffAttr    uint32
	TrackingAttr   uint32
	ResistanceAttr uint32
	FittingChance  uint32
	IsOffensive    bool
	IsAssistance   bool
	Mods           []Mod
}

type TypeEffect struct {
	ID      uint32
	Default bool
}

// attrSet is a sorted (id, value) list; lookups are binary searches. Shared, never copied per item.
type attrSet struct {
	ids  []uint32
	vals []float64
}

func (s *attrSet) get(id uint32) (float64, bool) {
	// branchless lower bound: attribute ids are effectively random per lookup,
	// so a branchy binary search mispredicts about half its steps
	ids := s.ids
	n := len(ids)
	if n == 0 {
		return 0, false
	}
	base := 0
	for n > 1 {
		half := n >> 1
		// mask is all ones when ids[base+half-1] < id (arithmetic select: Go does not emit CMOV here)
		mask := int((int64(ids[base+half-1]) - int64(id)) >> 63)
		base += half & mask
		n -= half
	}
	if ids[base] == id {
		return s.vals[base], true
	}
	return 0, false
}

func (s *attrSet) set(id uint32, v float64) {
	i := sort.Search(len(s.ids), func(i int) bool { return s.ids[i] >= id })
	if i < len(s.ids) && s.ids[i] == id {
		s.vals[i] = v
		return
	}
	s.ids = append(s.ids, 0)
	s.vals = append(s.vals, 0)
	copy(s.ids[i+1:], s.ids[i:])
	copy(s.vals[i+1:], s.vals[i:])
	s.ids[i], s.vals[i] = id, v
}

type TypeInfo struct {
	ID              uint32
	Name            string
	Group           uint32
	Category        uint32
	Published       bool
	Mass            float64
	Volume          float64
	Capacity        float64
	Radius          float64
	MarketGroup     *uint32
	MetaGroup       *uint32
	MetaLevel       *int32
	VariationParent *uint32
	raw             attrSet // dogma attributes as shipped
	base            attrSet // raw + authoritative mass/capacity/volume/radius
	Effects         []TypeEffect
	ReqSkills       []uint32 // deduplicated requiredSkill1..6
	Slot            Slot     // SlotNone if not fittable
}

// Attr returns the raw dogma attribute value of the type.
func (t *TypeInfo) Attr(id uint32) (float64, bool) { return t.raw.get(id) }

// AttrIDs lists the raw dogma attribute ids (sorted).
func (t *TypeInfo) AttrIDs() []uint32 { return t.raw.ids }

func (t *TypeInfo) HasEffect(id uint32) bool {
	for _, e := range t.Effects {
		if e.ID == id {
			return true
		}
	}
	return false
}

type GroupInfo struct {
	Name     string
	Category uint32
}

type DbuffInfo struct {
	Name          *string     `json:"name"`
	Aggregate     *string     `json:"aggregate"`
	Op            int32       `json:"op"`
	Item          []uint32    `json:"item"`
	Location      []uint32    `json:"location"`
	LocationGroup [][2]uint32 `json:"location_group"`
	LocationSkill [][2]uint32 `json:"location_skill"`
}

type MutaMapping struct {
	Inputs []uint32 `json:"inputs"`
	Output uint32   `json:"output"`
}

type MutaInfo struct {
	Attrs   map[string][2]float64 `json:"attrs"`
	Mapping []MutaMapping         `json:"mapping"`
}

type Dataset struct {
	Build        uint64
	ReleaseDate  *string
	SHA256       string
	Types        map[uint32]*TypeInfo
	Groups       map[uint32]*GroupInfo
	Categories   map[uint32]string // category id -> English name
	Attrs        map[uint32]*AttrInfo
	Effects      map[uint32]*EffectInfo
	Dbuffs       map[uint32]*DbuffInfo
	Mutaplasmids map[uint32]*MutaInfo
	NamesZh      map[uint32]string

	attrByName   map[string]uint32
	effectByName map[string]uint32
	typeByName   map[string]uint32
	// PublishedSkills is the sorted list of published skill type ids (category 16).
	PublishedSkills []uint32
	typesByGroup    map[uint32][]uint32
	nameIdx         sync.Once
	modeTypes       []uint32 // tactical destroyer modes (group 1306), ascending
	modeNames       []string // their lower-cased names
	ids             wellKnown
	canFitGroupA    []uint32 // canFitShipGroup01..20 attribute ids present in the dataset
	canFitTypeA     []uint32 // canFitShipType1..11

	maxAttr uint32 // largest attribute id (dense per-fit registry size)
	// dense id-indexed views of Attrs/Effects/Types/Groups for the hot paths (nil = absent)
	attrD          []*AttrInfo
	effectD        []*EffectInfo
	typeD          []*TypeInfo
	groupD         []*GroupInfo
	skillItemsOnce sync.Once
	skillItems     []Item
	tplOnce        sync.Once
	tpl            [][]amod
}

// ---- raw JSON shapes ----
type rawDs struct {
	Format        string `json:"format"`
	FormatVersion uint32 `json:"format_version"`
	Sde           struct {
		Build       uint64  `json:"build"`
		ReleaseDate *string `json:"release_date"`
	} `json:"sde"`
	Groups map[string]struct {
		Name     *string `json:"name"`
		Category uint32  `json:"category"`
	} `json:"groups"`
	Attributes map[string]struct {
		Name       string  `json:"name"`
		Default    float64 `json:"default"`
		Stackable  *bool   `json:"stackable"`
		HighIsGood *bool   `json:"high_is_good"`
		MinAttr    *uint32 `json:"min_attr"`
		MaxAttr    *uint32 `json:"max_attr"`
		Unit       *uint32 `json:"unit"`
		Display    *string `json:"display"`
	} `json:"attributes"`
	Effects map[string]struct {
		Name          string      `json:"name"`
		Category      uint8       `json:"category"`
		Duration      *uint32     `json:"duration_attr"`
		Discharge     *uint32     `json:"discharge_attr"`
		Range         *uint32     `json:"range_attr"`
		Falloff       *uint32     `json:"falloff_attr"`
		Tracking      *uint32     `json:"tracking_attr"`
		Resistance    *uint32     `json:"resistance_attr"`
		FittingChance *uint32     `json:"fitting_usage_chance_attr"`
		IsOffensive   bool        `json:"is_offensive"`
		IsAssistance  bool        `json:"is_assistance"`
		Mods          [][]float64 `json:"mods"`
	} `json:"effects"`
	Types        map[string]rawType           `json:"-"`
	Dbuffs       map[string]*DbuffInfo        `json:"dbuffs"`
	Mutaplasmids map[string]*MutaInfo         `json:"mutaplasmids"`
	Names        map[string]map[string]string `json:"names"`
	Categories   map[string]struct {
		Name *string `json:"name"`
	} `json:"categories"`
}

type rawType struct {
	Name            *string            `json:"name"`
	Group           uint32             `json:"group"`
	Category        uint32             `json:"category"`
	Published       bool               `json:"published"`
	Mass            float64            `json:"mass"`
	Volume          float64            `json:"volume"`
	Capacity        float64            `json:"capacity"`
	Radius          float64            `json:"radius"`
	MarketGroup     *uint32            `json:"market_group"`
	MetaGroup       *uint32            `json:"meta_group"`
	MetaLevel       *int32             `json:"meta_level"`
	VariationParent *uint32            `json:"variation_parent"`
	Attrs           map[string]float64 `json:"attrs"`
	Effects         [][2]uint32        `json:"effects"`
}

func u32(s string) uint32 { v, _ := strconv.ParseUint(s, 10, 32); return uint32(v) }
func optU(p *uint32) uint32 {
	if p == nil {
		return 0
	}
	return *p
}

// LoadPath loads a dataset file (gzip or plain JSON).
func LoadPath(path string) (*Dataset, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	return LoadBytes(b)
}

// LoadBytes parses dataset bytes (gzip or plain JSON).
func LoadBytes(b []byte) (*Dataset, error) {
	js := b
	if len(b) > 2 && b[0] == 0x1f && b[1] == 0x8b {
		zr, err := gzip.NewReader(bytes.NewReader(b))
		if err != nil {
			return nil, fmt.Errorf("gunzip: %w", err)
		}
		// ISIZE trailer = uncompressed size mod 2^32: read straight into a right-sized buffer
		buf := bytes.NewBuffer(make([]byte, 0, int(binary.LittleEndian.Uint32(b[len(b)-4:]))+512))
		_, err = buf.ReadFrom(zr)
		js = buf.Bytes()
		if err != nil {
			return nil, fmt.Errorf("gunzip: %w", err)
		}
	}
	// Parallel decode: the document is split into top-level sections with a cheap byte scanner,
	// the small sections are decoded by encoding/json concurrently, and the large "types" section
	// is split per type and decoded by a worker pool. sha256 runs alongside.
	defer debug.SetGCPercent(debug.SetGCPercent(400))
	var sum [32]byte
	var wg sync.WaitGroup
	wg.Add(1)
	go func() { defer wg.Done(); sum = sha256.Sum256(js) }()
	top, err := splitObject(js)
	if err != nil {
		return nil, fmt.Errorf("dataset json: %w", err)
	}
	var raw rawDs
	var typesRaw []byte
	other := []byte{'{'}
	for _, kv := range top {
		if string(kv.k) == "types" {
			typesRaw = kv.v
			continue
		}
		if len(other) > 1 {
			other = append(other, ',')
		}
		other = append(other, '"')
		other = append(other, kv.k...)
		other = append(other, '"', ':')
		other = append(other, kv.v...)
	}
	other = append(other, '}')
	var errOther, errTypes error
	wg.Add(1)
	go func() { defer wg.Done(); errOther = json.Unmarshal(other, &raw) }()
	var tkv []kvRaw
	var tinfos []*TypeInfo
	if typesRaw != nil {
		if tkv, errTypes = splitObject(typesRaw); errTypes == nil {
			tinfos = make([]*TypeInfo, len(tkv))
			errs := make([]error, runtime.GOMAXPROCS(0))
			var tw sync.WaitGroup
			for w := range errs {
				tw.Add(1)
				go func(w int) {
					defer tw.Done()
					for i := w; i < len(tkv); i += len(errs) {
						var rt rawType
						if err := json.Unmarshal(tkv[i].v, &rt); err != nil {
							if errs[w] == nil {
								errs[w] = err
							}
							continue
						}
						tinfos[i] = buildType(u32(string(tkv[i].k)), &rt)
					}
				}(w)
			}
			tw.Wait()
			errTypes = errors.Join(errs...)
		}
	}
	wg.Wait()
	if err := errors.Join(errOther, errTypes); err != nil {
		return nil, fmt.Errorf("dataset json: %w", err)
	}
	if raw.Format != "exct-eve-dataset" || raw.FormatVersion != 1 {
		return nil, fmt.Errorf("unsupported dataset format %s v%d", raw.Format, raw.FormatVersion)
	}
	ds := &Dataset{
		Build: raw.Sde.Build, ReleaseDate: raw.Sde.ReleaseDate, SHA256: hex.EncodeToString(sum[:]),
		Types: make(map[uint32]*TypeInfo, len(raw.Types)), Groups: map[uint32]*GroupInfo{},
		Attrs: make(map[uint32]*AttrInfo, len(raw.Attributes)), Effects: make(map[uint32]*EffectInfo, len(raw.Effects)),
		Dbuffs: map[uint32]*DbuffInfo{}, Mutaplasmids: map[uint32]*MutaInfo{}, NamesZh: map[uint32]string{},
		attrByName: map[string]uint32{}, effectByName: map[string]uint32{}, typeByName: map[string]uint32{},
		typesByGroup: map[uint32][]uint32{},
	}
	for k, a := range raw.Attributes {
		id := u32(k)
		ai := &AttrInfo{ID: id, Name: a.Name, Default: a.Default, Stackable: true, HighIsGood: true,
			MinAttr: optU(a.MinAttr), MaxAttr: optU(a.MaxAttr), Unit: optU(a.Unit)}
		if a.Stackable != nil {
			ai.Stackable = *a.Stackable
		}
		if a.HighIsGood != nil {
			ai.HighIsGood = *a.HighIsGood
		}
		if a.Display != nil {
			ai.Display = *a.Display
		}
		ds.Attrs[id] = ai
	}
	for k, e := range raw.Effects {
		id := u32(k)
		ei := &EffectInfo{ID: id, Name: e.Name, Category: e.Category, DurationAttr: optU(e.Duration),
			DischargeAttr: optU(e.Discharge), RangeAttr: optU(e.Range), FalloffAttr: optU(e.Falloff),
			TrackingAttr: optU(e.Tracking), ResistanceAttr: optU(e.Resistance), FittingChance: optU(e.FittingChance),
			IsOffensive: e.IsOffensive, IsAssistance: e.IsAssistance}
		for _, m := range e.Mods {
			if len(m) < 6 {
				continue
			}
			ei.Mods = append(ei.Mods, Mod{Func: int8(m[0]), Domain: int8(m[1]), Modified: uint32(m[2]),
				Modifying: uint32(m[3]), Op: int32(m[4]), Extra: uint32(m[5])})
		}
		ds.Effects[id] = ei
	}
	ds.Categories = make(map[uint32]string, len(raw.Categories))
	for k, c := range raw.Categories {
		if c.Name != nil {
			ds.Categories[u32(k)] = *c.Name
		} else {
			ds.Categories[u32(k)] = ""
		}
	}
	for k, g := range raw.Groups {
		gi := &GroupInfo{Category: g.Category}
		if g.Name != nil {
			gi.Name = *g.Name
		}
		ds.Groups[u32(k)] = gi
	}
	for _, ti := range tinfos {
		ds.Types[ti.ID] = ti
	}
	for k, v := range raw.Dbuffs {
		ds.Dbuffs[u32(k)] = v
	}
	for k, v := range raw.Mutaplasmids {
		ds.Mutaplasmids[u32(k)] = v
	}
	for k, v := range raw.Names["zh"] {
		ds.NamesZh[u32(k)] = v
	}
	ds.index()
	return ds, nil
}

// index builds every derived lookup (names, groups, skills, dense views, well-known ids) from the
// primary tables. Shared by the JSON loader and the binary cache loader.
func (ds *Dataset) index() {
	ds.attrByName = make(map[string]uint32, len(ds.Attrs))
	ds.effectByName = make(map[string]uint32, len(ds.Effects))
	for id, ai := range ds.Attrs {
		switch ai.Name {
		case "cpu", "power", "cpuOutput", "powerOutput":
			ai.round2 = true
		}
		ds.maxAttr = max(ds.maxAttr, id)
		ds.attrByName[ai.Name] = id
	}
	for id, ei := range ds.Effects {
		ds.effectByName[ei.Name] = id
	}
	var skills []uint32
	ds.modeTypes, ds.modeNames = nil, nil
	for id, ti := range ds.Types {
		if ti.Category == 16 && ti.Published {
			skills = append(skills, id)
		}
		if ti.Group == 1306 {
			ds.modeTypes = append(ds.modeTypes, id)
		}
	}
	slices.Sort(ds.modeTypes)
	for _, id := range ds.modeTypes {
		ds.modeNames = append(ds.modeNames, strings.ToLower(ds.Types[id].Name))
	}
	sort.Slice(skills, func(i, j int) bool { return skills[i] < skills[j] })
	ds.PublishedSkills = skills
	ds.attrD, ds.effectD, ds.typeD, ds.groupD = dense(ds.Attrs), dense(ds.Effects), dense(ds.Types), dense(ds.Groups)
	ds.ids = newWellKnown(ds)
	ds.canFitGroupA, ds.canFitTypeA = nil, nil
	for _, n := range canFitGroupNames {
		if a := ds.AttrID(n); a != 0 {
			ds.canFitGroupA = append(ds.canFitGroupA, a)
		}
	}
	for _, n := range canFitTypeNames {
		if a := ds.AttrID(n); a != 0 {
			ds.canFitTypeA = append(ds.canFitTypeA, a)
		}
	}
}

// betterNamed decides which of two same-named types wins the name lookup: published first, then lowest id.
func betterNamed(a, b *TypeInfo) bool {
	if a.Published != b.Published {
		return a.Published
	}
	return a.ID < b.ID
}

func appendUnique(l []uint32, v uint32) []uint32 {
	for _, x := range l {
		if x == v {
			return l
		}
	}
	return append(l, v)
}

// AttrID returns the attribute id for a name (0 if unknown).
func (ds *Dataset) AttrID(name string) uint32 { return ds.attrByName[name] }

// EffectID returns the effect id for a name (0 if unknown).
func (ds *Dataset) EffectID(name string) uint32 { return ds.effectByName[name] }

// TypeByName finds a type by (case-insensitive) English name.
func (ds *Dataset) TypeByName(name string) (uint32, bool) {
	ds.buildNameIndex()
	id, ok := ds.typeByName[strings.ToLower(strings.TrimSpace(name))]
	return id, ok
}

func (ds *Dataset) AttrDefault(id uint32) float64 {
	if a := ds.attr(id); a != nil {
		return a.Default
	}
	return 0
}

// TypesInGroup returns type ids of a group (sorted).
func (ds *Dataset) TypesInGroup(g uint32) []uint32 { ds.buildNameIndex(); return ds.typesByGroup[g] }

// buildNameIndex builds the name -> type and group -> types lookups on first use (saves ~8 ms of startup).
func (ds *Dataset) buildNameIndex() {
	ds.nameIdx.Do(func() {
		byName := make(map[string]uint32, len(ds.Types))
		byGroup := map[uint32][]uint32{}
		for id, ti := range ds.Types {
			lname := strings.ToLower(ti.Name)
			if prev, ok := byName[lname]; !ok || betterNamed(ti, ds.Types[prev]) {
				byName[lname] = id
			}
			byGroup[ti.Group] = append(byGroup[ti.Group], id)
		}
		for _, l := range byGroup {
			sort.Slice(l, func(i, j int) bool { return l[i] < l[j] })
		}
		ds.typeByName, ds.typesByGroup = byName, byGroup
	})
}

func inferSlot(t *TypeInfo) Slot {
	for _, e := range t.Effects {
		switch e.ID {
		case 12:
			return SlotHigh
		case 13:
			return SlotMid
		case 11:
			return SlotLow
		case 2663:
			return SlotRig
		case 3772:
			return SlotSubsystem
		case 6306:
			return SlotService
		}
	}
	return SlotNone
}

type kvRaw struct{ k, v []byte }

// splitObject splits a JSON object into its top-level key/raw-value pairs without decoding the
// values (values are validated later by encoding/json). Keys must not contain escapes.
func splitObject(b []byte) ([]kvRaw, error) {
	i, n := 0, len(b)
	ws := func() {
		for i < n && (b[i] == ' ' || b[i] == '\n' || b[i] == '\r' || b[i] == '\t') {
			i++
		}
	}
	bad := func(what string) error { return fmt.Errorf("offset %d: %s", i, what) }
	ws()
	if i >= n || b[i] != '{' {
		return nil, bad("expected object")
	}
	i++
	var out []kvRaw
	for {
		ws()
		if i < n && b[i] == '}' && len(out) == 0 {
			return out, nil
		}
		if i >= n || b[i] != '"' {
			return nil, bad("expected key")
		}
		ks := i + 1
		for i++; i < n && b[i] != '"'; i++ {
			if b[i] == '\\' {
				return nil, bad("escaped key")
			}
		}
		if i >= n {
			return nil, bad("unterminated key")
		}
		k := b[ks:i]
		i++
		ws()
		if i >= n || b[i] != ':' {
			return nil, bad("expected ':'")
		}
		i++
		ws()
		vs, depth := i, 0
	scan:
		for ; i < n; i++ {
			switch b[i] {
			case '"':
				for i++; i < n && b[i] != '"'; i++ {
					if b[i] == '\\' {
						i++
					}
				}
			case '{', '[':
				depth++
			case '}', ']':
				if depth == 0 {
					break scan
				}
				depth--
			case ',':
				if depth == 0 {
					break scan
				}
			}
		}
		if i >= n {
			return nil, bad("unterminated value")
		}
		ve := i
		for ve > vs && (b[ve-1] == ' ' || b[ve-1] == '\n' || b[ve-1] == '\r' || b[ve-1] == '\t') {
			ve--
		}
		out = append(out, kvRaw{k, b[vs:ve]})
		if b[i] == '}' {
			return out, nil
		}
		i++
	}
}

var reqSkillAttrs = [6]uint32{182, 183, 184, 1285, 1289, 1290}

// buildType converts one decoded type record (runs on loader worker goroutines).
func buildType(id uint32, t *rawType) *TypeInfo {
	ti := &TypeInfo{ID: id, Group: t.Group, Category: t.Category, Published: t.Published, Mass: t.Mass,
		Volume: t.Volume, Capacity: t.Capacity, Radius: t.Radius, MarketGroup: t.MarketGroup,
		MetaGroup: t.MetaGroup, MetaLevel: t.MetaLevel, VariationParent: t.VariationParent}
	if t.Name != nil {
		ti.Name = *t.Name
	}
	ids := make([]uint32, 0, len(t.Attrs))
	for a := range t.Attrs {
		ids = append(ids, u32(a))
	}
	sort.Slice(ids, func(i, j int) bool { return ids[i] < ids[j] })
	vals := make([]float64, len(ids))
	for i, a := range ids {
		vals[i] = t.Attrs[strconv.FormatUint(uint64(a), 10)]
	}
	ti.raw = attrSet{ids, vals}
	for _, e := range t.Effects {
		ti.Effects = append(ti.Effects, TypeEffect{ID: e[0], Default: e[1] != 0})
	}
	ti.derive()
	return ti
}

// derive computes base attributes (raw + authoritative mass/capacity/volume/radius), required skills and
// slot from the raw record (JSON loader and cache loader).
func (ti *TypeInfo) derive() { ti.deriveA(nil) }

// slab hands out sub-slices of large shared backing arrays (one allocation per 64K entries instead of
// two per type) for the dataset's immutable attribute tables. Each piece has its own fixed capacity.
type slab struct {
	u []uint32
	f []float64
}

func (s *slab) take(n, c int) ([]uint32, []float64) {
	if s == nil {
		return make([]uint32, n, c), make([]float64, n, c)
	}
	if cap(s.u)-len(s.u) < c {
		s.u, s.f = make([]uint32, 0, max(c, 1<<16)), make([]float64, 0, max(c, 1<<16))
	}
	l := len(s.u)
	s.u, s.f = s.u[:l+c], s.f[:l+c]
	return s.u[l : l+n : l+c], s.f[l : l+n : l+c]
}

func (ti *TypeInfo) deriveA(sl *slab) {
	ids, vals := ti.raw.ids, ti.raw.vals
	bi, bv := sl.take(len(ids), len(ids)+4)
	copy(bi, ids)
	copy(bv, vals)
	ti.base = attrSet{bi, bv}
	for _, f := range [4]struct {
		a uint32
		v float64
	}{{4, ti.Mass}, {38, ti.Capacity}, {161, ti.Volume}, {162, ti.Radius}} {
		if _, ok := ti.base.get(f.a); f.v != 0 || !ok {
			ti.base.set(f.a, f.v)
		}
	}
	ti.ReqSkills = nil
	for _, a := range reqSkillAttrs {
		if v, ok := ti.raw.get(a); ok && uint32(v) != 0 {
			ti.ReqSkills = appendUnique(ti.ReqSkills, uint32(v))
		}
	}
	ti.Slot = inferSlot(ti)
}

func dense[T any](m map[uint32]*T) []*T {
	n := uint32(0)
	for k := range m {
		n = max(n, k+1)
	}
	d := make([]*T, n)
	for k, v := range m {
		d[k] = v
	}
	return d
}

func at[T any](d []*T, id uint32) *T {
	if int(id) < len(d) {
		return d[id]
	}
	return nil
}

// attr / effect / typ / group: dense lookups equivalent to the maps (nil when absent).
func (ds *Dataset) attr(id uint32) *AttrInfo     { return at(ds.attrD, id) }
func (ds *Dataset) effect(id uint32) *EffectInfo { return at(ds.effectD, id) }
func (ds *Dataset) typ(id uint32) *TypeInfo      { return at(ds.typeD, id) }
func (ds *Dataset) group(id uint32) *GroupInfo   { return at(ds.groupD, id) }
