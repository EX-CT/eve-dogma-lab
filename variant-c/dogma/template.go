package dogma

// skillTemplate returns the modifiers sourced by all published skills, indexed by modified attribute id,
// assuming the canonical layout (ship = item 0, character = item 1, skills = items 2.. in PublishedSkills
// order, non-structure ship). Skills are ~90% of every fit's modifier sources and identical across requests
// (only their level attribute differs, which is read lazily), so they are registered once per Dataset.
func (ds *Dataset) skillTemplate() [][]amod {
	ds.tplOnce.Do(func() {
		f := &Fit{DS: ds, Ship: 0, Char: 1, reg: make([]attrMods, int(ds.maxAttr)+1), noPool: true}
		ch := ds.typ(1373)
		if ch == nil {
			return
		}
		// placeholders for ship and character (never sources here)
		f.Items = append(f.Items, Item{T: ch, Parent: -1, Charge: -1, ReqIndex: -1}, Item{T: ch, Parent: -1, Charge: -1, ReqIndex: -1})
		for _, s := range ds.PublishedSkills {
			idx, err := f.newItem(s, KSkill, LChar, ipath{"", -1, ""})
			if err != nil {
				return
			}
			f.Items[idx].Owned = false
		}
		maxAttr := uint32(0)
		for i := 2; i < len(f.Items); i++ {
			f.registerItem(i)
		}
		for _, a := range f.regUsed {
			maxAttr = max(maxAttr, a)
		}
		tpl := make([][]amod, maxAttr+1)
		for _, a := range f.regUsed {
			tpl[a] = f.reg[a].mods
		}
		ds.tpl = tpl
	})
	return ds.tpl
}

// skillItemTemplate returns prebuilt Items for ds.PublishedSkills (in order, no level overlay), copied
// into every canonical fit instead of constructing ~500 items one by one.
func (ds *Dataset) skillItemTemplate() []Item {
	ds.skillItemsOnce.Do(func() {
		f := &Fit{DS: ds, noPool: true}
		for _, s := range ds.PublishedSkills {
			idx, err := f.newItem(s, KSkill, LChar, ipath{})
			if err != nil {
				return
			}
			f.Items[idx].Owned = false
		}
		ds.skillItems = f.Items
	})
	return ds.skillItems
}
