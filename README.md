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
        "../wasmtime/target/release/wasmtime run --allow-precompiled peval.wevaled.cwasm"
Benchmark 1: ./peval.out
  Time (mean ± σ):     353.8 ms ±   7.1 ms    [User: 353.3 ms, System: 0.3 ms]
  Range (min … max):   342.4 ms … 367.4 ms    10 runs
 
Benchmark 2: ../wasmtime/target/release/wasmtime run --allow-precompiled peval.normal.cwasm
  Time (mean ± σ):     542.9 ms ±   3.1 ms    [User: 542.0 ms, System: 2.4 ms]
  Range (min … max):   536.7 ms … 547.1 ms    10 runs
 
Benchmark 3: ../wasmtime/target/release/wasmtime run --allow-precompiled peval.wevaled.cwasm
  Time (mean ± σ):      40.4 ms ±   2.7 ms    [User: 38.9 ms, System: 3.1 ms]
  Range (min … max):    37.8 ms …  55.8 ms    74 runs
 
  Warning: Statistical outliers were detected. Consider re-running this benchmark on a quiet system without any interferences from other programs. It might help to use the '--warmup' or '--prepare' options.
 
Summary
  '../wasmtime/target/release/wasmtime run --allow-precompiled peval.wevaled.cwasm' ran
    8.76 ± 0.61 times faster than './peval.out'
   13.44 ± 0.91 times faster than '../wasmtime/target/release/wasmtime run --allow-precompiled peval.normal.cwasm'
(took 12 sec.)                                                                                                                                                                           
hickory%
```
