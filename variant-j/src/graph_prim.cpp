// Graph primitives (graphs round 2): everything the graph evaluator (graph_eval.cpp) needs to compute any point of
// a graph, exported once per request: the built source fit (modified attributes, cycle / volley data, capacitor
// drains, stacking inputs of speed and signature, the stats object), the target fit (and its scrammed variant),
// the subwarp speed for warp_time, and one re-built fit per loadable charge for application_profile.
#include "graph.hpp"

#include <simdjson.h>

#include <algorithm>
#include <string>

#include "stats.hpp"

namespace evej {

namespace {

void write_engine_error(JW& w, const EngineError& e) { write_error(w, e.code, e.message, e.path); }

bool build_fit(Fit& f, const FitRequest& r, EngineError& err) {
  f.reset();
  return f.build(r, err);
}

// max velocity with propulsion / cloak / siege / doomsday / cyno / jump portal / entangler modules online and no
// projected effects (the speed a fit aligns and drops out of warp with)
double subwarp_speed(Worker& wk, const FitRequest& req) {
  static const std::string_view groups[] = {"Propulsion Module", "Cloaking Device", "Siege Module", "Super Weapon",
                                            "Cynosural Field Generator", "Jump Portal Generator", "Mass Entanglers"};
  FitRequest r = req;
  r.projected.clear();
  for (auto& m : r.modules) {
    const TypeRec* t = wk.ds.type(m.type_id);
    if (!t) continue;
    const GroupRec* g = wk.ds.group(t->group);
    if (!g || std::find(std::begin(groups), std::end(groups), wk.ds.group_name(*g)) == std::end(groups)) continue;
    if (!m.state || *m.state != State::Offline) m.state = State::Online;
  }
  Fit f(wk.ds, wk.ids);
  EngineError e{};
  if (!f.build(r, e)) return 0;
  return f.get(f.ship, wk.ids.maxVelocity);
}

// the fit with MWDs and MJDs online (a warp scrambler shuts them off); false if nothing changed
bool scrammed(const Worker& wk, const FitRequest& req, FitRequest& out) {
  out = req;
  bool changed = false;
  for (auto& m : out.modules) {
    const TypeRec* t = wk.ds.type(m.type_id);
    if (!t || (m.state && *m.state < State::Active)) continue;
    for (const TEff& e : wk.ds.type_effects(*t))
      if (e.id == wk.ids.e_mwd || e.id == wk.ids.e_mjd) {
        m.state = State::Online;
        changed = true;
        break;
      }
  }
  return changed;
}

bool takes_charges(const Worker& wk, const TypeRec& t) {
  double x;
  for (uint32_t a : wk.ids.chargeGroup)
    if (a && wk.ds.type_attr(t, a, x) && (uint32_t)x != 0) return true;
  return false;
}

// published, on-market charges of the module's charge groups with matching charge size and volume <= capacity
std::vector<uint32_t> valid_charges(const Worker& wk, const TypeRec& mt) {
  const Dataset& ds = wk.ds;
  std::vector<uint32_t> groups, out;
  double x;
  for (uint32_t a : wk.ids.chargeGroup)
    if (a && ds.type_attr(mt, a, x) && (uint32_t)x != 0 && std::find(groups.begin(), groups.end(), (uint32_t)x) == groups.end())
      groups.push_back((uint32_t)x);
  double ms = 0;
  const bool has_size = wk.ids.chargeSize && ds.type_attr(mt, wk.ids.chargeSize, ms);
  for (const TypeRec& ct : ds.types) {
    if (std::find(groups.begin(), groups.end(), ct.group) == groups.end()) continue;
    if (!ct.published || !ct.has_market_group) continue;
    double cs;
    if (has_size && ds.type_attr(ct, wk.ids.chargeSize, cs) && cs != ms) continue;
    if (mt.capacity > 0 && ct.volume > mt.capacity) continue;
    out.push_back(ct.id);
  }
  std::sort(out.begin(), out.end());
  return out;
}

// application_profile: the dominant weapon group (most active charge-using turrets / launchers, ties: first seen)
// and one fit per loadable charge with that charge in all of the group's modules
void charge_variants(Worker& wk, const FitRequest& req, JW& w) {
  Fit f(wk.ds, wk.ids);
  EngineError e{};
  if (!f.build(req, e)) {
    w.null();
    return;
  }
  std::vector<std::pair<uint32_t, std::vector<int32_t>>> groups;
  for (uint32_t i = 0; i < f.items.size(); i++) {
    const Item& it = f.items[i];
    if (it.kind != Kind::Module || it.req_index < 0 || it.state < State::Active) continue;
    std::string_view k = graph_weapon_kind(f, req, i);
    if (k != "turret" && k != "missile") continue;
    if (!takes_charges(wk, *it.t)) continue;
    auto g = std::find_if(groups.begin(), groups.end(), [&](auto& p) { return p.first == it.group; });
    if (g == groups.end()) groups.push_back({it.group, {it.req_index}});
    else g->second.push_back(it.req_index);
  }
  const std::pair<uint32_t, std::vector<int32_t>>* best = nullptr;
  for (auto& g : groups)
    if (!best || g.second.size() > best->second.size()) best = &g;
  w.obj();
  if (!best) {
    w.ki("group", 0).key("modules").arr().end_arr().key("variants").arr().end_arr().end_obj();
    return;
  }
  const std::vector<int32_t>& idx = best->second;
  w.ki("group", best->first).key("modules").arr();
  for (int32_t i : idx) w.i64(i);
  w.end_arr().key("variants").arr();
  const TypeRec* mt = wk.ds.type(req.modules[(size_t)idx[0]].type_id);
  Fit vf(wk.ds, wk.ids);
  for (uint32_t cid : valid_charges(wk, *mt)) {
    FitRequest r = req;
    for (int32_t i : idx) r.modules[(size_t)i].charge_type_id = cid;
    if (!build_fit(vf, r, e)) continue;
    const TypeRec* ct = wk.ds.type(cid);
    w.obj().ki("type_id", cid).ks("name", wk.ds.type_name(*ct)).ki("group", ct->group);
    if (ct->has_meta_group) w.ki("meta_group", ct->meta_group);
    if (ct->has_meta_level) w.ki("meta_level", ct->meta_level);
    w.key("source");
    graph_variant_prim(vf, r, idx, w);
    w.end_obj();
  }
  w.end_arr().end_obj();
}

bool graph_uses_target(std::string_view g) {
  return g == "damage" || g == "application_profile" || g == "ewar" || g == "remote_reps";
}

}  // namespace

bool graph_primitives(Worker& wk, const simdjson::dom::element& root, JW& w) {
  std::string_view graph;
  if (root["graph"].get_string().get(graph)) graph = "";
  simdjson::dom::element fit;
  if (root["fit"].get(fit)) {
    write_error(w, "BAD_REQUEST", "missing fit", "fit");
    return false;
  }
  FitRequest req;
  std::string perr = parse_request(fit, req);
  if (!perr.empty()) {
    write_error(w, "BAD_REQUEST", perr, "fit");
    return false;
  }
  if (!wk.fit) wk.fit = std::make_unique<Fit>(wk.ds, wk.ids);
  EngineError e{};
  if (!build_fit(*wk.fit, req, e)) {
    write_engine_error(w, e);
    return false;
  }
  JW src;
  graph_fit_prim(*wk.fit, req, src);
  // the target is built before anything is written, so a bad target fit gives a clean error object
  std::string target;
  simdjson::dom::element tfit;
  if (graph_uses_target(graph) && !root["target"]["fit"].get(tfit) && !tfit.is_null()) {
    FitRequest treq;
    perr = parse_request(tfit, treq);
    if (!perr.empty()) {
      write_error(w, "BAD_REQUEST", perr, "target.fit");
      return false;
    }
    Fit tf(wk.ds, wk.ids);
    if (!tf.build(treq, e)) {
      write_engine_error(w, e);
      return false;
    }
    JW t;
    t.obj().key("normal");
    graph_fit_prim(tf, treq, t);
    FitRequest sreq;
    if (scrammed(wk, treq, sreq) && build_fit(tf, sreq, e)) {
      t.key("scrammed");
      graph_fit_prim(tf, sreq, t);
    }
    t.end_obj();
    target = std::move(t.s);
  }
  w.obj().ks("schema", "eve-dogma-graph-primitives/1").ks("engine", ENGINE_NAME).key("source").raw(src.s);
  if (graph == "warp_time") w.key("subwarp_speed").num_raw(subwarp_speed(wk, req));
  if (graph == "application_profile") {
    w.key("charges");
    charge_variants(wk, req, w);
  }
  if (!target.empty()) w.key("target").raw(target);
  w.end_obj();
  return true;
}

}  // namespace evej
