import sys, json
sys.argv = [sys.argv[0]] + sys.argv[1:]
import importlib.util
spec = importlib.util.spec_from_file_location("po", "/workspace/exct-eve/lab-h/oracle_local/pyfa_oracle.py")
po = importlib.util.module_from_spec(spec); spec.loader.exec_module(po)
for p in sys.argv[1:]:
    fit = po.build(json.load(open(p))); fit.calculateModifiedAttributes()
    print(p, "factorReload", fit.factorReload, "capUsed", fit.capUsed, "capRecharge", fit.capRecharge, "peak", fit.calculateCapRecharge())
    for m in fit.modules:
        if m.isEmpty: continue
        cp = m.getCycleParameters()
        print(" ", m.item.name, m.state, "capNeed", m.getModifiedItemAttr("capacitorNeed"), "capUse", m.capUse, "avg", cp.averageTime if cp else None, "shots", m.numShots, "reload", m.reloadTime)
