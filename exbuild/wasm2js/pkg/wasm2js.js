import * as wasm from "./wasm2js_bg.wasm.js";
export * from "./wasm2js_bg.js";
import { __wbg_set_wasm } from "./wasm2js_bg.js";
__wbg_set_wasm(wasm);
wasm.__wbindgen_start();
