declare namespace wasm_bindgen {
	/* tslint:disable */
	/* eslint-disable */
	/**
	 * Entry point invoked by `worker.js`, a bit of a hack but see the "TODO" above
	 * about `worker.js` in general.
	 */
	export function child_entry_point(ptr: number): void;
	export class RenderingScene {
	  free(): void;
	  /**
	   * Returns the JS promise object which resolves when the render is complete
	   */
	  promise(): Promise<any>;
	  /**
	   * Return a progressive rendering of the image so far
	   */
	  imageSoFar(): ImageData;
	}
	export class Scene {
	  free(): void;
	  /**
	   * Creates a new scene from the JSON description in `object`, which we
	   * deserialize here into an actual scene.
	   */
	  constructor(object: any);
	  /**
	   * Renders this scene with the provided concurrency and worker pool.
	   *
	   * This will spawn up to `concurrency` workers which are loaded from or
	   * spawned into `pool`. The `RenderingScene` state contains information to
	   * get notifications when the render has completed.
	   */
	  render(concurrency: number, pool: WorkerPool): RenderingScene;
	}
	export class WorkerPool {
	  free(): void;
	  /**
	   * Creates a new `WorkerPool` which immediately creates `initial` workers.
	   *
	   * The pool created here can be used over a long period of time, and it
	   * will be initially primed with `initial` workers. Currently workers are
	   * never released or gc'd until the whole pool is destroyed.
	   *
	   * # Errors
	   *
	   * Returns any error that may happen while a JS web worker is created and a
	   * message is sent to it.
	   */
	  constructor(initial: number);
	}
	
}

declare type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

declare interface InitOutput {
  readonly __wbg_workerpool_free: (a: number, b: number) => void;
  readonly workerpool_new: (a: number) => number[];
  readonly child_entry_point: (a: number) => number[];
  readonly __wbg_scene_free: (a: number, b: number) => void;
  readonly scene_new: (a: number) => number[];
  readonly scene_render: (a: number, b: number, c: number) => number[];
  readonly __wbg_renderingscene_free: (a: number, b: number) => void;
  readonly renderingscene_promise: (a: number) => number;
  readonly renderingscene_imageSoFar: (a: number) => number;
  readonly memory: WebAssembly.Memory;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly __externref_table_alloc: () => number;
  readonly __wbindgen_export_5: WebAssembly.Table;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly __wbindgen_export_7: WebAssembly.Table;
  readonly __externref_table_dealloc: (a: number) => void;
  readonly closure50_externref_shim: (a: number, b: number, c: number) => void;
  readonly closure88_externref_shim: (a: number, b: number, c: number) => void;
  readonly closure161_externref_shim: (a: number, b: number, c: number, d: number) => void;
  readonly __wbindgen_thread_destroy: (a?: number, b?: number, c?: number) => void;
  readonly __wbindgen_start: (a: number) => void;
}

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {{ module_or_path: InitInput | Promise<InitInput>, memory?: WebAssembly.Memory, thread_stack_size?: number }} module_or_path - Passing `InitInput` directly is deprecated.
* @param {WebAssembly.Memory} memory - Deprecated.
*
* @returns {Promise<InitOutput>}
*/
declare function wasm_bindgen (module_or_path?: { module_or_path: InitInput | Promise<InitInput>, memory?: WebAssembly.Memory, thread_stack_size?: number } | InitInput | Promise<InitInput>, memory?: WebAssembly.Memory): Promise<InitOutput>;
