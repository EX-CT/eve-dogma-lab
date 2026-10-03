#pragma once
// Pyfa-style graphs (contract eve-dogma-graphs 0.2): request validation, engine primitives and the evaluator.
//   graph_prim.cpp  engine side: built fits -> primitives JSON (eve-dogma-graph-primitives/1)
//   graph_eval.cpp  evaluator: GraphRequest + primitives -> GraphResult (pure functions, no engine access)
#include <simdjson.h>

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "api.hpp"
#include "jsonw.hpp"

namespace evej {

// Small immutable JSON value for the evaluator (requests and primitives). Object members are sorted by key so
// lookups are a binary search; a missing member is the shared null value.
struct JV {
  enum class T : uint8_t { Null, Bool, Num, Str, Arr, Obj };
  T t = T::Null;
  bool b = false;
  double n = 0;
  std::string s;
  std::vector<JV> a;
  std::vector<std::pair<std::string, JV>> o;

  static const JV& nil();
  static JV from(const simdjson::dom::element& e);

  bool is_null() const { return t == T::Null; }
  bool is_num() const { return t == T::Num; }
  bool is_str() const { return t == T::Str; }
  bool is_arr() const { return t == T::Arr; }
  bool is_obj() const { return t == T::Obj; }
  const JV& operator[](std::string_view k) const;
  const JV& operator[](size_t i) const { return i < a.size() ? a[i] : nil(); }
  bool has(std::string_view k) const { return !(*this)[k].is_null(); }
  // `v ?? d` for numbers (null, missing or non-number -> d)
  double num(double d = 0) const { return t == T::Num ? n : d; }
  std::optional<double> opt() const { return t == T::Num ? std::optional<double>(n) : std::nullopt; }
  std::string_view str(std::string_view d = "") const { return t == T::Str ? std::string_view(s) : d; }
  // JS truthiness
  bool truthy() const {
    switch (t) {
      case T::Null: return false;
      case T::Bool: return b;
      case T::Num: return n != 0 && n == n;
      case T::Str: return !s.empty();
      default: return true;
    }
  }
  size_t size() const { return t == T::Arr ? a.size() : o.size(); }
};

using Series = std::vector<std::optional<double>>;

struct GraphError {
  std::string code, message, path;
};

// Contract validation (rules in contract order); nullopt if the request may go to the engine.
std::optional<GraphError> graph_validate(const simdjson::dom::element& req);
// Engine primitives for one request (writes the primitives object, or an error object and returns false).
bool graph_primitives(Worker& wk, const simdjson::dom::element& req, JW& w);
// GraphRequest + primitives -> GraphResult object written to w (or an error object; returns false).
bool graph_evaluate(const JV& req, const JV& prim, JW& w);
// One complete graph request: JSON text -> GraphResult or {"error": ...} in w.
bool graph_json(Worker& wk, std::string_view request, JW& w);
bool graph_element(Worker& wk, const simdjson::dom::element& req, JW& w);

}  // namespace evej
