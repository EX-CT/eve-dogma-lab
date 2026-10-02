using EveDogmaK.Data;
using EveDogmaK.Requests;
using EveDogmaK.Rules;

namespace EveDogmaK.Engine;

/// <summary>Builds the object graph for one request, then hands it to the <see cref="RuleBook"/> for modifier registration.</summary>
public static class FitBuilder
{
    public const int CharacterTypeId = 1373;

    public static Fit Build(Dataset ds, FitRequest req)
    {
        var fit = new Fit(ds, req);
        var k = fit.K;
        fit.Ship = NewItem(fit, req.ShipTypeId, ItemKind.Ship, ItemLocation.Ship, "/ship/type_id");
        fit.IsStructure = fit[fit.Ship].Category == 65;
        fit.Char = NewItem(fit, CharacterTypeId, ItemKind.Character, ItemLocation.Character, "/character");
        if (req.SecurityStatus is double sec && !k.PilotSecurityStatus.IsNone) fit.SetBase(fit.Char, k.PilotSecurityStatus, sec);

        AddSkills(fit, req);
        AddMode(fit, req);
        for (int i = 0; i < req.Modules.Count; i++) AddModule(fit, i, req.Modules[i], $"/modules/{i}");
        for (int i = 0; i < req.Drones.Count; i++) AddDrone(fit, i, req.Drones[i]);
        for (int i = 0; i < req.Fighters.Count; i++) AddFighter(fit, i, req.Fighters[i]);
        for (int i = 0; i < req.Implants.Count; i++)
        {
            int idx = NewItem(fit, req.Implants[i], ItemKind.Implant, ItemLocation.Character, $"/implants/{i}");
            fit[idx].RequestIndex = i;
        }
        for (int i = 0; i < req.Boosters.Count; i++)
        {
            int idx = NewItem(fit, req.Boosters[i].TypeId, ItemKind.Booster, ItemLocation.Character, $"/boosters/{i}");
            fit[idx].BoosterSideEffects = req.Boosters[i].SideEffects;
            fit[idx].RequestIndex = i;
        }
        for (int i = 0; i < req.EnvironmentEffects.Count; i++)
            NewItem(fit, req.EnvironmentEffects[i], ItemKind.Beacon, ItemLocation.Nowhere, $"/environment/effect_type_ids/{i}");
        AddProjected(fit, req);
        ApplySystemSecurity(fit, req);
        foreach (var o in req.Overrides)
            foreach (var it in fit.Items)
                if (it.TypeId == o.TypeId) fit.SetBase(it.Index, new AttrId(o.AttributeId), o.Value);

        RuleBook.Default.Register(fit);
        return fit;
    }

    private static int NewItem(Fit fit, int typeId, ItemKind kind, ItemLocation loc, string path)
    {
        var t = fit.Ds.Type(typeId) ?? throw new EngineException("UNKNOWN_TYPE", $"unknown type_id {typeId}", path);
        var item = new Item
        {
            Index = fit.Items.Count, Type = t, Kind = kind, Location = loc, BaseAttrs = t.Attrs,
            Owned = kind is ItemKind.Module or ItemKind.Charge or ItemKind.Drone or ItemKind.Fighter or ItemKind.Ship,
            Effects = new List<EffectRef>(t.Effects), RequiredSkills = t.RequiredSkills,
        };
        fit.Items.Add(item);
        return item.Index;
    }

    private static void AddSkills(Fit fit, FitRequest req)
    {
        var ds = fit.Ds;
        int def = req.DefaultSkillLevel ?? 0;
        // every published skill exists (untrained = level 0): ship-bonus attributes are scaled by a skill-level
        // PreMul on the skill, so a missing skill would leave the raw per-level value
        var levels = new SortedDictionary<int, int>();
        foreach (var s in ds.PublishedSkills) levels[s] = def;
        foreach (var (key, lvl) in req.SkillLevels)
        {
            if (int.TryParse(key, out var id)) levels[id] = lvl;
            else if (ds.TypeByNameLookup(key) is int byName) levels[byName] = lvl;
        }
        foreach (var (s, l) in levels)
        {
            if (ds.Type(s) == null) continue;
            int idx = NewItem(fit, s, ItemKind.Skill, ItemLocation.Character, "/character/skills");
            fit.SetBase(idx, KnownIds.SkillLevel, Math.Clamp(l, 0, 5));
        }
    }

