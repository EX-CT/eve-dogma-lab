import json,glob,subprocess,sys
prev=sys.argv[1] if len(sys.argv)>1 else "/tmp/hprev-target/release/eve-dogma-h"
cur="variant-h/target/release/eve-dogma-h"
D="/workspace/exct-eve/data/dataset-3569502.json.gz"
files=sorted(glob.glob("/workspace/exct-eve/eve-dogma-bench/cases/*.json")+glob.glob("cases_*/*.json"))
lines=[]
for f in files:
    try: j=json.load(open(f))
    except: continue
    r=j.get("request",j) if isinstance(j,dict) else None
    if isinstance(r,dict) and "ship" in r: lines.append(json.dumps(r))
# also every skill level variant: default 0 / 3 and explicit levels
extra=[]
for l in lines[:150]:
    r=json.loads(l)
    for lv in (0,3):
        r2=json.loads(l); r2.setdefault("character",{}).setdefault("skills",{})["default_level"]=lv; extra.append(json.dumps(r2))
inp="\n".join(lines+extra)+"\n"
run=lambda b: subprocess.run([b,"--dataset",D,"batch"],input=inp,capture_output=True,text=True).stdout.splitlines()
a,b=run(prev),run(cur)
diff=[i for i in range(len(a)) if a[i]!=b[i]]
print("fits",len(a),len(b),"byte-different",len(diff))
for i in diff[:3]:
    x,y=json.loads(a[i]),json.loads(b[i])
    def d(p,u,v):
        if isinstance(u,dict) and isinstance(v,dict):
            for k in set(u)|set(v): d(p+"/"+k,u.get(k),v.get(k))
        elif isinstance(u,list) and isinstance(v,list) and len(u)==len(v):
            for k,(m,n) in enumerate(zip(u,v)): d(p+f"/{k}",m,n)
        elif u!=v: print(i,p,u,v)
    d("",x,y)
