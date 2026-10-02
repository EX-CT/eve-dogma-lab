//! EFT export, port of Pyfa `service/port/eft.py exportEft` (all options on) for the fit as Pyfa's GUI holds it
//! after `Fit.fill()` (empty-slot lines), contract 1.4.1 ruling 4. GPL-3.0-or-later (derived from Pyfa).
use crate::data::{Dataset, TypeInfo};
use crate::eos::cx::Fit;
use crate::eos::fit::{infer_slot, BuildError};
use crate::eos::stats::float_unerr;
use crate::request::{FitRequest, Mutation, SlotReq, StateReq};

/// Pyfa SLOT_ORDER and FittingSlot names
const SLOT_ORDER: [(SlotReq, &str, &str); 6] = [
    (SlotReq::Low, "Low", "lowSlots"),
    (SlotReq::Mid, "Med", "medSlots"),
    (SlotReq::High, "High", "hiSlots"),
    (SlotReq::Rig, "Rig", "rigSlots"),
    (SlotReq::Subsystem, "Subsystem", "maxSubSystems"),
    (SlotReq::Service, "Service", "serviceSlots"),
];

/// exportDrones DRONE_ORDER, by market group id (Pyfa eve.db invmarketgroups names)
fn drone_order(mg: Option<u32>) -> usize {
    match mg {
        Some(837) | Some(1531) => 0,  // Light Scout Drones
        Some(3881) => 1,              // Light Hybrid Drones
        Some(838) | Some(1532) => 2,  // Medium Scout Drones
        Some(3882) => 3,              // Medium Hybrid Drones
        Some(839) | Some(359) => 4,   // Heavy Attack Drones
        Some(3883) => 5,              // Heavy Hybrid Drones
        Some(911) | Some(1533) => 6,  // Sentry Drones
        Some(843) | Some(1586) => 7,  // Combat Utility Drones
        Some(841) | Some(1029) => 8,  // Electronic Warfare Drones
        Some(842) | Some(1030) => 9,  // Logistic Drones
        Some(158) | Some(358) => 10,  // Mining Drones
        Some(1643) | Some(1646) => 11, // Salvage Drones
        _ => 12,
    }
}

const FIGHTER_ORDER: [&str; 6] = ["Light Fighter", "Structure Light Fighter", "Heavy Fighter", "Structure Heavy Fighter", "Support Fighter", "Structure Support Fighter"];

/// Python repr() of a float
pub fn py_repr(v: f64) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    // shortest round-trip digits
    let e = format!("{:e}", v); // e.g. "-1.2345e3"
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let sign = if neg { "-" } else { "" };
    if (-4..16).contains(&exp) {
        let n = digits.len() as i32;
        let s = if exp >= 0 {
            if n <= exp + 1 {
                format!("{}{}.0", digits, "0".repeat((exp + 1 - n) as usize))
            } else {
                format!("{}.{}", &digits[..(exp + 1) as usize], &digits[(exp + 1) as usize..])
            }
        } else {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        };
        format!("{sign}{s}")
    } else {
        let m = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits.clone() };
        let es = if exp < 0 { format!("-{:02}", -exp) } else { format!("+{:02}", exp) };
        format!("{sign}{m}e{es}")
    }
}

fn name_of<'a>(ds: &'a Dataset, id: u32) -> &'a str {
    ds.types.get(&id).map(|t| t.name.as_str()).unwrap_or("")
}

