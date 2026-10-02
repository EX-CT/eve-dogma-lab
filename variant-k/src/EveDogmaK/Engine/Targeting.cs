using EveDogmaK.Data;

namespace EveDogmaK.Engine;

/// <summary>
/// Resolves a modifier's (func, domain, filter) to target items. Location/group/skill lookups go through
/// indexes built once per fit, so registering ~500 skills does not scan the whole item list per modifier.
/// Targets are always yielded in item order (keeps modifier order identical to a linear scan).
/// </summary>
public sealed class TargetIndex
{
    private readonly List<int> _shipLoc = new(), _charLoc = new();
    private readonly Dictionary<int, List<int>> _shipByGroup = new(), _charByGroup = new();
    private readonly Dictionary<int, List<int>> _shipBySkill = new(), _ownedBySkill = new(), _charReachBySkill = new();
    private static readonly List<int> Empty = new();

    public TargetIndex(Fit fit)
    {
        foreach (var it in fit.Items)
        {
            int i = it.Index;
            if (it.Location == ItemLocation.Ship)
            {
                _shipLoc.Add(i);
                Add(_shipByGroup, it.Group, i);
                foreach (var s in Distinct(it.RequiredSkills)) Add(_shipBySkill, s, i);
            }
            if (it.Location == ItemLocation.Character)
            {
                _charLoc.Add(i);
                Add(_charByGroup, it.Group, i);
            }
            if (it.Owned) foreach (var s in Distinct(it.RequiredSkills)) Add(_ownedBySkill, s, i);
            if ((it.Owned || it.Location == ItemLocation.Character) && it.Kind != ItemKind.Skill)
                foreach (var s in Distinct(it.RequiredSkills)) Add(_charReachBySkill, s, i);
        }
    }

    private static IEnumerable<int> Distinct(int[] a) => a.Length <= 1 ? a : a.Distinct();
    private static void Add(Dictionary<int, List<int>> d, int k, int i)
    {
        if (!d.TryGetValue(k, out var l)) d[k] = l = new List<int>();
        l.Add(i);
    }
    private static List<int> Get(Dictionary<int, List<int>> d, int k) => d.TryGetValue(k, out var l) ? l : Empty;

    /// <summary>Append the targets of a modifier emitted by item <paramref name="src"/> to <paramref name="output"/>.</summary>
    public void Resolve(Fit fit, int src, ModFunc func, ModDomain domain, int filter, List<int> output)
    {
        output.Clear();
        var s = fit.Items[src];
        switch (domain)
        {
            case ModDomain.Item:
                if (func == ModFunc.Item) output.Add(src);
                break;
            case ModDomain.Other:
                if (s.Charge >= 0) output.Add(s.Charge);
                else if (s.Parent >= 0) output.Add(s.Parent);
                break;
            case ModDomain.Ship:
            case ModDomain.Structure:
                if (domain == ModDomain.Structure && !fit.IsStructure) break;
                switch (func)
                {
                    case ModFunc.Item: output.Add(fit.Ship); break;
                    case ModFunc.Location: output.AddRange(_shipLoc); break;
                    case ModFunc.LocationGroup: output.AddRange(Get(_shipByGroup, filter)); break;
                    case ModFunc.LocationRequiredSkill: output.AddRange(Get(_shipBySkill, filter)); break;
                    case ModFunc.OwnerRequiredSkill: output.AddRange(Get(_ownedBySkill, filter)); break;
                }
                break;
            case ModDomain.Char:
                switch (func)
                {
                    case ModFunc.Item: output.Add(fit.Char); break;
                    case ModFunc.Location: output.AddRange(_charLoc); break;
                    case ModFunc.LocationGroup: output.AddRange(Get(_charByGroup, filter)); break;
                    case ModFunc.LocationRequiredSkill:
                    case ModFunc.OwnerRequiredSkill: output.AddRange(Get(_charReachBySkill, filter)); break;
                }
                break;
        }
    }
}
