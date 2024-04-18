WASI_CXX=/opt/wasi-sdk/bin/clang++
CXXFLAGS=-O2 -I include
peval.out: peval.cc include/wizer.h include/weval.h
	$(CXX) $(CXXFLAGS) $< -o $@

peval.wasm: peval.cc include/wizer.h include/weval.h
	$(WASI_CXX) $(CXXFLAGS) $< -o $@

peval.cwasm: peval.wasm
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