/// renderMutant: `[N] Base`, `  Mutaplasmid`, `  attr value, ...` (names sorted, floatUnerr, Python repr)
fn render_mutant(ds: &Dataset, r: usize, m: &Mutation, out_type: Option<&TypeInfo>) -> String {
    let base = ds.types.get(&m.base_type_id);
    let mut lines = vec![format!("[{}] {}", r, name_of(ds, m.base_type_id))];
    let mid = m.mutaplasmid_type_id.unwrap_or(0);
    lines.push(format!("  {}", name_of(ds, mid)));
    let mut attrs: Vec<(String, f64)> = Vec::new();
    if let Some(mu) = ds.mutaplasmids.get(&mid) {
        for (a, lo, hi) in &mu.attrs {
            // Pyfa Module/Drone.__init__ builds mutators only for attributes the item has
            let has = out_type.map(|t| t.attr(*a).is_some()).unwrap_or(false) || base.map(|t| t.attr(*a).is_some()).unwrap_or(false);
            if !has {
                continue;
            }
            let bv = base.and_then(|t| t.attr(*a)).unwrap_or(0.0);
            let mut v = m.attributes.get(&a.to_string()).copied().unwrap_or(bv);
            if bv == 0.0 {
                v = 0.0;
            } else {
                let (lo, hi) = (crate::eos::stats::py_round_digits(*lo, 3), crate::eos::stats::py_round_digits(*hi, 3));
                let r = v / bv;
                if !(lo <= r && r <= hi) {
                    let (a1, b1) = (lo * bv, hi * bv);
                    v = v.max(a1.min(b1)).min(a1.max(b1));
                }
            }
            let name = ds.attrs.get(a).map(|x| x.name.clone()).unwrap_or_default();
            attrs.push((name, v));
        }
    }
    attrs.sort_by(|x, y| x.0.cmp(&y.0));
    lines.push(format!("  {}", attrs.iter().map(|(n, v)| format!("{} {}", n, py_repr(float_unerr(*v)))).collect::<Vec<_>>().join(", ")));
    lines.join("\n")
}

fn mutated_type<'a>(ds: &'a Dataset, m: &Mutation) -> Option<&'a TypeInfo> {
    let mu = ds.mutaplasmids.get(&m.mutaplasmid_type_id?)?;
    let out = mu.mapping.iter().find(|x| x.0.contains(&m.base_type_id)).map(|x| x.1)?;
    ds.types.get(&out)
}

