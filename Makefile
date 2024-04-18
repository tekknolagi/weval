WASI_CXX=/opt/wasi-sdk/bin/clang++
CXXFLAGS=-O2 -I include
peval.out: peval.cc include/wizer.h include/weval.h
	$(CXX) $(CXXFLAGS) $< -o $@

peval.wasm: peval.cc include/wizer.h include/weval.h
	$(WASI_CXX) $(CXXFLAGS) -DDO_WEVAL $< -o $@

peval.normal.wasm: peval.cc
	$(WASI_CXX) $(CXXFLAGS) $< -o $@

peval.cwasm: peval.wasm
	../wasmtime/target/release/wasmtime compile $< -o $@

peval.normal.cwasm: peval.normal.wasm
	../wasmtime/target/release/wasmtime compile $< -o $@

peval.wat: peval.wasm
	wasm2wat $< > $@

peval.wevaled.wasm: peval.wasm ./target/release/weval
	./target/release/weval weval -i $< -o $@ -w

peval.wevaled.cwasm: peval.wevaled.wasm
	../wasmtime/target/release/wasmtime compile $< -o $@

peval.wevaled.wat: peval.wevaled.wasm
	wasm2wat $< > $@

./target/release/weval:
	cargo build --release

bench: peval.out peval.normal.cwasm peval.wevaled.cwasm
	hyperfine \
		"./peval.out" \
		"../wasmtime/target/release/wasmtime run --allow-precompiled peval.normal.cwasm" \
		"../wasmtime/target/release/wasmtime run --allow-precompiled peval.wevaled.cwasm"

.PHONY: bench
