#!/usr/bin/env python3
"""Contract 0.2 validation precedence and error codes, plus the 0.2 clamps (self-contained; dataset from
EVE_DOGMA_DATASET, skipped when unset).  python3 -m unittest discover -s tests"""
import copy
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path[:0] = [ROOT, os.path.join(os.path.dirname(ROOT), "variant-g")]

FIT = {"schema_version": 1, "ship": {"type_id": 587}}  # Rifter, empty
BASE = {"schema_version": 1, "graph": "damage", "fit": FIT, "x": {"axis": "distance_m", "values": [0, 1000]}, "y": ["dps"]}
_ENG = []


def engine():
    if not _ENG:
        d = os.environ.get("EVE_DOGMA_DATASET")
        if not d:
            raise unittest.SkipTest("EVE_DOGMA_DATASET not set")
        from evedogma_g import dataset
        from g3.graph import Engine
        _ENG.append(Engine(dataset.load(d), dataset_path=d))
    return _ENG[0]


def req(**kw):
    r = copy.deepcopy(BASE)
    for k, v in kw.items():
        if v is None:
            r.pop(k, None)
        else:
            r[k] = v
    return r


class Validation(unittest.TestCase):
    def code(self, r):
        res = engine().graph(r)
        return res.get("error", {}).get("code")

    def test_structural_bad_request_first(self):
        bad_ship = {"schema_version": 1, "ship": {"type_id": 999999999}}
        self.assertEqual(self.code(req(graph=None, fit=bad_ship)), "BAD_REQUEST")
        self.assertEqual(self.code(req(fit=None)), "BAD_REQUEST")
        self.assertEqual(self.code(req(x=None)), "BAD_REQUEST")
        self.assertEqual(self.code(req(x={"axis": "distance_m"})), "BAD_REQUEST")
        self.assertEqual(self.code(req(x={"axis": "distance_m", "values": [0, None]})), "BAD_REQUEST")
        self.assertEqual(self.code(req(y=[])), "BAD_REQUEST")
        self.assertEqual(self.code(req(y=None, graph="nope")), "BAD_REQUEST")

    def test_graph_axis_enum_type_order(self):
        self.assertEqual(self.code(req(graph="fitDamageStats")), "UNKNOWN_GRAPH")
        self.assertEqual(self.code(req(x={"axis": "distance_km", "values": [0]})), "BAD_AXIS")
        self.assertEqual(self.code(req(y=["dps", "alpha"])), "BAD_AXIS")
        self.assertEqual(self.code(req(graph="ecm_burst", x={"axis": "tgt_dps", "values": [1]}, y=["tgt_lock_time_s"])),
                         "BAD_AXIS")
        bad_ship = {"schema_version": 1, "ship": {"type_id": 999999999}}
        self.assertEqual(self.code(req(settings={"mobile_drone_mode": "orbit"}, fit=bad_ship)), "BAD_REQUEST")
        self.assertEqual(self.code(req(target={"fit": FIT, "resist_mode": "kinetic"})), "BAD_REQUEST")
        self.assertEqual(self.code(req(fit=bad_ship)), "UNKNOWN_TYPE")
        self.assertEqual(self.code(req(target={"fit": bad_ship})), "UNKNOWN_TYPE")
        # graphs without a target ignore target.fit
        self.assertIsNone(self.code(req(graph="lock_time", x={"axis": "tgt_sig_m", "values": [100]}, y=["time_s"],
                                        target={"fit": bad_ship})))

    def test_empty_x_still_validated(self):
        self.assertEqual(self.code(req(x={"axis": "distance_m", "values": []}, y=[])), "BAD_REQUEST")
        res = engine().graph(req(x={"axis": "distance_m", "values": []}))
        self.assertEqual(res["series"], {"dps": []})

    def test_clamps(self):
        e = engine()
        a = e.graph(req(x={"axis": "distance_m", "values": [0, 500]}, y=["damage"], params={"time_s": 2500}))
        b = e.graph(req(x={"axis": "distance_m", "values": [0, 500]}, y=["damage"], params={"time_s": 9000}))
        self.assertEqual(a["series"], b["series"])
        r = req(graph="ewar", y=["web_pct"])
        self.assertEqual(e.graph(dict(r, params={"resist": 1.0}))["series"], e.graph(dict(r, params={"resist": 7}))["series"])
        self.assertEqual(e.graph(dict(r, params={"resist": 0.0}))["series"], e.graph(dict(r, params={"resist": -3}))["series"])


if __name__ == "__main__":
    unittest.main()
