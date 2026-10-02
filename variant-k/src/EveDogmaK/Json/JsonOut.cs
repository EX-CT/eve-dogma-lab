using System.Globalization;
using System.Text;

namespace EveDogmaK.Json;

/// <summary>
/// Minimal output JSON tree. Objects serialise with keys sorted (ordinal), floats rounded to 6 decimals,
/// non-finite floats as null: deterministic, and the same shape as the reference engine's output.
/// </summary>
public abstract class JNode
{
    public static readonly JNode Null = new JNull();
    public static implicit operator JNode(double v) => new JFloat(v);
    public static implicit operator JNode(long v) => new JInt(v);
    public static implicit operator JNode(int v) => new JInt(v);
    public static implicit operator JNode(bool v) => new JBool(v);
    public static implicit operator JNode(string? v) => v == null ? Null : new JStr(v);
    public static JNode Of(double? v) => v is double d ? new JFloat(d) : Null;
    public static JNode Of(int? v) => v is int d ? new JInt(d) : Null;
    public abstract void Write(StringBuilder sb);
    public string Serialize() { var sb = new StringBuilder(8192); Write(sb); return sb.ToString(); }
}

public sealed class JNull : JNode { public override void Write(StringBuilder sb) => sb.Append("null"); }
public sealed class JBool : JNode
{
    public readonly bool V; public JBool(bool v) { V = v; }
    public override void Write(StringBuilder sb) => sb.Append(V ? "true" : "false");
}
public sealed class JInt : JNode
{
    public readonly long V; public JInt(long v) { V = v; }
    public override void Write(StringBuilder sb) => sb.Append(V.ToString(CultureInfo.InvariantCulture));
}
public sealed class JFloat : JNode
{
    public readonly double V; public JFloat(double v) { V = v; }
    public static double Round6(double v) => double.IsFinite(v) ? Math.Round(v * 1e6, MidpointRounding.AwayFromZero) / 1e6 : v;
    public override void Write(StringBuilder sb) => WriteDouble(sb, Round6(V));
    public static void WriteDouble(StringBuilder sb, double v)
    {
        if (!double.IsFinite(v)) { sb.Append("null"); return; }
        if (v == Math.Floor(v) && Math.Abs(v) < 1e16) { sb.Append(v.ToString("F0", CultureInfo.InvariantCulture)).Append(".0"); return; }
        var s = v.ToString("R", CultureInfo.InvariantCulture);
        if (s.Contains('E')) s = s.Replace("E+", "e").Replace("E", "e");
        sb.Append(s);
    }
}
public sealed class JStr : JNode
{
    public readonly string V; public JStr(string v) { V = v; }
    public override void Write(StringBuilder sb) => WriteString(sb, V);
    public static void WriteString(StringBuilder sb, string s)
    {
        sb.Append('"');
        foreach (var c in s)
        {
            switch (c)
            {
                case '"': sb.Append("\\\""); break;
                case '\\': sb.Append("\\\\"); break;
                case '\n': sb.Append("\\n"); break;
                case '\r': sb.Append("\\r"); break;
                case '\t': sb.Append("\\t"); break;
                case '\b': sb.Append("\\b"); break;
                case '\f': sb.Append("\\f"); break;
                default:
                    if (c < 0x20) sb.Append("\\u").Append(((int)c).ToString("x4")); else sb.Append(c);
                    break;
            }
        }
        sb.Append('"');
    }
}
public sealed class JArr : JNode
{
    public readonly List<JNode> Items = new();
    public JArr() { }
    public JArr(IEnumerable<JNode> items) { Items.AddRange(items); }
    public void Add(JNode n) => Items.Add(n);
    public int Count => Items.Count;
    public override void Write(StringBuilder sb)
    {
        sb.Append('[');
        for (int i = 0; i < Items.Count; i++) { if (i > 0) sb.Append(','); Items[i].Write(sb); }
        sb.Append(']');
    }
}
public sealed class JObj : JNode, System.Collections.IEnumerable
{
    private readonly SortedDictionary<string, JNode> _m = new(StringComparer.Ordinal);
    public JNode this[string k] { get => _m[k]; set => _m[k] = value; }
    public void Add(string k, JNode v) => _m[k] = v;
    public bool ContainsKey(string k) => _m.ContainsKey(k);
    System.Collections.IEnumerator System.Collections.IEnumerable.GetEnumerator() => _m.GetEnumerator();
    public override void Write(StringBuilder sb)
    {
        sb.Append('{');
        bool first = true;
        foreach (var (k, v) in _m)
        {
            if (!first) sb.Append(',');
            first = false;
            JStr.WriteString(sb, k);
            sb.Append(':');
            v.Write(sb);
        }
        sb.Append('}');
    }
}