pub fn export(ds: &Dataset, req: &FitRequest, name: &str) -> Result<String, BuildError> {
    let mut fit = Fit::build(ds, req)?;
    fit.calculate(&[]);
    let ship = fit.ship;
    let ship_name = &fit.items[ship].t.name;
    let header = format!("[{}, {}]", ship_name, name);
    let mut sections: Vec<String> = Vec::new();
    let mut mutants: Vec<String> = Vec::new();

    // Section 1: modules by rack, then [Empty X slot] lines for free slots (Fit.fill)
    let mut racks: Vec<String> = Vec::new();
    for (slot, sname, attr) in SLOT_ORDER {
        let mut lines: Vec<String> = Vec::new();
        let mut used = 0usize;
        for m in &req.modules {
            let Some(t) = ds.types.get(&m.type_id) else { continue };
            let t_eff = m.mutation.as_ref().and_then(|mu| mutated_type(ds, mu)).unwrap_or(t);
            if infer_slot(t_eff) != Some(slot) {
                continue;
            }
            used += 1;
            let mut s = match &m.mutation {
                Some(mu) => name_of(ds, mu.base_type_id).to_string(),
                None => t.name.clone(),
            };
            let offline = if matches!(m.state, Some(StateReq::Offline)) { " /offline" } else { "" };
            let suffix = match &m.mutation {
                Some(mu) => {
                    mutants.push(render_mutant(ds, mutants.len() + 1, mu, Some(t_eff)));
                    format!(" [{}]", mutants.len())
                }
                None => String::new(),
            };
            if let Some(c) = m.charge_type_id {
                s = format!("{}, {}", s, name_of(ds, c));
            }
            lines.push(format!("{s}{offline}{suffix}"));
        }
        let total = fit.g(ship, attr);
        let free = (total - used as f64) as i64;
        for _ in 0..free.max(0) {
            lines.push(format!("[Empty {} slot]", sname));
        }
        if !lines.is_empty() {
            racks.push(lines.join("\n"));
        }
    }
    if !racks.is_empty() {
        sections.push(racks.join("\n\n"));
    }

    // Section 2: drones (DRONE_ORDER, unmutated first, fullName), fighters
    let mut minion: Vec<String> = Vec::new();
    let mut drones: Vec<(usize, bool, String, String, Option<&Mutation>, u32)> = Vec::new();
    for d in &req.drones {
        let (base_id, mutated) = match &d.mutation {
            Some(mu) => (mu.base_type_id, true),
            None => (d.type_id, false),
        };
        let bt = ds.types.get(&base_id);
        let mg = bt.and_then(|t| t.market_group.or_else(|| t.variation_parent.and_then(|p| ds.types.get(&p)).and_then(|p| p.market_group)));
        let bname = name_of(ds, base_id).to_string();
        let full = match &d.mutation {
            Some(mu) => {
                // MutatedMixin.fullName: "<mutaplasmid short name> <base name>"
                let mn = name_of(ds, mu.mutaplasmid_type_id.unwrap_or(0));
                let short = mn.split(' ').next().unwrap_or("");
                if short != mn { format!("{} {}", short, bname) } else { name_of(ds, d.type_id).to_string() }
            }
            None => name_of(ds, d.type_id).to_string(),
        };
        drones.push((drone_order(mg), mutated, full, bname, d.mutation.as_ref(), d.quantity));
    }
    drones.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    let mut dl: Vec<String> = Vec::new();
    for (_, _, _, bname, mu, q) in &drones {
        let suffix = match mu {
            Some(mu) => {
                let ot = mutated_type(ds, mu);
                mutants.push(render_mutant(ds, mutants.len() + 1, mu, ot));
                format!(" [{}]", mutants.len())
            }
            None => String::new(),
        };
        dl.push(format!("{} x{}{}", bname, q, suffix));
    }
    if !dl.is_empty() {
        minion.push(dl.join("\n"));
    }
    let mut fl: Vec<(usize, String, u32)> = Vec::new();
    for &f in &fit.fighters {
        let t = fit.items[f].t;
        let gname = ds.group_name(t.group);
        let ord = FIGHTER_ORDER.iter().position(|x| *x == gname).unwrap_or(FIGHTER_ORDER.len());
        fl.push((ord, t.name.clone(), fit.items[f].amount));
    }
    fl.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    if !fl.is_empty() {
        minion.push(fl.iter().map(|(_, n, a)| format!("{} x{}", n, a)).collect::<Vec<_>>().join("\n"));
    }
    if !minion.is_empty() {
        sections.push(minion.join("\n\n"));
    }

    // Section 3: implants (by implantness), boosters (by boosterness)
    let mut chr: Vec<String> = Vec::new();
    let slot_attr = |id: u32, a: &str| -> i64 { ds.types.get(&id).and_then(|t| t.attr(ds.attr_id(a))).unwrap_or(0.0) as i64 };
    let mut imps: Vec<u32> = req.implants.clone();
    imps.sort_by_key(|&i| slot_attr(i, "implantness"));
    if !imps.is_empty() {
        chr.push(imps.iter().map(|&i| name_of(ds, i).to_string()).collect::<Vec<_>>().join("\n"));
    }
    let mut boos: Vec<u32> = req.boosters.iter().map(|b| b.type_id).collect();
    boos.sort_by_key(|&i| slot_attr(i, "boosterness"));
    if !boos.is_empty() {
        chr.push(boos.iter().map(|&i| name_of(ds, i).to_string()).collect::<Vec<_>>().join("\n"));
    }
    if !chr.is_empty() {
        sections.push(chr.join("\n\n"));
    }

    // Section 4: cargo by (category name, group name, type name)
    let mut cargo: Vec<(String, String, String, u32)> = req
        .cargo
        .iter()
        .map(|c| {
            let t = ds.types.get(&c.type_id);
            let g = t.map(|t| t.group).unwrap_or(0);
            let cat = t.map(|t| t.category).unwrap_or(0);
            (ds.category_names.get(&cat).cloned().unwrap_or_default(), ds.group_name(g).to_string(), name_of(ds, c.type_id).to_string(), c.quantity)
        })
        .collect();
    cargo.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
    if !cargo.is_empty() {
        sections.push(cargo.iter().map(|c| format!("{} x{}", c.2, c.3)).collect::<Vec<_>>().join("\n"));
    }

    // Section 5: mutations
    if !mutants.is_empty() {
        sections.push(mutants.join("\n"));
    }
    Ok(format!("{}\n\n{}", header, sections.join("\n\n\n")))
}
