//! WebAssembly build of the Variant H engine (target `wasm32-unknown-unknown`).
//!
//! Plain C-ABI exports, so any host (the bundled `web/eve-dogma-h.mjs` loader, Node, a browser, wasmtime) can use it
//! without generated glue:
//! - `h_alloc(len) -> ptr` / `h_free(ptr, len)`: buffers the host writes inputs into
//! - `h_load(ptr, len) -> status`: dataset bytes (`dataset-*.json.gz` or plain JSON); 0 = ok
//! - `h_calc` / `h_search` / `h_eft_parse` / `h_eft_export(ptr, len) -> status`: UTF-8 input (FitRequest JSON, a
//!   search query, EFT text, FitRequest JSON); the JSON / text result is read with `h_result_ptr()` +
//!   `h_result_len()` (also on error, where it holds `{"error": ...}`); status 0 = ok, 1 = error
//!
//! The engine code is the same crate as the native CLI: results are identical (see `web/test-node.mjs`).
use eve_dogma_h::{tools, Dataset, FitRequest};
use serde_json::json;
use std::cell::RefCell;

thread_local! {
    static DS: RefCell<Option<Dataset>> = const { RefCell::new(None) };
    static OUT: RefCell<String> = const { RefCell::new(String::new()) };
}

fn set_out(s: String) {
    OUT.with(|o| *o.borrow_mut() = s);
}

fn input<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(ptr, len) } }
}

fn err(code: &str, message: impl Into<String>) -> i32 {
    set_out(json!({"error": {"code": code, "message": message.into(), "path": ""}}).to_string());
    1
}

fn with_ds(f: impl FnOnce(&Dataset) -> i32) -> i32 {
    DS.with(|d| match d.borrow().as_ref() {
        Some(ds) => f(ds),
        None => err("NO_DATASET", "call h_load first"),
    })
}

fn utf8<'a>(ptr: *const u8, len: usize) -> Result<&'a str, i32> {
    std::str::from_utf8(input(ptr, len)).map_err(|e| err("BAD_INPUT", e.to_string()))
}

#[unsafe(no_mangle)]
pub extern "C" fn h_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` must come from `h_alloc(len)` with the same `len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn h_free(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len.max(1)) });
}

#[unsafe(no_mangle)]
pub extern "C" fn h_load(ptr: *const u8, len: usize) -> i32 {
    match Dataset::load_bytes(input(ptr, len)) {
        Ok(ds) => {
            set_out(json!({"ok": true}).to_string());
            DS.with(|d| *d.borrow_mut() = Some(ds));
            0
        }
        Err(e) => err("BAD_DATASET", e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn h_calc(ptr: *const u8, len: usize) -> i32 {
    let Ok(text) = utf8(ptr, len) else { return 1 };
    with_ds(|ds| {
        let out = eve_dogma_h::calc_json(ds, text);
        let failed = out.starts_with("{\"error\"");
        set_out(out);
        failed as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn h_search(ptr: *const u8, len: usize) -> i32 {
    let Ok(q) = utf8(ptr, len) else { return 1 };
    with_ds(|ds| {
        set_out(tools::search(ds, q, None, None).to_string());
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn h_eft_parse(ptr: *const u8, len: usize) -> i32 {
    let Ok(text) = utf8(ptr, len) else { return 1 };
    with_ds(|ds| match tools::eft_parse(ds, text, None) {
        Ok(v) => {
            set_out(v.to_string());
            0
        }
        Err(e) => err("EFT_PARSE", format!("{e:?}")),
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn h_eft_export(ptr: *const u8, len: usize) -> i32 {
    let Ok(text) = utf8(ptr, len) else { return 1 };
    with_ds(|ds| match serde_json::from_str::<FitRequest>(text) {
        Ok(req) => {
            set_out(tools::eft_export(ds, &req, None));
            0
        }
        Err(e) => err("BAD_REQUEST", e.to_string()),
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn h_result_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn h_result_len() -> usize {
    OUT.with(|o| o.borrow().len())
}
