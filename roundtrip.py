import json,glob,subprocess
files=sorted(glob.glob("/workspace/exct-eve/eve-dogma-bench/cases/*.json")+glob.glob("cases_*/*.json"))
reqs=[]
for f in files:
    try: j=json.load(open(f))
    except: continue
    r=j.get("request",j) if isinstance(j,dict) else None
    if isinstance(r,dict) and "ship" in r: reqs.append(r)
B=["variant-h/target/release/eve-dogma-h","--dataset","/workspace/exct-eve/data/dataset-3569502.json.gz","serve-stdio"]
def rpc(calls):
    out=subprocess.run(B,input="".join(json.dumps(c)+"\n" for c in calls),capture_output=True,text=True).stdout.splitlines()
    return [json.loads(l).get("result") for l in out]
t1=[x["text"] for x in rpc([{"id":i,"method":"eft_export","params":{"fit":r,"name":"oracle"}} for i,r in enumerate(reqs)])]
p=rpc([{"id":i,"method":"eft_parse","params":{"text":t}} for i,t in enumerate(t1)])
bad=[i for i,x in enumerate(p) if not x or "error" in x]
t2=[x["text"] if x else None for x in rpc([{"id":i,"method":"eft_export","params":{"fit":x,"name":"oracle"}} for i,x in enumerate(p)])]
diff=[i for i in range(len(t1)) if t1[i]!=t2[i] and i not in bad]
print(len(reqs),"parse errors",len(bad),"roundtrip diffs",len(diff))
for i in (bad+diff)[:3]:
    import difflib; print(files[i] if len(files)==len(reqs) else i); print("\n".join(difflib.unified_diff(t1[i].splitlines(),(t2[i] or "").splitlines(),lineterm="")))
