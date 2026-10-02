"""Closed-loop HTTP load against serve-http: C keep-alive clients POST /v1/calc over the corpus.
usage: python3 tools/http_load.py [host:port] [clients] [requests]"""
import glob, http.client, sys, threading, time
host = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1:8080"
C = int(sys.argv[2]) if len(sys.argv) > 2 else 8
N = int(sys.argv[3]) if len(sys.argv) > 3 else 2000
bodies = [open(f, "rb").read() for f in sorted(glob.glob("testdata/requests/*.json"))]
lat, lock, nxt = [], threading.Lock(), [0]
def worker():
    c = http.client.HTTPConnection(host)
    while True:
        with lock:
            i = nxt[0]; nxt[0] += 1
        if i >= N: return
        t = time.perf_counter()
        c.request("POST", "/v1/calc", bodies[i % len(bodies)], {"Content-Type": "application/json"})
        r = c.getresponse(); r.read()
        with lock: lat.append(time.perf_counter() - t)
t0 = time.perf_counter()
ts = [threading.Thread(target=worker) for _ in range(C)]
[t.start() for t in ts]; [t.join() for t in ts]
el = time.perf_counter() - t0
lat.sort()
print(f"{N} requests, {C} clients: {N/el:.0f} req/s, median {lat[len(lat)//2]*1000:.2f} ms, p95 {lat[int(len(lat)*.95)]*1000:.2f} ms")
