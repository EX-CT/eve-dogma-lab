"""Vectorised building blocks shared by the graphs."""
import math

import numpy as np

from evedogma_g.stats import float_unerr

NAN = float("nan")
PENALTY = np.exp(-(np.arange(64, dtype=float) ** 2) / 7.1289)


def range_factor(optimal, falloff, d, restricted=True):
    """Pyfa calculateRangeFactor over an array of distances (None = no distance -> 1)"""
    d = np.asarray(d, dtype=float)
    optimal = optimal or 0.0
    falloff = falloff or 0.0
    if falloff > 0:
        r = 0.5 ** ((np.maximum(0.0, d - optimal) / falloff) ** 2)
        if restricted:
            r = np.where(d > optimal + 3 * falloff, 0.0, r)
        return r
    return np.where(d <= optimal, 1.0, 0.0)


def unerr(a):
    """floatUnerr (round to 8 significant digits) elementwise"""
    a = np.asarray(a, dtype=float)
    out = np.empty_like(a)
    flat, of = a.ravel(), out.ravel()
    for k, x in enumerate(flat.tolist()):
        of[k] = float_unerr(x) if math.isfinite(x) else x
    return out


def stack_mult(rows):
    """Pyfa calculateMultiplier for one stacking group, vectorised over points.
    rows: array (n_sources, n_points) of multipliers. Returns array (n_points,)."""
    if rows is None or len(rows) == 0:
        return None
    m = np.asarray(rows, dtype=float)
    out = np.ones(m.shape[1])
    for sel in (m > 1, m < 1):
        part = np.where(sel, m, 1.0)
        dev = -np.abs(part - 1.0)
        o = np.argsort(dev, axis=0, kind="stable")
        srt = np.take_along_axis(part, o, axis=0)
        w = PENALTY[:len(srt)].reshape(-1, 1)
        out = out * np.prod(1.0 + (srt - 1.0) * w, axis=0)
    return out


def regen_amount(cmax, tau_s, c0, t):
    """C·(1 + e^(−5t/τ)·(√(c0/C) − 1))²"""
    return cmax * (1 + np.exp(5 * -np.asarray(t, float) / tau_s) * (math.sqrt(c0 / cmax) - 1)) ** 2


def regen_rate(cmax, tau_s, c):
    c = np.asarray(c, float)
    return 10 * cmax / tau_s * (np.sqrt(c / cmax) - c / cmax)
