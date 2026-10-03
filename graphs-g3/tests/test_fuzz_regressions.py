#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Regressions found by the bench's differential fuzzer (graphs/tools/fuzz_graphs.py), expected values from the
Pyfa graph oracle (graphs/pending). Dataset from EVE_DOGMA_DATASET; skipped when unset."""
import json
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path[:0] = [ROOT, os.path.join(os.path.dirname(ROOT), "variant-g")]


class FuzzRegressions(unittest.TestCase):
    def test_cases(self):
        d = os.environ.get("EVE_DOGMA_DATASET")
        if not d:
            self.skipTest("EVE_DOGMA_DATASET not set")
        from evedogma_g import dataset
        from g3.graph import Engine
        eng = Engine(dataset.load(d), dataset_path=d)
        cases = json.load(open(os.path.join(HERE, "data", "fuzz_regressions.json")))
        for cid, cs in sorted(cases.items()):
            with self.subTest(cid):
                res = eng.graph(cs["request"])
                k = cs["request"]["x"]["values"].index(cs["x0"])
                got = res["series"][cs["y"]][k]
                self.assertLessEqual(abs(got - cs["want"]), max(1e-3, 1e-4 * abs(cs["want"])), (cid, got, cs["want"]))


if __name__ == "__main__":
    unittest.main()
