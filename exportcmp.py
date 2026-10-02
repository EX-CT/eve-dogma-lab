#!/usr/bin/env python3
"""GPL test tool: compare H's eft_export with Pyfa's exportEft on the same FitRequests (black-box)."""
import sys
sys.path = [p for p in sys.path if not p.rstrip("/").endswith("lab-h")]
import json, os, glob, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)) + "/oracle_local")
import pyfa_oracle as po
import importlib.abc, importlib.machinery, types
from unittest import mock
class _Mock(importlib.abc.MetaPathFinder, importlib.abc.Loader):
    def find_spec(self, name, path, target=None):
        if name.split(".")[0] in ("wx", "gui"):
            return importlib.machinery.ModuleSpec(name, self, is_package=True)
    def create_module(self, spec):
        m = mock.MagicMock(); m.__path__ = []; m.__name__ = spec.name; m.__spec__ = spec
        m.NewEvent = m.NewCommandEvent = lambda: (mock.MagicMock(), mock.MagicMock())
        return m
    def exec_module(self, m): pass
for k in [k for k in sys.modules if k == "wx" or k.startswith("wx.")]: del sys.modules[k]
sys.meta_path.insert(0, _Mock())
from service.port.eft import exportEft
from service.const import PortEftOptions  # noqa
opts = {o: True for o in PortEftOptions}
files = sorted(glob.glob("/workspace/exct-eve/eve-dogma-bench/cases/*.json") + glob.glob("cases_req/*.json") + glob.glob("cases_new/*.json") + glob.glob("cases_ecm/*.json") + glob.glob("cases_eft/*.json") + glob.glob("cases_export/*.json"))
reqs = []
for f in files:
    j = json.load(open(f))
    r = j.get("request", j)
    if "ship" in r: reqs.append((f, r))
inp = "".join(json.dumps({"id": i, "method": "eft_export", "params": {"fit": r, "name": "oracle"}}) + "\n" for i, (f, r) in enumerate(reqs))
out = subprocess.run(["variant-h/target/release/eve-dogma-h", "--dataset", "/workspace/exct-eve/data/dataset-3569502.json.gz", "serve-stdio"], input=inp, capture_output=True, text=True).stdout.splitlines()
ok = bad = err = 0
for (f, r), line in zip(reqs, out):
    h = json.loads(line)["result"]["text"]
    try:
        fit = po.build(r)
        from eos.saveddata.cargo import Cargo
        for c in r.get("cargo", []):
            cg = Cargo(po.item(c["type_id"])); cg.amount = c.get("quantity", 1); fit.cargo.append(cg)
        fit.calculateModifiedAttributes()
        p = exportEft(fit, opts, None)
    except Exception as e:
        err += 1; po.eos.db.saveddata_session.rollback(); continue
    if p == h: ok += 1
    else:
        bad += 1
        if bad <= int(os.environ.get("SHOW", "4")):
            print("==", f); print(repr(p)); print(repr(h))
print(f"identical {ok} differ {bad} pyfa-error {err} of {len(reqs)}")
