using EveDogmaK.Requests;
using EveDogmaK.Data;
using EveDogmaK.Engine;

namespace EveDogmaK.Rules;

// Engine specials: effects CCP ships without modifierInfo. Each rule states the game behaviour it encodes.

/// <summary>AB / MWD: +massAddition to ship mass, velocity * (1 + speedFactor/100 * speedBoostFactor / mass); MWD also blooms signature.</summary>
public sealed class PropulsionRule : IEffectRule
{
    public string Name => "propulsion";
    public bool Matches(EffectInfo e, KnownIds k) => e.Id == k.Afterburner || e.Id == k.Microwarpdrive;
    public void Apply(in EffectContext c)
    {
        var k = c.K;
        c.OnShip(KnownIds.Mass, Op.ModAdd, c.FromSelf(k.MassAddition));
        c.OnShip(k.MaxVelocity, Op.PostMul, ModSource.Propulsion(c.Source, c.Fit.Ship, k.SpeedFactor, k.SpeedBoostFactor));
        if (c.Effect.Id == k.Microwarpdrive)
            c.OnShip(k.SignatureRadius, Op.PostPercent, c.FromSelf(k.SignatureRadiusBonus));
    }
}

/// <summary>
/// Fighter self abilities without modifierInfo (Pyfa hand-written handlers, eos LGPL): afterburner / MWD / evasive
/// maneuvers boost the squadron's own speed, signature and shield resonances while the ability is on.
/// </summary>
public sealed class FighterSelfAbilityRule : IEffectRule
{
    private static readonly Dictionary<string, (string Target, string Source, Op Op)[]> Table = new()
    {
        ["fighterAbilityMicroWarpDrive"] = new[]
        {
            ("maxVelocity", "fighterAbilityMicroWarpDriveSpeedBonus", Op.PostPercent),
            ("signatureRadius", "fighterAbilityMicroWarpDriveSignatureRadiusBonus", Op.PostPercent),
        },
        ["fighterAbilityAfterburner"] = new[] { ("maxVelocity", "fighterAbilityAfterburnerSpeedBonus", Op.PostPercent) },
        ["fighterAbilityEvasiveManeuvers"] = new[]
        {
            ("maxVelocity", "fighterAbilityEvasiveManeuversSpeedBonus", Op.PostPercent),
            ("signatureRadius", "fighterAbilityEvasiveManeuversSignatureRadiusBonus", Op.PostPercent),
            ("shieldEmDamageResonance", "fighterAbilityEvasiveManeuversEmResonance", Op.PostMul),
            ("shieldThermalDamageResonance", "fighterAbilityEvasiveManeuversThermResonance", Op.PostMul),
            ("shieldKineticDamageResonance", "fighterAbilityEvasiveManeuversKinResonance", Op.PostMul),
            ("shieldExplosiveDamageResonance", "fighterAbilityEvasiveManeuversExpResonance", Op.PostMul),
        },
    };

    public string Name => "fighter-self-ability";
    public bool Matches(EffectInfo e, KnownIds k) => e.Modifiers.Length == 0 && Table.ContainsKey(e.Name);
    public void Apply(in EffectContext c)
    {
        if (c.Item.Kind != ItemKind.Fighter) return;
        foreach (var (target, source, op) in Table[c.Effect.Name])
            c.Fit.AddModifier(c.Source, c.Ds.AttrIdOf(target), op, c.FromSelf(c.Ds.AttrIdOf(source)), c.Source, c.SourceCategory);
    }
}

/// <summary>Micro jump drive signature bloom; unlike the MWD's it is not stacking penalised.</summary>
public sealed class MicroJumpDriveRule : IEffectRule
{
    public string Name => "micro-jump-drive";
    public bool Matches(EffectInfo e, KnownIds k) => e.Id == k.MicroJumpDrive;
    public void Apply(in EffectContext c) =>
        c.OnShip(c.K.SignatureRadius, Op.PostPercent, c.FromSelf(c.K.SignatureRadiusBonusPercent), Fit.ShipCategory);
}

/// <summary>T3C subsystems: add hi/med/low slots to the hull.</summary>
public sealed class SlotModifierRule : IEffectRule
{
    public string Name => "subsystem-slots";
    public bool Matches(EffectInfo e, KnownIds k) => e.Id == k.SlotModifier;
    public void Apply(in EffectContext c)
    {
        var k = c.K;
        c.OnShip(k.HiSlots, Op.ModAdd, c.FromSelf(k.HiSlotModifier));
        c.OnShip(k.MedSlots, Op.ModAdd, c.FromSelf(k.MedSlotModifier));
        c.OnShip(k.LowSlots, Op.ModAdd, c.FromSelf(k.LowSlotModifier));
    }
}

/// <summary>T3C subsystems: add turret/launcher hardpoints to the hull.</summary>
public sealed class HardpointModifierRule : IEffectRule
{
    public string Name => "subsystem-hardpoints";
    public bool Matches(EffectInfo e, KnownIds k) => e.Id == k.HardPointModifier;
    public void Apply(in EffectContext c)
    {
        var k = c.K;
        c.OnShip(k.TurretSlotsLeft, Op.ModAdd, c.FromSelf(k.TurretHardPointModifier));
        c.OnShip(k.LauncherSlotsLeft, Op.ModAdd, c.FromSelf(k.LauncherHardPointModifier));
    }
}

