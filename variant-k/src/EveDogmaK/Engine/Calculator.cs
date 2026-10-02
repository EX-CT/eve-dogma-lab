using EveDogmaK.Data;
using EveDogmaK.Json;
using EveDogmaK.Requests;
using EveDogmaK.Stats;

namespace EveDogmaK.Engine;

/// <summary>Public entry point: <c>Calc(dataset, request) -> FitStats</c>. Pure: no I/O, no clocks, no global mutable state.</summary>
public static class Calculator
{
    public static JNode Calc(Dataset ds, FitRequest req)
    {
        try
        {
            var fit = FitBuilder.Build(ds, req);
            return new StatsCalculator(fit).Compute();
        }
        catch (EngineException e)
        {
            return Error(e.Code, e.Message, e.Path);
        }
    }

    public static string CalcJson(Dataset ds, string requestJson)
    {
        FitRequest req;
        try { req = RequestParser.Parse(requestJson); }
        catch (RequestException e) { return Error(e.Code, e.Message, e.Path).Serialize(); }
        return Calc(ds, req).Serialize();
    }

    public static JObj Error(string code, string message, string path) =>
        new() { { "error", new JObj { { "code", code }, { "message", message }, { "path", path } } } };
}
