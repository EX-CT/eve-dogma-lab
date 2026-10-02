#!/usr/bin/env python3
"""Transpile Pyfa's eos/effects.py (GPL-3.0-or-later) into Rust (src/generated/effects.rs).

Each `class EffectN(BaseEffect)` becomes metadata (runTime, type, grouped, dealsDamage) plus, when its
handler only uses the supported subset of Pyfa's effect API, a Rust function with the same statements in
the same order. Names (attributes, skills, groups) are resolved to ids from the EXCT dataset at
generation time. Handlers the transpiler cannot express are listed in UNTRANSLATED (reason) and are either
hand-ported in src/eos/custom.rs or are no-ops for this engine's outputs.

usage: pyfa2rs.py PYFA_EFFECTS_PY DATASET_JSON_GZ OUT_RS
"""
import ast, gzip, json, sys, re

EFFECTS_PY, DATASET, OUT = sys.argv[1:4]
FIT_PY = EFFECTS_PY.replace("effects.py", "saveddata/fit.py")
ds = json.load(gzip.open(DATASET))
ATTR = {v["name"]: int(k) for k, v in ds["attributes"].items()}
TYPE_BY_NAME = {}
for k, v in sorted(ds["types"].items(), key=lambda kv: int(kv[0])):
    TYPE_BY_NAME.setdefault(v["name"], int(k))
GROUP_BY_NAME = {}
for k, v in sorted(ds["groups"].items(), key=lambda kv: int(kv[0])):
    GROUP_BY_NAME.setdefault(v["name"], int(k))
EXTRA_ATTRS = {}  # unknown attribute names -> synthetic ids
EXTRA_BASE = 1_000_000
MISSING_SKILLS, MISSING_GROUPS = set(), set()
FIT_EXTRA = {}  # fit.extraAttributes names -> synthetic ids too


def attr_id(name):
    if name in ATTR:
        return ATTR[name]
    if name not in EXTRA_ATTRS:
        EXTRA_ATTRS[name] = EXTRA_BASE + len(EXTRA_ATTRS)
    return EXTRA_ATTRS[name]


def type_id(name):
    if name in TYPE_BY_NAME:
        return TYPE_BY_NAME[name]
    MISSING_SKILLS.add(name)
    return 0xFFFFFFFF


def group_id(name):
    if name in GROUP_BY_NAME:
        return GROUP_BY_NAME[name]
    MISSING_GROUPS.add(name)
    return 0xFFFFFFFF


class Untranslatable(Exception):
    pass


# ---------------------------------------------------------------- value kinds
class V:
    """kind: num | bool | str | tuple | item | list | none | strsel | ctx | kwargs | fit | extra | mad | typ | group"""
    def __init__(self, kind, code=None, const=None, sub=None):
        self.kind, self.code, self.const, self.sub = kind, code, const, sub

    def __repr__(self):
        return f"V({self.kind},{self.code},{self.const})"


def num(code):
    return V("num", code)


def boolean(code):
    return V("bool", code)


def const(v):
    if isinstance(v, bool):
        return V("bool", "true" if v else "false", v)
    if isinstance(v, (int, float)):
        return V("num", f64(v), v)
    if isinstance(v, str):
        return V("str", None, v)
    if isinstance(v, tuple):
        return V("tuple", None, v)
    if v is None:
        return V("none", None, None)
    raise Untranslatable(f"const {v!r}")


def f64(v):
    s = repr(float(v))
    if s == "inf":
        return "f64::INFINITY"
    return s + ("" if "." in s or "e" in s else ".0") if not s.endswith(".0") else s


def pyval(v):
    if v.const is not None or v.kind == "none":
        return v.const
    raise Untranslatable(f"not constant: {v}")


def as_num(v):
    if v.kind == "num":
        return v.code
    if v.kind == "bool":
        return f"(({v.code}) as i32 as f64)"
    if v.kind == "none":
        return "0.0"
    raise Untranslatable(f"num expected, got {v.kind}")


def truth(v):
    if v.kind == "bool":
        return v.code
    if v.kind == "num":
        return f"(({v.code}) != 0.0)"
    if v.kind == "none":
        return "false"
    if v.kind == "item":
        return f"(({v.code}) != NONE)"
    if v.kind == "str":
        return "true" if v.const else "false"
    if v.kind == "tuple":
        return "true" if v.const else "false"
    raise Untranslatable(f"truth of {v.kind}")


LISTS = {"modules": "L::Modules", "drones": "L::Drones", "fighters": "L::Fighters", "appliedImplants": "L::Implants",
         "implants": "L::Implants", "boosters": "L::Boosters"}
CTX = {"skill", "implant", "booster", "ship", "module", "moduleCharge", "drone", "fighter", "projected", "droneCharge",
       "commandRun", "system", "structure"}
OPS = {"boostItemAttr": "Op::Boost", "multiplyItemAttr": "Op::Multiply", "increaseItemAttr": "Op::Increase",
       "forceItemAttr": "Op::Force", "preAssignItemAttr": "Op::PreAssign"}
