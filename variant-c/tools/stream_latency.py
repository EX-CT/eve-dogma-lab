"""Round-trip latency of the long-running modes (one request in flight at a time, like an MCP/web backend).
usage: python3 tools/stream_latency.py [dataset] [n]"""
import json, subprocess, sys, time
D = sys.argv[1] if len(sys.argv) > 1 else "/workspace/exct-eve/data/dataset-3569502.json.gz"
N = int(sys.argv[2]) if len(sys.argv) > 2 else 500
req = json.load(open("testdata/requests/exct_rifter.json"))
for mode in ("batch", "serve-stdio"):
    t0 = time.perf_counter()
    p = subprocess.Popen(["bin/eve-dogma-go", "--dataset", D, mode], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL, text=True, bufsize=1)
    lat = []
    for i in range(N):
        line = json.dumps(req if mode == "batch" else {"id": i, "method": "calc", "params": req})
        t = time.perf_counter()
        p.stdin.write(line + "\n"); p.stdin.flush()
        out = p.stdout.readline()
        lat.append(time.perf_counter() - t)
        assert out.startswith("{"), out
    p.stdin.close(); p.wait()
    first, rest = lat[0], sorted(lat[1:])
    print(f"{mode:12s} first reply (incl. dataset load) {(first + 0) * 1000:7.1f} ms | warm round-trip median "
          f"{rest[len(rest)//2]*1000:.3f} ms, p95 {rest[int(len(rest)*.95)]*1000:.3f} ms ({N} requests)")
