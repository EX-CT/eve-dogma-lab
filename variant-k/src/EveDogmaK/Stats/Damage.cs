using EveDogmaK.Json;
using EveDogmaK.Requests;

namespace EveDogmaK.Stats;

/// <summary>Damage split by type (em, thermal, kinetic, explosive).</summary>
public readonly record struct Damage(double Em, double Thermal, double Kinetic, double Explosive)
{
    public static readonly Damage Zero = new(0, 0, 0, 0);
    public double Total => Em + Thermal + Kinetic + Explosive;
    public Damage Scale(double k) => new(Em * k, Thermal * k, Kinetic * k, Explosive * k);
    public static Damage operator +(Damage a, Damage b) => new(a.Em + b.Em, a.Thermal + b.Thermal, a.Kinetic + b.Kinetic, a.Explosive + b.Explosive);
    /// <summary>Damage dealt to a target with the given resist fractions (0..1).</summary>
    public double Versus(DamageProfile resist) =>
        Em * (1.0 - resist.Em) + Thermal * (1.0 - resist.Thermal) + Kinetic * (1.0 - resist.Kinetic) + Explosive * (1.0 - resist.Explosive);
    public JObj ToJson() => new() { { "em", Em }, { "thermal", Thermal }, { "kinetic", Kinetic }, { "explosive", Explosive }, { "total", Total } };
}