COPS = {k.replace("Item", "Charge"): v for k, v in OPS.items()}
FOPS = {"filteredItemBoost": ("Op::Boost", False), "filteredItemMultiply": ("Op::Multiply", False),
        "filteredItemIncrease": ("Op::Increase", False), "filteredItemForce": ("Op::Force", False),
        "filteredItemPreAssign": ("Op::PreAssign", False),
        "filteredChargeBoost": ("Op::Boost", True), "filteredChargeMultiply": ("Op::Multiply", True),
        "filteredChargeIncrease": ("Op::Increase", True), "filteredChargeForce": ("Op::Force", True),
        "filteredChargePreAssign": ("Op::PreAssign", True)}
PEN_GROUPS = {"default": 0, "preMul": 1, "postMul": 2, "postDiv": 3, "postPerc": 4, "postPercent": 5,
              "cloakingScanResolutionMultiplier": 6}


class Fn:
    def __init__(self, cls_consts, self_name, params):
        self.env = {}
        self.cls = cls_consts
        self.self_name = self_name
        self.params = params
        self.lines = []
        self.indent = 1
        self.tmp = 0
        self.declared = {}

    def emit(self, s):
        self.lines.append("    " * self.indent + s)

    def fresh(self):
        self.tmp += 1
        return f"t{self.tmp}"

    # ------------------------------------------------------------ expressions
    def expr(self, n):
        m = getattr(self, "x_" + type(n).__name__, None)
        if m is None:
            raise Untranslatable(f"expr {type(n).__name__}: {ast.unparse(n)[:60]}")
        return m(n)

    def x_Constant(self, n):
        return const(n.value)

    def x_Tuple(self, n):
        return V("tuple", None, tuple(pyval(self.expr(e)) for e in n.elts))

    x_List = x_Tuple

    def x_Name(self, n):
        nm = n.id
        if nm in self.env:
            return self.env[nm]
        if nm == self.self_name:
            return V("item", "me")
        if nm == "fit":
            return V("fit")
        if nm == "context":
            return V("ctx")
        if nm == "kwargs":
            return V("kwargs")
        if nm == "projectionRange":
            return V("range", "cx.proj_range")
        if nm in ("True", "False"):
            return const(nm == "True")
        if nm == "None":
            return const(None)
        raise Untranslatable(f"name {nm}")

    def x_JoinedStr(self, n):
        parts = []
        sel = None
        for v in n.values:
            if isinstance(v, ast.Constant):
                parts.append(str(v.value))
            else:
                x = self.expr(v.value)
                if x.kind == "strsel":
                    raise Untranslatable("fstring strsel")
                parts.append(str(pyval(x)))
        return const("".join(parts))

    def x_BinOp(self, n):
        a, b = self.expr(n.left), self.expr(n.right)
        if isinstance(n.op, ast.Mod) and a.kind == "str":
            args = pyval(b) if b.kind == "tuple" else (pyval(b),)
            return const(a.const % args)
        if a.kind == "str" and b.kind == "str" and isinstance(n.op, ast.Add):
            return const(a.const + b.const)
        if a.const is not None and b.const is not None and a.kind == "num" and b.kind == "num":
            ops = {ast.Add: lambda x, y: x + y, ast.Sub: lambda x, y: x - y, ast.Mult: lambda x, y: x * y,
                   ast.Div: lambda x, y: x / y, ast.Pow: lambda x, y: x ** y}
            if type(n.op) in ops:
                r = ops[type(n.op)](a.const, b.const)
                return const(r if isinstance(r, int) else float(r))
        sym = {ast.Add: "+", ast.Sub: "-", ast.Mult: "*", ast.Div: "/"}.get(type(n.op))
        if sym:
            return num(f"({as_num(a)} {sym} {as_num(b)})")
        if isinstance(n.op, ast.Pow):
            return num(f"({as_num(a)}).powf({as_num(b)})")
        if isinstance(n.op, ast.FloorDiv):
            return num(f"({as_num(a)} / {as_num(b)}).floor()")
        raise Untranslatable(f"binop {ast.unparse(n)[:50]}")

    def x_UnaryOp(self, n):
        a = self.expr(n.operand)
        if isinstance(n.op, ast.USub):
            if a.const is not None and a.kind == "num":
                return const(-a.const)
            return num(f"(-{as_num(a)})")
        if isinstance(n.op, ast.Not):
            if a.const is not None and a.kind in ("bool", "str", "tuple"):
                return const(not a.const)
            return boolean(f"(!{truth(a)})")
        raise Untranslatable("unary")

    def x_BoolOp(self, n):
        vals = [self.expr(v) for v in n.values]
        if all(v.kind == "bool" for v in vals):
            j = " && " if isinstance(n.op, ast.And) else " || "
            return boolean("(" + j.join(v.code for v in vals) + ")")
        # python `a or b` with numbers (e.g. projectionRange or 0)
        if isinstance(n.op, ast.Or) and len(vals) == 2:
            a, b = vals
            if a.kind == "range":
                return num(f"cx.proj_range.unwrap_or({as_num(b)})")
            if a.kind == "num":
                t = self.fresh()
                return num(f"{{ let {t} = {a.code}; if {t} != 0.0 {{ {t} }} else {{ {as_num(b)} }} }}")
        if all(v.kind in ("bool", "num", "item") for v in vals):
            j = " && " if isinstance(n.op, ast.And) else " || "
            return boolean("(" + j.join(truth(v) for v in vals) + ")")
        raise Untranslatable(f"boolop {ast.unparse(n)[:60]}")

    def x_IfExp(self, n):
        c = self.expr(n.test)
        a, b = self.expr(n.body), self.expr(n.orelse)
        if c.kind == "bool" and c.const is not None:
            return a if c.const else b
        if a.kind in ("num", "none") and b.kind in ("num", "none"):
            return num(f"(if {truth(c)} {{ {as_num(a)} }} else {{ {as_num(b)} }})")
        if a.kind == "bool" and b.kind == "bool":
            return boolean(f"(if {truth(c)} {{ {a.code} }} else {{ {b.code} }})")
        if a.kind == "str" and b.kind == "str":
            return V("strsel", f"(if {truth(c)} {{ 0 }} else {{ 1 }})", None, [a.const, b.const])
        raise Untranslatable("ifexp kinds")

    def x_Compare(self, n):
        if len(n.ops) != 1:
            raise Untranslatable("chained compare")
        op, rhs_n = n.ops[0], n.comparators[0]
        lhs = self.expr(n.left)
        rhs = self.expr(rhs_n)
        if isinstance(op, (ast.In, ast.NotIn)):
            neg = isinstance(op, ast.NotIn)
            if rhs.kind == "ctx":
                c = pyval(lhs)
                if c not in CTX:
                    raise Untranslatable(f"ctx {c}")
                code = f"cx.ctx(Ctx::{c[0].upper()}{c[1:]})"
            elif rhs.kind == "kwargs":
                code = "true" if pyval(lhs) == "effect" else "false"
            elif rhs.kind == "tuple":
                if lhs.kind == "group":
                    code = "(" + " || ".join(f"{lhs.code} == {group_id(g)}" for g in rhs.const) + ")" if rhs.const else "false"
                elif lhs.kind == "typ":
                    code = "(" + " || ".join(f"{lhs.code} == {int(g)}" for g in rhs.const) + ")" if rhs.const else "false"
                elif lhs.kind == "str":
                    code = "true" if lhs.const in rhs.const else "false"
                elif lhs.kind == "num":
                    code = "(" + " || ".join(f"{lhs.code} == {f64(g)}" for g in rhs.const) + ")"
                else:
                    raise Untranslatable("in tuple")
            elif rhs.kind == "mad":
                code = f"cx.mad_contains({rhs.code}, {attr_id(pyval(lhs))})"
            elif rhs.kind == "typeattrs":
                code = f"cx.type_has_attr({rhs.code}, {attr_id(pyval(lhs))})"
            else:
                raise Untranslatable(f"in {rhs.kind}")
            return boolean(f"(!{code})" if neg else code)
        if isinstance(op, (ast.Is, ast.IsNot)):
            neg = isinstance(op, ast.IsNot)
            if rhs.kind != "none":
                raise Untranslatable("is non-None")
            if lhs.kind == "item":
                code = f"({lhs.code} == NONE)"
            elif lhs.kind == "none":
                code = "true"
            elif lhs.kind in ("num", "bool", "str"):
                code = "false"  # attribute values are never None for known attributes
            else:
                raise Untranslatable("is None kind")
            return boolean(f"(!{code})" if neg else code)
        sym = {ast.Eq: "==", ast.NotEq: "!=", ast.Lt: "<", ast.LtE: "<=", ast.Gt: ">", ast.GtE: ">="}[type(op)]
        if lhs.kind == "group" and rhs.kind == "str":
            return boolean(f"({lhs.code} {sym} {group_id(rhs.const)})")
        if lhs.kind == "typ" and rhs.kind == "num":
            return boolean(f"({lhs.code} {sym} {int(rhs.const)})")
        if lhs.kind == "tname" and rhs.kind == "str":
            return boolean(f"({lhs.code} {sym} {type_id(rhs.const)})")
        if lhs.kind == "str" and rhs.kind == "str":
            return const({"==": lhs.const == rhs.const, "!=": lhs.const != rhs.const}[sym])
        if lhs.kind == "strsel" and rhs.kind == "str":
            idx = [i for i, s in enumerate(lhs.sub) if s == rhs.const]
            code = "(" + " || ".join(f"{lhs.code} == {i}" for i in idx) + ")" if idx else "false"
            return boolean(code if sym == "==" else f"(!{code})")
        return boolean(f"({as_num(lhs)} {sym} {as_num(rhs)})")

    def x_Subscript(self, n):
        base = self.expr(n.value)
        idx = None if isinstance(n.slice, ast.Slice) else self.expr(n.slice)
        if base.kind == "extra":
            return num(f"cx.extra({attr_id(pyval(idx))})")
        if base.kind == "str":
            if isinstance(n.slice, ast.Slice):
                lo = pyval(self.expr(n.slice.lower)) if n.slice.lower else None
                hi = pyval(self.expr(n.slice.upper)) if n.slice.upper else None
                return const(base.const[lo:hi])
            return const(base.const[int(pyval(idx))])
        if base.kind == "kwargs" and pyval(idx) == "effect":
            return V("effect", "cx.effect")
        if base.kind == "tuple":
            i = pyval(idx)
            return const(base.const[i])
        raise Untranslatable("subscript")

    def x_NamedExpr(self, n):
        v = self.expr(n.value)
        self.assign_name(n.target.id, v)
        return self.env[n.target.id]

    def x_Attribute(self, n):
        a = n.attr
        if isinstance(n.value, ast.Name) and n.value.id == "cls":
            if a in self.cls:
                return const(self.cls[a])
            raise Untranslatable(f"cls.{a}")
        if isinstance(n.value, ast.Name) and n.value.id == "FittingModuleState":
            return const({"OFFLINE": -1, "ONLINE": 0, "ACTIVE": 1, "OVERHEATED": 2}[a])
        base = self.expr(n.value)
        if base.kind == "fit":
            if a == "ship":
                return V("item", "cx.ship")
            if a == "character":
                return V("item", "cx.chr")
            if a in LISTS:
                return V("list", LISTS[a])
            if a == "extraAttributes":
                return V("extra")
            if a == "scanType":
                return V("strsel", "cx.scan_type()", None, ["Magnetometric", "Ladar", "Radar", "Gravimetric", "Multispectral"])
            if a == "isStructure":
                return boolean("cx.is_structure()")
            if a == "factorReload":
                return boolean("cx.factor_reload()")
            raise Untranslatable(f"fit.{a}")
        if base.kind == "item":
            if a == "level":
                return num(f"cx.level({base.code})")
            if a == "charge":
                return V("item", f"cx.charge_of({base.code})")
            if a == "item":
                return V("itemtype", base.code)
            if a == "ship":
                return V("item", "cx.ship")
            if a == "owner":
                return V("fit")
            if a == "itemModifiedAttributes":
                return V("mad", base.code)
            if a == "amount":
                return num(f"cx.amount({base.code})")
            if a == "amountActive":
                return num(f"cx.amount_active({base.code})")
            if a == "state":
                return num(f"cx.state({base.code})")
            if a == "ID":
                return V("typ", f"cx.type_id({base.code})")
            if a == "name":
                return V("tname", f"cx.type_id({base.code})")
            if a == "group":
                return V("group", f"cx.group_id({base.code})")
            if a == "rahPatternOverride":
                return const(None)
            raise Untranslatable(f"item.{a}")
        if base.kind == "itemtype":
            if a == "group":
                return V("group", f"cx.group_id({base.code})")
            if a == "groupID":
                return V("typ", f"(cx.group_id({base.code}) as u32)")
            if a == "ID":
                return V("typ", f"cx.type_id({base.code})")
            if a == "name":
                return V("tname", f"cx.type_id({base.code})")
            if a == "attributes":
                return V("typeattrs", base.code)
            raise Untranslatable(f"item.item.{a}")
        if base.kind == "group" and a == "name":
            return base
        if base.kind == "strsel":
            raise Untranslatable("strsel attr")
        raise Untranslatable(f"attr {a} of {base.kind}")

    def kwopts(self, kws, extra_pos=None):
        skill, stack, group, post = "0", "false", 0, "false"
        kw = "false"
        for k in kws:
            if k.arg is None:
                kw = "true"
                continue  # **kwargs (effect=effect): resistance handled by the runtime
            v = self.expr(k.value)
            if k.arg == "skill":
                if v.kind == "str":
                    skill = str(type_id(v.const))
                elif v.kind == "item":
                    skill = f"cx.type_id({v.code})"
                elif v.kind == "none":
                    skill = "0"
                else:
                    raise Untranslatable("skill kw")
            elif k.arg == "stackingPenalties":
                stack = truth(v)
            elif k.arg == "penaltyGroup":
                group = PEN_GROUPS[pyval(v)] if v.kind != "none" else 7
            elif k.arg == "position":
                post = "true" if pyval(v) == "post" else "false"
            elif k.arg in ("effect",):
                pass
            else:
                raise Untranslatable(f"kw {k.arg}")
        return f"O {{ skill: {skill}, stack: {stack}, group: {group}, post: {post}, kw: {kw} }}"

    def attr_ref(self, v):
        """attribute name (const or strsel) -> rust u32 expression"""
        if v.kind == "str":
            return str(attr_id(v.const))
        if v.kind == "strsel":
            return "[" + ", ".join(str(attr_id(s)) for s in v.sub) + f"][{v.code}]"
        raise Untranslatable(f"attr name kind {v.kind}")

    def filt(self, lam):
        if not isinstance(lam, ast.Lambda):
            raise Untranslatable("filter not lambda")
        var = lam.args.args[0].arg
        sub = Fn(self.cls, self.self_name, self.params)
        sub.env = dict(self.env)
        sub.env[var] = V("item", "m")
        return sub.filt_expr(lam.body)

    def filt_expr(self, n):
        v = self.expr(n)
        return truth(v)

    def x_Call(self, n):
        f = n.func
        args = n.args
        if isinstance(f, ast.Name):
            nm = f.id
            if nm in ("min", "max") and len(args) == 2:
                a, b = [as_num(self.expr(x)) for x in args]
                return num(f"({a}).{nm}({b})")
            if nm == "abs":
                return num(f"({as_num(self.expr(args[0]))}).abs()")
            if nm == "round":
                return num(f"py_round({as_num(self.expr(args[0]))})")
            if nm == "int":
                return num(f"({as_num(self.expr(args[0]))}).trunc()")
            if nm == "float":
                return self.expr(args[0])
            if nm == "range":
                vals = [int(pyval(self.expr(x))) for x in args]
                return const(tuple(range(*vals)))
            if nm == "calculateRangeFactor":
                kw = {k.arg: self.expr(k.value) for k in n.keywords}
                pos = [self.expr(x) for x in args]
                names = ["srcOptimalRange", "srcFalloffRange", "distance", "restrictedRange"]
                for i, p in enumerate(pos):
                    kw[names[i]] = p
                d = kw["distance"]
                dcode = "cx.proj_range" if d.kind == "range" else f"Some({as_num(d)})"
                rr = truth(kw["restrictedRange"]) if "restrictedRange" in kw else "true"
                return num(f"calc_range_factor({as_num(kw['srcOptimalRange'])}, {as_num(kw['srcFalloffRange'])}, {dcode}, {rr})")
            if nm == "hasattr":
                o, at = self.expr(args[0]), pyval(self.expr(args[1]))
                if o.kind == "item":
                    return boolean(f"cx.has_py_attr({o.code}, \"{at}\")")
                raise Untranslatable("hasattr")
            raise Untranslatable(f"call {nm}")
        if not isinstance(f, ast.Attribute):
            raise Untranslatable("call target")
        meth = f.attr
        if isinstance(f.value, ast.Name) and f.value.id == "ModifiedAttributeDict" and meth == "getResistance":
            return num("cx.resistance()")
        base = self.expr(f.value)
        if base.kind == "str" and meth == "format":
            vals = [self.expr(a) for a in args]
            kwv = {k.arg: self.expr(k.value) for k in n.keywords}
            if any(v.kind == "strsel" for v in vals):
                if len(vals) != 1:
                    raise Untranslatable("format strsel multi")
                s = vals[0]
                return V("strsel", s.code, None, [base.const.format(x) for x in s.sub])
            return const(base.const.format(*[pyval(v) for v in vals], **{k: pyval(v) for k, v in kwv.items()}))
        if base.kind == "str" and meth in ("capitalize", "lower", "upper", "title"):
            return const(getattr(base.const, meth)())
        if base.kind == "item":
            if meth in ("getModifiedItemAttr", "getModifiedChargeAttr", "getItemBaseAttrValue", "getChargeBaseAttrValue"):
                an = self.expr(args[0])
                dflt = as_num(self.expr(args[1])) if len(args) > 1 else None
                fn = {"getModifiedItemAttr": "attr", "getModifiedChargeAttr": "charge_attr",
                      "getItemBaseAttrValue": "base_attr", "getChargeBaseAttrValue": "charge_base_attr"}[meth]
                if an.kind == "str" and an.const not in ATTR and dflt is not None:
                    return num(dflt)
                return num(f"cx.{fn}({base.code}, {self.attr_ref(an)})")
            if meth in OPS or meth in COPS:
                op = OPS.get(meth) or COPS[meth]
                tgt = base.code if meth in OPS else f"cx.charge_of({base.code})"
                an = self.attr_ref(self.expr(args[0]))
                val = as_num(self.expr(args[1])) if len(args) > 1 else "0.0"
                if meth.startswith("increase") and len(args) > 2:
                    raise Untranslatable("increase positional")
                self.emit(f"cx.op({tgt}, {op}, {an}, {val}, {self.kwopts(n.keywords)});")
                return V("none")
            if meth in ("requiresSkill", "getAttribute"):
                return self.x_Call(ast.Call(func=ast.Attribute(value=ast.Attribute(value=f.value, attr="item"), attr=meth), args=args, keywords=n.keywords))
            raise Untranslatable(f"item.{meth}()")
        if base.kind == "itemtype":
            if meth == "requiresSkill":
                s = self.expr(args[0])
                if s.kind == "str":
                    return boolean(f"cx.req_skill({base.code}, {type_id(s.const)})")
                if s.kind == "item":
                    return boolean(f"cx.req_skill({base.code}, cx.type_id({s.code}))")
                raise Untranslatable("requiresSkill arg")
            if meth == "getAttribute":
                return num(f"cx.type_attr({base.code}, {attr_id(pyval(self.expr(args[0])))})")
            raise Untranslatable(f"item.item.{meth}")
        if base.kind == "list":
            if meth in FOPS:
                op, charge = FOPS[meth]
                fl = self.filt(args[0])
                an = self.attr_ref(self.expr(args[1]))
                val = as_num(self.expr(args[2])) if len(args) > 2 else "0.0"
                self.emit(f"cx.filtered({base.code}, {'true' if charge else 'false'}, &|cx: &Cx, m: It| {fl}, {op}, {an}, {val}, {self.kwopts(n.keywords)});")
                return V("none")
            raise Untranslatable(f"list.{meth}")
        if base.kind == "fit":
            if meth == "getPilotSecurity":
                kw = {k.arg: as_num(self.expr(k.value)) for k in n.keywords}
                lo = kw.get("low_limit", "-10.0")
                hi = kw.get("high_limit", "5.0")
                return num(f"cx.pilot_security({lo}, {hi})")
            if meth == "addCommandBonus":
                vals = [self.expr(a) for a in args]
                rt = pyval(vals[4]) if len(vals) > 4 else "normal"
                self.emit(f"cx.add_command_bonus({as_num(vals[0])}, {as_num(vals[1])}, {vals[2].code}, RT_{rt.upper()});")
                return V("none")
            if meth == "getSystemSecurity":
                return num("cx.system_security()")
            raise Untranslatable(f"fit.{meth}")
        if base.kind == "extra":
            if meth == "increase":
                self.emit(f"cx.extra_increase({attr_id(pyval(self.expr(args[0])))}, {as_num(self.expr(args[1]))});")
                return V("none")
            if meth == "boost":
                self.emit(f"cx.extra_boost({attr_id(pyval(self.expr(args[0])))}, {as_num(self.expr(args[1]))});")
                return V("none")
            raise Untranslatable(f"extra.{meth}")
        raise Untranslatable(f"call {meth} on {base.kind}")

    # ------------------------------------------------------------ statements
    def assign_name(self, nm, v):
        if v.kind in ("str", "tuple", "none") or (v.kind in ("num", "bool", "none") and v.const is not None and nm not in self.declared):
            self.env[nm] = v
            return
        if v.kind in ("num", "bool"):
            ty = "f64" if v.kind == "num" else "bool"
            if nm in self.declared and self.declared[nm] != ty:
                raise Untranslatable("retyped var")
            rn = f"v_{nm}"
            if nm in self.declared:
                self.emit(f"{rn} = {v.code};")
            else:
                self.declared[nm] = ty
                self.emit(f"let mut {rn}: {ty} = {v.code};")
            self.env[nm] = V(v.kind, rn)
            return
        if v.kind in ("item", "list", "strsel", "group", "typ"):
            if v.kind == "item" and v.code not in ("me", "cx.ship", "cx.chr"):
                rn = f"v_{nm}"
                self.emit(f"let {rn}: It = {v.code};")
                self.env[nm] = V("item", rn)
            else:
                self.env[nm] = v
            return
        raise Untranslatable(f"assign {v.kind}")

    def stmts(self, body):
        for i, s in enumerate(body):
            if isinstance(s, ast.If) and self.needs_split(s):
                c = self.expr(s.test)
                rest = body[i + 1:]
                saved = (dict(self.env), dict(self.declared))
                self.emit(f"if {truth(c)} {{")
                self.indent += 1
                self.stmts(list(s.body) + rest)
                self.indent -= 1
                self.env, self.declared = dict(saved[0]), dict(saved[1])
                self.emit("} else {")
                self.indent += 1
                self.stmts(list(s.orelse) + rest)
                self.indent -= 1
                self.emit("}")
                self.env, self.declared = saved
                return
            self.stmt(s)

    def needs_split(self, s):
        try:
            c = self.expr(s.test)
        except Untranslatable:
            return False
        if c.const is not None or c.kind in ("str", "tuple", "none"):
            return False
        for b in s.body + s.orelse:
            if isinstance(b, ast.Assign) and len(b.targets) == 1 and isinstance(b.targets[0], ast.Name):
                if isinstance(b.value, ast.Constant) and (b.value.value is None or isinstance(b.value.value, str)):
                    return True
        return False

    def stmt(self, s):
        if isinstance(s, ast.Expr):
            if isinstance(s.value, ast.Constant):
                return
            v = self.expr(s.value)
            if v.kind != "none":
                raise Untranslatable("expr stmt with value")
        elif isinstance(s, ast.Assign):
            if len(s.targets) != 1:
                raise Untranslatable("multi assign")
            t = s.targets[0]
            if isinstance(t, ast.Name):
                self.assign_name(t.id, self.expr(s.value))
            elif isinstance(t, ast.Subscript) and self.expr(t.value).kind == "extra":
                key = pyval(self.expr(t.slice))
                self.emit(f"cx.extra_set({attr_id(key)}, {as_num(self.expr(s.value))});")
            elif isinstance(t, ast.Attribute) and self.expr(t.value).kind == "item":
                it = self.expr(t.value).code
                v = self.expr(s.value)
                if t.attr == "reloadTime":
                    self.emit(f"cx.set_reload_time({it}, {as_num(v)});")
                elif t.attr == "forceReload":
                    self.emit(f"cx.set_force_reload({it}, {truth(v)});")
                else:
                    raise Untranslatable(f"set item.{t.attr}")
            elif isinstance(t, ast.Subscript) and self.expr(t.value).kind == "mad":
                it = self.expr(t.value).code
                key = pyval(self.expr(t.slice))
                self.emit(f"cx.set_intermediary({it}, {attr_id(key)}, {as_num(self.expr(s.value))});")
            elif isinstance(t, ast.Tuple):
                v = self.expr(s.value)
                if v.kind != "tuple":
                    raise Untranslatable("tuple unpack")
                for tn, vv in zip(t.elts, v.const):
                    self.assign_name(tn.id, const(vv))
            else:
                raise Untranslatable("assign target")
        elif isinstance(s, ast.AugAssign):
            if not isinstance(s.target, ast.Name):
                raise Untranslatable("augassign target")
            cur = self.expr(s.target)
            rhs = self.expr(s.value)
            sym = {ast.Add: "+", ast.Sub: "-", ast.Mult: "*", ast.Div: "/"}[type(s.op)]
            nv = num(f"({as_num(cur)} {sym} {as_num(rhs)})")
            if s.target.id not in self.declared:
                self.declared[s.target.id] = "f64"
                self.emit(f"let mut v_{s.target.id}: f64 = {nv.code};")
                self.env[s.target.id] = num(f"v_{s.target.id}")
            else:
                self.emit(f"v_{s.target.id} = {nv.code};")
        elif isinstance(s, ast.If):
            c = self.expr(s.test)
            if c.kind == "bool" and c.const is not None or c.kind in ("str", "tuple", "none"):
                take = bool(c.const) if c.kind != "none" else False
                self.stmts(s.body if take else s.orelse)
                return
            self.hoist(s.body + s.orelse)
            self.emit(f"if {truth(c)} {{")
            self.indent += 1
            self.stmts(s.body)
            self.indent -= 1
            if s.orelse:
                self.emit("} else {")
                self.indent += 1
                self.stmts(s.orelse)
                self.indent -= 1
            self.emit("}")
        elif isinstance(s, ast.For):
            it = self.expr(s.iter)
            if it.kind != "tuple":
                raise Untranslatable(f"for over {it.kind}")
            for val in it.const:
                if isinstance(s.target, ast.Name):
                    self.env[s.target.id] = const(val)
                elif isinstance(s.target, ast.Tuple):
                    for tn, vv in zip(s.target.elts, val):
                        self.env[tn.id] = const(vv)
                else:
                    raise Untranslatable("for target")
                self.stmts(s.body)
        elif isinstance(s, ast.Try):
            self.stmts(s.body)  # pilot security lookups cannot fail here
        elif isinstance(s, ast.Pass):
            pass
        elif isinstance(s, ast.Return):
            if s.value is not None:
                raise Untranslatable("return value")
            self.emit("return;")
        else:
            raise Untranslatable(f"stmt {type(s).__name__}")

    def hoist(self, body):
        """declare numeric vars first assigned inside a branch so they outlive it"""
        for n in body:
            for x in ast.walk(n):
                if isinstance(x, ast.Assign) and len(x.targets) == 1 and isinstance(x.targets[0], ast.Name):
                    nm = x.targets[0].id
                    if nm in self.declared or nm in self.env:
                        continue
                    try:
                        sub = Fn(self.cls, self.self_name, self.params)
                        sub.env = dict(self.env)
                        sub.declared = dict(self.declared)
                        v = sub.expr(x.value)
                    except Untranslatable:
                        continue
                    if v.kind == "num":
                        self.declared[nm] = "f64"
                        self.emit(f"let mut v_{nm}: f64 = 0.0;")
                        self.env[nm] = num(f"v_{nm}")
                    elif v.kind == "bool":
                        self.declared[nm] = "bool"
                        self.emit(f"let mut v_{nm}: bool = false;")
                        self.env[nm] = boolean(f"v_{nm}")