    /// <summary>Tactical destroyers must have a mode: default to the first (lowest type id), like the client and Pyfa.</summary>
    private static void AddMode(Fit fit, FitRequest req)
    {
        int? mode = req.ModeTypeId;
        if (mode == null)
        {
            var shipName = fit.Ds.Type(req.ShipTypeId)!.Name.ToLowerInvariant();
            foreach (var (name, id) in fit.Ds.TacticalModes)
                if (name.StartsWith(shipName, StringComparison.Ordinal)) { mode = id; break; }
            if (mode != null) fit.Warnings.Add($"no tactical mode given; defaulted to type {mode}");
        }
        if (mode is int m) NewItem(fit, m, ItemKind.Mode, ItemLocation.Nowhere, "/ship/mode_type_id");
    }

    private static void AddModule(Fit fit, int i, ModuleReq m, string path)
    {
        int idx = NewItem(fit, m.TypeId, ItemKind.Module, ItemLocation.Ship, path);
        var it = fit[idx];
        it.Slot = m.Slot ?? InferSlot(it.Type);
        it.RequestIndex = i;
        it.Spool = m.Spool;
        it.State = m.State ?? ModuleState.Online;
        // rigs and subsystems are passive: online unless explicitly offline
        if (it.Slot is Slot.Rig or Slot.Subsystem && it.State != ModuleState.Offline) it.State = ModuleState.Online;
        if (m.Mutation != null) ApplyMutation(fit, idx, m.Mutation);
        if (m.ChargeTypeId is int c)
        {
            int cidx = NewItem(fit, c, ItemKind.Charge, ItemLocation.Ship, path + "/charge_type_id");
            fit[cidx].Parent = idx;
            fit[cidx].RequestIndex = i;
            it.Charge = cidx;
        }
    }

    private static void AddDrone(Fit fit, int i, DroneReq d)
    {
        int idx = NewItem(fit, d.TypeId, ItemKind.Drone, ItemLocation.Space, $"/drones/{i}");
        if (d.Mutation != null) ApplyMutation(fit, idx, d.Mutation);
        var it = fit[idx];
        it.Quantity = Math.Max(d.Quantity, 1);
        it.ActiveCount = Math.Min(Math.Max(d.Active ?? 0, 0), it.Quantity);
        it.State = it.ActiveCount > 0 ? ModuleState.Active : ModuleState.Offline;
        it.RequestIndex = i;
    }

    private static void AddFighter(Fit fit, int i, FighterReq f)
    {
        var ds = fit.Ds;
        int idx = NewItem(fit, f.TypeId, ItemKind.Fighter, ItemLocation.Space, $"/fighters/{i}");
        var it = fit[idx];
        int maxSq = fit.Has(idx, fit.K.FighterSquadronMaxSize) ? (int)fit.Base(idx, fit.K.FighterSquadronMaxSize) : 1;
        it.Quantity = Math.Clamp(f.Quantity ?? maxSq, 1, Math.Max(maxSq, 1));
        if ((f.Quantity ?? 0) > maxSq) fit.Warnings.Add($"fighters/{i}: squadron size {f.Quantity} capped to {maxSq}");
        it.ActiveCount = f.Active ? it.Quantity : 0;
        it.State = f.Active ? ModuleState.Active : ModuleState.Offline;
        it.FighterAbilities = f.Abilities ?? PyfaDefaultAbilities(ds, it);
        it.RequestIndex = i;
    }

    /// <summary>Pyfa default: standard attack on; other abilities (except MWD/evasive/MJD) only if listed before it.</summary>
    private static int[] PyfaDefaultAbilities(Dataset ds, Item it)
    {
        var on = new List<int>();
        bool stdSeen = false;
        foreach (var e in it.Effects.Select(x => x.Id.Value).OrderBy(x => x))
        {
            var name = ds.Effect(new EffectId(e))?.Name;
            if (name == null || !name.StartsWith("fighterAbility", StringComparison.Ordinal)) continue;
            if (name == "fighterAbilityAttackM") { on.Add(e); stdSeen = true; }
            else if (!stdSeen && name is not ("fighterAbilityMicroWarpDrive" or "fighterAbilityEvasiveManeuvers" or "fighterAbilityMicroJumpDrive"))
                on.Add(e);
        }
        return on.ToArray();
    }

