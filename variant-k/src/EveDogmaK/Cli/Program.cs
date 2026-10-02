using EveDogmaK.Stats;
using System.Diagnostics;
using System.Text;
using EveDogmaK.Data;
using EveDogmaK.Engine;
using EveDogmaK.Json;
using EveDogmaK.Requests;

namespace EveDogmaK.Cli;

public static class Program
{
    private const string Usage = """
        eve-dogma-k <command> [--dataset PATH] [args]

        Commands:
          calc [FILE]            FitRequest JSON (file or stdin) -> FitStats JSON
          batch                  JSONL FitRequests on stdin -> JSONL FitStats on stdout
          serve-stdio            JSONL RPC: {"id":..,"method":"calc|search|type|meta","params":..}
          search QUERY           search types by name
          type ID|NAME           show type with base attributes
          meta                   dataset info
          bench [FILE] [-n N]    time N calculations of a request

        Dataset: --dataset PATH, or $EVE_DOGMA_DATASET, or ./dataset.json.gz
        """;

    public static int Main(string[] argv)
    {
        var args = argv.ToList();
        string? dataset = TakeFlag(args, "--dataset");
        string cmd = args.Count > 0 ? args[0] : "";
        var stdout = new StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false), 1 << 16) { AutoFlush = false, NewLine = "\n" };
        try
        {
            switch (cmd)
            {
                case "calc":
                {
                    var ds = Load(dataset);
                    var input = ReadInput(args.Count > 1 ? args[1] : null);
                    var res = Calculator.CalcJson(ds, input);
                    stdout.WriteLine(res);
                    stdout.Flush();
                    return res.StartsWith("{\"error\"", StringComparison.Ordinal) ? 2 : 0;
                }
                case "batch":
                {
                    var ds = Load(dataset);
                    using var stdin = new StreamReader(Console.OpenStandardInput(), Encoding.UTF8, false, 1 << 16);
                    string? line;
                    while ((line = stdin.ReadLine()) != null)
                    {
                        if (string.IsNullOrWhiteSpace(line)) continue;
                        stdout.WriteLine(Calculator.CalcJson(ds, line));
                    }
                    stdout.Flush();
                    return 0;
                }
                case "serve-stdio":
                {
                    var ds = Load(dataset);
                    Console.Error.WriteLine($"eve-dogma-k serve-stdio ready (sde {ds.Build})");
                    using var stdin = new StreamReader(Console.OpenStandardInput(), Encoding.UTF8, false, 1 << 16);
                    string? line;
                    while ((line = stdin.ReadLine()) != null)
                    {
                        if (string.IsNullOrWhiteSpace(line)) continue;
                        stdout.WriteLine(Rpc.Handle(ds, line).Serialize());
                        stdout.Flush();
                    }
                    return 0;
                }
                case "search":
                {
                    var ds = Load(dataset);
                    stdout.WriteLine(Rpc.Search(ds, string.Join(' ', args.Skip(1)), 25).Serialize());
                    stdout.Flush();
                    return 0;
                }
                case "type":
                {
                    var ds = Load(dataset);
                    stdout.WriteLine(Rpc.TypeInfo(ds, string.Join(' ', args.Skip(1))).Serialize());
                    stdout.Flush();
                    return 0;
                }
                case "meta":
                {
                    var sw = Stopwatch.StartNew();
                    var ds = Load(dataset);
                    var m = Rpc.Meta(ds);
                    m["load_ms"] = sw.Elapsed.TotalMilliseconds;
                    stdout.WriteLine(m.Serialize());
                    stdout.Flush();
                    return 0;
                }
                case "bench":
                {
                    int n = int.TryParse(TakeFlag(args, "-n"), out var nn) ? nn : 1000;
                    var sw = Stopwatch.StartNew();
                    var ds = Load(dataset);
                    double loadMs = sw.Elapsed.TotalMilliseconds;
                    var req = RequestParser.Parse(ReadInput(args.Count > 1 ? args[1] : null));
                    for (int i = 0; i < 20; i++) Calculator.Calc(ds, req).Serialize(); // warm-up
                    sw.Restart();
                    for (int i = 0; i < n; i++) Calculator.Calc(ds, req).Serialize();
                    double el = sw.Elapsed.TotalSeconds;
                    // phase breakdown: build graph / derived stats / serialise
                    double tb = 0, ts = 0, tj = 0;
                    for (int i = 0; i < n; i++)
                    {
                        long t0 = Stopwatch.GetTimestamp();
                        var fit = FitBuilder.Build(ds, req);
                        long t1 = Stopwatch.GetTimestamp();
                        var st = new StatsCalculator(fit).Compute();
                        long t2 = Stopwatch.GetTimestamp();
                        st.Serialize();
                        long t3 = Stopwatch.GetTimestamp();
                        tb += t1 - t0; ts += t2 - t1; tj += t3 - t2;
                    }
                    double us = 1e6 / Stopwatch.Frequency / n;
                    stdout.WriteLine(new JObj { { "dataset_load_ms", loadMs }, { "iterations", n }, { "total_s", el }, { "per_calc_us", el / n * 1e6 },
                        { "build_us", tb * us }, { "stats_us", ts * us }, { "json_us", tj * us } }.Serialize());
                    stdout.Flush();
                    return 0;
                }
                default:
                    Console.Error.WriteLine(Usage);
                    return cmd is "" or "help" or "--help" ? 0 : 2;
            }
        }
        catch (RequestException e)
        {
            Console.Error.WriteLine($"error: {e.Message}");
            return 2;
        }
    }

    private static string? TakeFlag(List<string> args, string flag)
    {
        int p = args.IndexOf(flag);
        if (p < 0) return null;
        string? v = p + 1 < args.Count ? args[p + 1] : null;
        args.RemoveRange(p, Math.Min(2, args.Count - p));
        return v;
    }

    private static Dataset Load(string? path)
    {
        var p = path ?? Environment.GetEnvironmentVariable("EVE_DOGMA_DATASET") ?? "dataset.json.gz";
        try { return DatasetCache.Load(p); }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or InvalidDataException or System.Text.Json.JsonException)
        {
            Console.Error.WriteLine($"error: read {p}: {e.Message}");
            Environment.Exit(3);
            throw;
        }
    }

    private static string ReadInput(string? file)
    {
        if (file != null && file != "-")
        {
            try { return File.ReadAllText(file); }
            catch (IOException e) { Console.Error.WriteLine($"error: {file}: {e.Message}"); Environment.Exit(2); }
        }
        using var stdin = new StreamReader(Console.OpenStandardInput(), Encoding.UTF8);
        return stdin.ReadToEnd();
    }
}