/// <summary>
/// The general case: SDE modifierInfo. Targets come from (func, domain, filter). Two documented exceptions:
/// skill filter 0 means "the type owning the effect" (EXCT pipeline convention for skill self-bonuses), and
/// bastion hull resists are not stacking penalised (observed in game / Pyfa although the SDE marks them non-stackable).
/// </summary>
public sealed class DataDrivenModifierRule : IEffectRule
{
    private readonly List<int> _targets = new();
    public string Name => "sde-modifiers";
    public bool Matches(EffectInfo e, KnownIds k) => true;
    public void Apply(in EffectContext c)
    {
        var fit = c.Fit;
        var index = fit.Targets;
        bool bastion = c.Effect.Id == c.K.Bastion;
        foreach (var m in c.Effect.Modifiers)
        {
            if (m.Func == ModFunc.EffectStopper || m.Op == Op.Unsupported) continue;
            if (m.Domain is ModDomain.TargetId or ModDomain.Target) continue; // projected-only, handled on the receiving fit
            int filter = m.Filter == 0 && m.Func is ModFunc.LocationRequiredSkill or ModFunc.OwnerRequiredSkill ? c.Item.TypeId : m.Filter;
            index.Resolve(fit, c.Source, m.Func, m.Domain, filter, _targets);
            int cat = bastion && Array.IndexOf(KnownIds.HullResonances, m.Modified) >= 0 ? Fit.ShipCategory : c.SourceCategory;
            foreach (var t in _targets) fit.AddModifier(t, m.Modified, m.Op, ModSource.FromAttr(c.Source, m.Modifying), c.Source, cat);
        }
    }
}

/// <summary>
/// Active local modules whose SDE effect has no modifierInfo but a hand-written Pyfa handler (eos LGPL; re-expressed
/// from the reference engine). Some of these effects are target-category in the SDE, so the rule runs before the
/// state gate, for active (or overheated) modules only. A ship source category marks a boost Pyfa applies unpenalised.
/// </summary>
public static class LocalSpecialRule
{
    private static readonly HashSet<string> Names = new()
    {
        "superWeaponAmarr", "superWeaponCaldari", "superWeaponGallente", "superWeaponMinmatar", "doomsdaySlash",
        "doomsdayBeamDOT", "doomsdayConeDOT", "doomsdayHOG", "debuffLance",
        "emergencyHullEnergizer", "entosisLink", "microJumpPortalDrive", "microJumpPortalDriveCapital", "warpDisruptSphere",
    };

    public static bool TryApply(Fit fit, Item it, EffectInfo e, ModuleState state)
    {
        if (e.Modifiers.Length != 0 || it.Kind != ItemKind.Module || state < ModuleState.Active || !Names.Contains(e.Name)) return false;
        var ds = fit.Ds;
        int i = it.Index, ship = fit.Ship, cat = it.Category, shipCat = Fit.ShipCategory;
        AttrId A(string n) => ds.AttrIdOf(n);
        void On(int target, string attr, Op op, ModSource src, int category) => fit.AddModifier(target, A(attr), op, src, i, category);
        ModSource Self(string n) => ModSource.FromAttr(i, A(n));
        switch (e.Name)
        {
            case "emergencyHullEnergizer":
                foreach (var t in new[] { "Em", "Thermal", "Kinetic", "Explosive" })
                    On(ship, t.ToLowerInvariant() + "DamageResonance", Op.PostMul, Self($"hull{t}DamageResonance"), cat);
                break;
            case "entosisLink":
                On(ship, "disallowAssistance", Op.PostAssign, Self("disallowAssistance"), shipCat);
                foreach (var t in new[] { "Gravimetric", "Magnetometric", "Radar", "Ladar" })
                    On(ship, $"scan{t}Strength", Op.PostPercent, Self($"scan{t}StrengthPercent"), cat);
                break;
            case "microJumpPortalDrive":
            case "microJumpPortalDriveCapital":
                On(ship, "signatureRadius", Op.PostPercent, Self("signatureRadiusBonusPercent"), cat);
                break;
            case "warpDisruptSphere":
                On(ship, "disallowAssistance", Op.PostAssign, ModSource.Const(1.0), shipCat);
                if (it.Charge < 0)
                {
                    fit.AddModifier(ship, KnownIds.Mass, Op.PostPercent, Self("massBonusPercentage"), i, shipCat);
                    On(ship, "signatureRadius", Op.PostPercent, Self("signatureRadiusBonus"), shipCat);
                    foreach (var t in fit.Items.Where(t => t.Kind == ItemKind.Module && t.Location == ItemLocation.Ship
                                 && ds.Groups.TryGetValue(t.Group, out var g) && g.Name == "Propulsion Module").Select(t => t.Index).ToList())
                    {
                        On(t, "speedBoostFactor", Op.PostPercent, Self("speedBoostFactorBonus"), shipCat);
                        On(t, "speedFactor", Op.PostPercent, Self("speedFactorBonus"), shipCat);
                    }
                }
                break;
            default: // superweapons / lances
                On(ship, "maxVelocity", Op.PostPercent, Self("speedFactor"), cat);
                On(ship, "warpScrambleStatus", Op.ModAdd, Self("siegeModeWarpStatus"), cat);
                break;
        }
        return true;
    }
}
