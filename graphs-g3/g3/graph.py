"""GraphRequest -> GraphResult dispatcher (contract CONTRACT-GRAPHS 0.1)."""
import math

import numpy as np

from . import simple
from .ctx import FitCache, GraphError

AXES = {
    "damage": {"distance_m": ("dps", "volley", "damage"), "time_s": ("dps", "volley", "damage"),
               "tgt_speed_mps": ("dps", "volley", "damage"), "tgt_sig_m": ("dps", "volley", "damage")},
    "application_profile": {"distance_m": ("dps", "volley")},
    "ewar": {"distance_m": tuple(simple.EWAR_Y)},
    "remote_reps": {"distance_m": ("rps", "total"), "time_s": ("rps", "total")},
    "capacitor": {"time_s": ("cap_gj", "cap_regen_gj_s"), "cap_pct": ("cap_gj", "cap_regen_gj_s")},
    "shield_regen": {"time_s": ("shield_hp", "shield_regen_hp_s"), "shield_pct": ("shield_hp", "shield_regen_hp_s")},
    "mobility": {"time_s": ("speed_mps", "distance_m", "momentum_kg_mps", "bump_speed_mps", "bump_distance_m")},
    "warp_time": {"distance_m": ("time_s",)},
    "lock_time": {"tgt_sig_m": ("time_s",)},
}
SETTINGS_DEFAULT = {"ignore_resists": True, "apply_projected": True, "ignore_lock_range": True,
                    "ignore_drone_control_range": False, "mobile_drone_mode": "auto"}


def _err(code, msg, path=""):
    return {"error": {"code": code, "message": msg, "path": path}}


def _num(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(v)


class Engine:
    def __init__(self, ds, cache=True, dataset_path=None):
        self.ds = ds
        self.dataset_path = dataset_path
        self.cache = FitCache(ds, enabled=cache)

    def graph(self, req):
        try:
            return self._graph(req)
        except GraphError as e:
            return _err(e.code, e.message, e.path)
        except Exception as e:  # noqa: BLE001 - one bad request must not end a batch / RPC session
            return _err("INTERNAL", f"{type(e).__name__}: {e}")

    def _graph(self, req):
        if not isinstance(req, dict):
            raise GraphError("BAD_REQUEST", "GraphRequest must be an object", "")
        sv = req.get("schema_version", 1)
        if sv != 1:
            raise GraphError("UNSUPPORTED_SCHEMA", f"schema_version {sv!r} not supported", "/schema_version")
        gname = req.get("graph")
        if gname not in AXES:
            raise GraphError("UNKNOWN_GRAPH", f"unknown graph {gname!r}", "/graph")
        x = req.get("x")
        if not isinstance(x, dict):
            raise GraphError("BAD_REQUEST", "x must be an object", "/x")
        axis = x.get("axis")
        if axis not in AXES[gname]:
            raise GraphError("BAD_AXIS", f"x axis {axis!r} not valid for graph {gname}", "/x/axis")
        vals = x.get("values")
        if not isinstance(vals, list) or not all(_num(v) for v in vals):
            raise GraphError("BAD_REQUEST", "x.values must be a list of finite numbers", "/x/values")
        ys = req.get("y")
        if isinstance(ys, str):
            ys = [ys]
        if not isinstance(ys, list) or not ys:
            raise GraphError("BAD_AXIS", "y must be a non-empty list", "/y")
        for k, y in enumerate(ys):
            if y not in AXES[gname][axis]:
                raise GraphError("BAD_AXIS", f"y series {y!r} not valid for graph {gname} / x {axis}", f"/y/{k}")
        params = req.get("params") or {}
        if not isinstance(params, dict):
            raise GraphError("BAD_REQUEST", "params must be an object", "/params")
        settings = dict(SETTINGS_DEFAULT)
        s = req.get("settings") or {}
        if not isinstance(s, dict):
            raise GraphError("BAD_REQUEST", "settings must be an object", "/settings")
        for k, v in s.items():
            if k in settings and v is not None:
                settings[k] = v
        fit = req.get("fit")
        if not isinstance(fit, dict):
            raise GraphError("BAD_REQUEST", "fit must be a FitRequest object", "/fit")
        c = self.cache.get(fit)
        xs = np.asarray(vals, dtype=float)
        if gname == "lock_time":
            out = simple.lock_time(self, fit, c, xs, ys, params, settings)
        elif gname == "warp_time":
            out = simple.warp_time(self, fit, c, xs, ys, params, settings)
        elif gname == "mobility":
            out = simple.mobility(self, fit, c, xs, ys, params, settings)
        elif gname == "shield_regen":
            out = simple.shield_regen(self, fit, c, xs, ys, params, settings, axis)
        elif gname == "capacitor":
            out = simple.capacitor(self, fit, c, xs, ys, params, settings, axis)
        elif gname == "ewar":
            out = simple.ewar(self, fit, c, xs, ys, params, settings)
        elif gname == "remote_reps":
            out = simple.remote_reps(self, fit, c, xs, ys, params, settings, axis)
        else:
            from . import damage
            out = damage.run(self, req, c, xs, ys, params, settings, gname, axis)
        series = {}
        for y in ys:
            a = out[y]
            a = np.asarray(a, dtype=float)
            lst = a.tolist()
            if not np.isfinite(a).all():
                for k in np.flatnonzero(~np.isfinite(a)).tolist():
                    lst[k] = None
            series[y] = lst
        for k, v in out.items():
            if k.endswith("_charge_type_id"):
                series[k] = v
        res = {"graph": gname, "x_axis": axis, "x": vals, "series": series}
        meta = out.get("_meta")
        if meta:
            res["meta"] = meta
        return res