    private static void AddProjected(Fit fit, FitRequest req)
    {
        for (int i = 0; i < req.Projected.Count; i++)
        {
            var p = req.Projected[i];
            switch (p.Kind)
            {
                case "module" when p.Module != null:
                    for (int n = 0; n < Math.Max(p.Amount, 1); n++)
                    {
                        int idx = NewItem(fit, p.Module.TypeId, ItemKind.Projected, ItemLocation.Nowhere, $"/projected/{i}");
                        var it = fit[idx];
                        it.State = p.Module.State ?? ModuleState.Active;
                        it.DistanceM = p.DistanceM;
                        it.RequestIndex = i;
                    }
                    break;
                case "drone" when p.Drone != null:
                    for (int n = 0; n < Math.Max(p.Amount, 1) * Math.Max(p.Drone.Quantity, 1); n++)
                    {
                        int idx = NewItem(fit, p.Drone.TypeId, ItemKind.Projected, ItemLocation.Nowhere, $"/projected/{i}");
                        fit[idx].State = ModuleState.Active;
                        fit[idx].DistanceM = p.DistanceM;
                    }
                    break;
                case "module": case "drone": break;
                default: fit.Warnings.Add($"projected kind '{p.Kind}' not supported yet (index {i})"); break;
            }
        }
    }

    /// <summary>system security -> securityModifier (used by structure rigs etc.). Default nullsec, like Pyfa.</summary>
    private static void ApplySystemSecurity(Fit fit, FitRequest req)
    {
        var k = fit.K;
        var sec = (req.SystemSecurity ?? "nullsec").ToLowerInvariant();
        AttrId src;
        switch (sec)
        {
            case "hisec": case "highsec": case "high": src = k.HiSecModifier; break;
            case "lowsec": case "low": src = k.LowSecModifier; break;
            case "nullsec": case "null": case "wspace": case "wormhole": case "w-space": src = k.NullSecModifier; break;
            default: fit.Warnings.Add($"unknown system_security '{sec}', using nullsec"); src = k.NullSecModifier; break;
        }
        foreach (var it in fit.Items)
            if (fit.Has(it.Index, src)) fit.SetBase(it.Index, k.SecurityModifier, fit.Base(it.Index, src));
    }

    /// <summary>Mutated (abyssal) items: base type's attributes, own attributes on top, then rolled absolute values clamped to the mutaplasmid range.</summary>
    private static void ApplyMutation(Fit fit, int idx, Mutation m)
    {
        var ds = fit.Ds;
        var it = fit[idx];
        var merged = new SortedDictionary<int, double>();
        foreach (var (a, v) in it.BaseAttrs.Entries()) merged[a.Value] = v;
        var baseType = ds.Type(m.BaseTypeId);
        if (baseType != null)
        {
            foreach (var (a, v) in baseType.RawAttrs.Entries()) merged[a.Value] = v;
            foreach (var (a, v) in it.Type.RawAttrs.Entries()) merged[a.Value] = v;
            foreach (var e in baseType.Effects)
                if (!it.Type.HasEffect(e.Id)) it.Effects.Add(e);
            if (it.RequiredSkills.Length == 0) it.RequiredSkills = baseType.RequiredSkills;
            if (merged.GetValueOrDefault(4) == 0.0 && baseType.Mass != 0.0) merged[4] = baseType.Mass;
        }
        MutaplasmidInfo? muta = m.MutaplasmidTypeId is int mid && ds.Mutaplasmids.TryGetValue(mid, out var mi) ? mi : null;
        foreach (var (key, rolled) in m.Attributes)
        {
            if (!int.TryParse(key, out var aid) || aid < 0) continue;
            double val = rolled;
            if (muta != null && baseType != null && muta.Ranges.TryGetValue(aid, out var range) && baseType.RawAttrs.TryGet(new AttrId(aid), out var bv))
            {
                double a = bv * range.Lo, b = bv * range.Hi;
                double mn = a < b ? a : b, mx = a < b ? b : a;
                if (bv != 0.0) val = Math.Clamp(val, mn, mx);
            }
            merged[aid] = val;
        }
        it.BaseAttrs = AttrTable.From(merged);
    }

    /// <summary>Slot from the type's slot effect (hiPower 12, medPower 13, loPower 11, rigSlot 2663, subSystem 3772, serviceSlot 6306).</summary>
    public static Slot? InferSlot(TypeInfo t)
    {
        foreach (var e in t.Effects)
            switch (e.Id.Value)
            {
                case 12: return Slot.High;
                case 13: return Slot.Mid;
                case 11: return Slot.Low;
                case 2663: return Slot.Rig;
                case 3772: return Slot.Subsystem;
                case 6306: return Slot.Service;
            }
        return null;
    }
}
