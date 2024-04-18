<div align="center">
  <h1>weval</h1>

  <p>
    <strong>weval Wasm partial evaluator</strong>
  </p>

  <strong>A <a href="https://bytecodealliance.org/">Bytecode Alliance</a> project</strong>

  <p>
    <a href="https://github.com/bytecodealliance/weval/actions?query=workflow%3ACI"><img src="https://github.com/bytecodealliance/weval/workflows/CI/badge.svg" alt="build status" /></a>
    <a href="https://bytecodealliance.zulipchat.com/#narrow/stream/223391-wasm"><img src="https://img.shields.io/badge/zulip-join_chat-brightgreen.svg" alt="zulip chat" /></a>
    <a href="https://docs.rs/weval"><img src="https://docs.rs/weval/badge.svg" alt="Documentation Status" /></a>
  </p>

  <h3>
    <a href="https://github.com/bytecodealliance/weval/blob/main/CONTRIBUTING.md">Contributing</a>
    <span> | </span>
    <a href="https://bytecodealliance.zulipchat.com/#narrow/stream/223391-wasm">Chat</a>
  </h3>
</div>

`weval` partially evaluates WebAssembly snapshots to turn interpreters into
compilers (see [Futamura
projection](https://en.wikipedia.org/wiki/Partial_evaluation#Futamura_projections)
for more).

`weval` binaries are available via releases on this repo or via an [npm
package](https://www.npmjs.com/package/@cfallin/weval).

Usage of weval is like:

```
$ weval weval -w -i program.wasm -o wevaled.wasm
```

which runs Wizer on `program.wasm` to obtain a snapshot, then processes any
weval requests (function specialization requests) in the resulting heap image,
appending the specialized functions and filling in function pointers in
`wevaled.wasm`.

See the API in `include/weval.h` for more.

### Releasing Checklist

- Bump the version in `Cargo.toml` and `cargo check` to ensure `Cargo.lock` is
  updated as well.
- Bump the tag version (`TAG` constant) in `npm/weval/index.js`.
- Bump the npm package version in `npm/weval/package.json`.
- Run `npm i` in `npm/weval/` to ensure the `package-lock.json` file is
  updated.

- Commit all of this as a "version bump" PR.
- Push it to `main` and ensure CI completes successfully.
- Tag as `v0.x.y` and push that tag.
- `cargo publish` from the root.
- `npm publish` from `npm/weval/`.

### Further Details

The theory behind weval is described in the author's blog post
[here](https://cfallin.org/blog/2024/08/28/weval/), covering partial evaluation
and Futumura projections as well as how weval's main transform works.

### Uses

weval is in use to provide ahead-of-time compilation of JavaScript by wevaling
a build of the [SpiderMonkey](https://spidermonkey.dev) interpreter, providing
3-5x speedups over the generic interpreter. Please let us know if you use it
elsewhere!






`weval` is still beta-level and not yet ready for production use (though it's
getting close!).

```
hickory% make bench                
hyperfine \
        "./peval.out" \
        "../wasmtime/target/release/wasmtime run --allow-precompiled peval.normal.cwasm" \
        "../wasmtime/target/release/wasmtime run --allow-precompiled peval.wevaled.cwasm" \
        "node wrapper.mjs peval.normal.wasm" \
        "node wrapper.mjs peval.wevaled.wasm" \
        "~/.bun/bin/bun wrapper.mjs peval.normal.wasm" \
        "~/.bun/bin/bun wrapper.mjs peval.wevaled.wasm"
Benchmark 1: ./peval.out
  Time (mean ± σ):     341.0 ms ±   3.7 ms    [User: 340.0 ms, System: 0.9 ms]
  Range (min … max):   335.1 ms … 348.5 ms    10 runs
 
Benchmark 2: ../wasmtime/target/release/wasmtime run --allow-precompiled peval.normal.cwasm
  Time (mean ± σ):     532.1 ms ±   9.5 ms    [User: 529.0 ms, System: 4.6 ms]
  Range (min … max):   523.9 ms … 556.3 ms    10 runs
 
Benchmark 3: ../wasmtime/target/release/wasmtime run --allow-precompiled peval.wevaled.cwasm
  Time (mean ± σ):      40.1 ms ±   1.8 ms    [User: 38.7 ms, System: 3.1 ms]
  Range (min … max):    37.6 ms …  49.2 ms    68 runs
 
Benchmark 4: node wrapper.mjs peval.normal.wasm
  Time (mean ± σ):      1.442 s ±  0.021 s    [User: 1.372 s, System: 0.018 s]
  Range (min … max):    1.406 s …  1.482 s    10 runs
 
Benchmark 5: node wrapper.mjs peval.wevaled.wasm
  Time (mean ± σ):     309.0 ms ±  14.9 ms    [User: 229.4 ms, System: 18.2 ms]
  Range (min … max):   287.7 ms … 336.3 ms    10 runs
 
Benchmark 6: ~/.bun/bin/bun wrapper.mjs peval.normal.wasm
  Time (mean ± σ):     581.0 ms ±   8.4 ms    [User: 578.8 ms, System: 9.4 ms]
  Range (min … max):   572.3 ms … 600.7 ms    10 runs
 
Benchmark 7: ~/.bun/bin/bun wrapper.mjs peval.wevaled.wasm
  Time (mean ± σ):      47.1 ms ±   1.4 ms    [User: 38.2 ms, System: 15.2 ms]
  Range (min … max):    44.9 ms …  51.4 ms    62 runs
 
Summary
  '../wasmtime/target/release/wasmtime run --allow-precompiled peval.wevaled.cwasm' ran
    1.17 ± 0.06 times faster than '~/.bun/bin/bun wrapper.mjs peval.wevaled.wasm'
    7.70 ± 0.51 times faster than 'node wrapper.mjs peval.wevaled.wasm'
    8.50 ± 0.40 times faster than './peval.out'
   13.26 ± 0.64 times faster than '../wasmtime/target/release/wasmtime run --allow-precompiled peval.normal.cwasm'
   14.48 ± 0.69 times faster than '~/.bun/bin/bun wrapper.mjs peval.normal.wasm'
   35.94 ± 1.70 times faster than 'node wrapper.mjs peval.normal.wasm'
(took 38 sec.)                                                                                                                                                                           
hickory%
```
