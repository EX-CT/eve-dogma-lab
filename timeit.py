import subprocess,time,sys
D="/workspace/exct-eve/data/dataset-3569502.json.gz"
inp=open("/tmp/all.jsonl","rb").read(); n=inp.count(b"\n")
one=inp.split(b"\n")[0]+b"\n"
for b in ["variant-h/target/release/eve-dogma-h","/workspace/exct-eve/eve-dogma-rs/target/release/eve-dogma"]:
    best=1e9
    for _ in range(3):
        t=time.perf_counter(); subprocess.run([b,"--dataset",D,"batch"],input=inp,stdout=subprocess.DEVNULL); best=min(best,time.perf_counter()-t)
    cold=1e9
    for _ in range(5):
        t=time.perf_counter(); subprocess.run([b,"--dataset",D,"calc"],input=one,stdout=subprocess.DEVNULL); cold=min(cold,time.perf_counter()-t)
    print(f"{b.split('/')[-1]}: batch {n} fits {best:.3f}s = {n/best:.0f} fits/s ({best/n*1000:.2f} ms/fit incl. startup); single calc process {cold*1000:.0f} ms")