def main():
    src = open(EFFECTS_PY).read()
    tree = ast.parse(src)
    metas, fns, untranslated = [], [], []
    for c in tree.body:
        if not isinstance(c, ast.ClassDef) or not c.name.startswith("Effect"):
            continue
        try:
            eid = int(c.name[6:])
        except ValueError:
            continue
        doc = ast.get_docstring(c) or ""
        name = doc.strip().split("\n")[0].strip()
        cls = {}
        handler = None
        for b in c.body:
            if isinstance(b, ast.Assign) and len(b.targets) == 1 and isinstance(b.targets[0], ast.Name):
                try:
                    cls[b.targets[0].id] = ast.literal_eval(b.value)
                except Exception:
                    pass
            if isinstance(b, ast.FunctionDef) and b.name == "handler":
                handler = b
        rt = cls.get("runTime", "normal")
        ty = cls.get("type", ())
        if isinstance(ty, str):
            ty = (ty,)
        metas.append((eid, name, rt, ty, bool(cls.get("grouped", False)), bool(cls.get("dealsDamage", False)),
                      cls.get("activeByDefault", True), handler is not None, cls.get("prefix", ""), bool(cls.get("hasCharges", False))))
        if handler is None:
            continue
        params = [a.arg for a in handler.args.args]
        if params and params[0] == "cls":
            params = params[1:]
        self_name = params[1]
        fn = Fn(cls, self_name, params)
        try:
            fn.stmts(handler.body)
            fns.append((eid, name, fn.lines))
        except (Untranslatable, KeyError, IndexError, TypeError, ValueError) as e:
            untranslated.append((eid, name, str(e)))
    # ---- Fit.__runCommandBoosts: the per-warfare-buff handlers
    fit_tree = ast.parse(open(FIT_PY).read())
    chain = None
    for node in ast.walk(fit_tree):
        if isinstance(node, ast.FunctionDef) and node.name == "__runCommandBoosts":
            for x in ast.walk(node):
                if isinstance(x, ast.If) and isinstance(x.test, ast.Compare) and ast.unparse(x.test) == "warfareBuffID == 10":
                    chain = x
                    break
    buffs = []
    while chain is not None:
        bid = int(ast.unparse(chain.test.comparators[0]))
        fn = Fn({}, "__none__", [])
        fn.env["self"] = V("fit")
        fn.env["value"] = num("value")
        fn.indent = 2
        try:
            fn.stmts(chain.body)
            buffs.append((bid, fn.lines))
        except Untranslatable as e:
            untranslated.append((100000000 + bid, f"commandBuff{bid}", str(e)))
        nxt = chain.orelse
        chain = nxt[0] if len(nxt) == 1 and isinstance(nxt[0], ast.If) else None
    out = ["// @generated by tools/pyfa2rs.py from Pyfa eos/effects.py -- DO NOT EDIT.",
           "// Pyfa is GPL-3.0-or-later; this file is a mechanical translation of it and carries the same license.",
           "#![allow(unused_mut, unused_variables, unused_parens, clippy::all)]",
           "use crate::eos::cx::*;", ""]
    out.append("pub static META: &[EffMeta] = &[")
    for eid, name, rt, ty, grouped, dd, abd, has, prefix, hc in sorted(metas):
        flags = " | ".join(f"T_{t.upper()}" for t in ty) or "0"
        out.append(f"    EffMeta {{ id: {eid}, name: {json.dumps(name)}, run_time: RT_{rt.upper()}, types: {flags}, grouped: {str(grouped).lower()}, deals_damage: {str(dd).lower()}, active_by_default: {str(bool(abd)).lower()}, has_handler: {str(has).lower()}, prefix: {json.dumps(prefix)}, has_charges: {str(hc).lower()} }},")
    out.append("];")
    out.append("")
    out.append("pub static UNTRANSLATED: &[(u32, &str, &str)] = &[")
    for eid, name, why in sorted(untranslated):
        out.append(f"    ({eid}, {json.dumps(name)}, {json.dumps(why)}),")
    out.append("];")
    out.append("")
    out.append("pub static EXTRA_ATTRS: &[(u32, &str)] = &[")
    for nm, i in sorted(EXTRA_ATTRS.items(), key=lambda kv: kv[1]):
        out.append(f"    ({i}, {json.dumps(nm)}),")
    out.append("];")
    out.append("")
    out.append("/// Run the translated Pyfa handler of `eid`; false if there is none (custom or untranslated).")
    out.append("pub fn run(cx: &mut Cx, eid: u32, me: It) -> bool {")
    out.append("    match eid {")
    for eid, name, lines in sorted(fns):
        out.append(f"        {eid} => e{eid}(cx, me),")
    out.append("        _ => return false,")
    out.append("    }")
    out.append("    true")
    out.append("}")
    out.append("")
    out.append("/// Pyfa Fit.__runCommandBoosts: apply warfare buff `id` with `value` to this fit.")
    out.append("pub fn command_buff(cx: &mut Cx, id: u32, value: f64) -> bool {")
    out.append("    match id {")
    for bid, lines in buffs:
        out.append(f"        {bid} => {{")
        out.extend(lines)
        out.append("        }")
    out.append("        _ => return false,")
    out.append("    }")
    out.append("    true")
    out.append("}")
    out.append("")
    for eid, name, lines in sorted(fns):
        out.append(f"/// {name}")
        out.append(f"fn e{eid}(cx: &mut Cx, me: It) {{")
        out.extend(lines)
        out.append("}")
    open(OUT, "w").write("\n".join(out) + "\n")
    print(f"effects: {len(metas)} classes, {len(fns)} translated, {len(untranslated)} untranslated", file=sys.stderr)
    import collections
    why = collections.Counter(re.sub(r"\d+", "N", u[2])[:50] for u in untranslated)
    for k, v in why.most_common(40):
        print(f"  {v:4d} {k}", file=sys.stderr)
    print(f"  extra attrs: {len(EXTRA_ATTRS)}; missing skills: {sorted(MISSING_SKILLS)[:20]}; missing groups: {sorted(MISSING_GROUPS)[:20]}", file=sys.stderr)


main()
