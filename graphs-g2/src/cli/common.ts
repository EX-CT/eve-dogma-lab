// Shared by graph-batch, graph and serve-stdio: contract validation before the engine, evaluation after it.
import { evaluate, GraphError, validate } from "../evaluator/index.js";

export function errObj(e: any): { error: { code: string; message: string; path: string } } {
  if (e instanceof GraphError) return { error: { code: e.code, message: e.message, path: e.path } };
  if (e instanceof SyntaxError) return { error: { code: "BAD_REQUEST", message: `invalid JSON: ${e.message}`, path: "" } };
  return { error: { code: "INTERNAL", message: String(e?.message ?? e), path: "" } };
}

/** null when the request may go to the engine, else the error object (the first failing rule wins). */
export function precheck(req: unknown): ReturnType<typeof errObj> | null {
  try {
    validate(req);
    return null;
  } catch (e) {
    return errObj(e);
  }
}

/** engine primitives line (or object) + request -> GraphResult or error object */
export function finish(req: any, prim: any): any {
  try {
    if (typeof prim === "string") prim = JSON.parse(prim);
    if (prim?.error) return { error: prim.error };
    return evaluate(req, prim);
  } catch (e) {
    return errObj(e);
  }
}

export function defaultEngine(): string {
  return new URL("../../../variant-c/bin/eve-dogma-go", import.meta.url).pathname;
}

export function arg(n: string): string | undefined {
  const i = process.argv.indexOf(n);
  return i >= 0 ? process.argv[i + 1] : undefined;
}
