## weval: the WebAssembly partial evaluator

`weval` partially evaluates WebAssembly snapshots to turn interpreters into
compilers (see [Futamura
projection](https://en.wikipedia.org/wiki/Partial_evaluation#Futamura_projections)
for more).

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
