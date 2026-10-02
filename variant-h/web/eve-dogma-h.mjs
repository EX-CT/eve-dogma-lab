// Loader for the Variant H WebAssembly build (browser or Node, no dependencies).
//
//   import { load } from "./eve-dogma-h.mjs";
//   const h = await load(wasmBytes, datasetBytes);   // ArrayBuffer / Uint8Array (dataset .json.gz or .json)
//   const stats = h.calc(fitRequest);                 // object in, FitStats object out (same as the CLI)
//   h.search("rifter"); h.eftParse(text); h.eftExport(fitRequest);
//
// In a browser: `load(await (await fetch("eve_dogma_h_wasm.wasm")).arrayBuffer(), await (await fetch(url)).arrayBuffer())`.
const enc = new TextEncoder();
const dec = new TextDecoder();

export async function load(wasmBytes, datasetBytes) {
  const { instance } = await WebAssembly.instantiate(wasmBytes, {});
  const x = instance.exports;
  const call = (fn, bytes) => {
    const p = x.h_alloc(bytes.length);
    new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
    const status = fn(p, bytes.length);
    x.h_free(p, bytes.length);
    const out = dec.decode(new Uint8Array(x.memory.buffer, x.h_result_ptr(), x.h_result_len()));
    return { status, out };
  };
  const json = (fn, text) => JSON.parse(call(fn, enc.encode(text)).out);
  const r = call(x.h_load, new Uint8Array(datasetBytes));
  if (r.status !== 0) throw new Error(`dataset: ${r.out}`);
  return {
    calc: (req) => json(x.h_calc, typeof req === "string" ? req : JSON.stringify(req)),
    calcText: (reqText) => call(x.h_calc, enc.encode(reqText)).out,
    search: (query) => json(x.h_search, query),
    eftParse: (text) => json(x.h_eft_parse, text),
    eftExport: (req) => {
      const { status, out } = call(x.h_eft_export, enc.encode(typeof req === "string" ? req : JSON.stringify(req)));
      if (status !== 0) throw new Error(out);
      return out;
    },
  };
}
