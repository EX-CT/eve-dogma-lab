#pragma once
#include <optional>

#include "engine.hpp"
#include "jsonw.hpp"

namespace evej {
inline constexpr const char* ENGINE_NAME = "eve-dogma-j 0.1.0";
double range_factor(double optimal, double falloff, std::optional<double> distance, bool restricted);
void compute_stats(Fit& fit, const FitRequest& req, JW& out);
// graph primitives of one built fit (ship, items, cap drains, stats), see graphs.cpp
void graph_fit_prim(Fit& fit, const FitRequest& req, JW& out);
void graph_variant_prim(Fit& fit, const FitRequest& req, const std::vector<int32_t>& idx, JW& out);
const char* graph_weapon_kind(Fit& fit, const FitRequest& req, uint32_t item);
}  // namespace evej
