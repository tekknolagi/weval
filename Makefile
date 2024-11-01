WASI_CXX=/opt/wasi-sdk/bin/clang++
CXXFLAGS=-O2 -flto
peval.out: peval.cc
	$(CXX) $(CXXFLAGS) $< -o $@

peval.wasm: peval.cc include/wizer.h include/weval.h
	$(WASI_CXX) $(CXXFLAGS) -DDO_WEVAL -I include $< -o $@

peval.normal.wasm: peval.cc
	$(WASI_CXX) $(CXXFLAGS) $< -o $@

peval.opt_locals.wasm: peval.cc include/wizer.h include/weval.h
	$(WASI_CXX) $(CXXFLAGS) -DDO_WEVAL -DSPECIALIZE_LOCALS -I include $< -o $@

%.cwasm: %.wasm
	../wasmtime/target/release/wasmtime compile $< -o $@

%.wevaled.wasm: %.wasm ./target/release/weval
	./target/release/weval weval -i $< -o $@ -w

%.wat: %.wasm
	wasm2wat $< > $@

./target/release/weval:
	cargo build --release

bench: peval.out peval.normal.cwasm peval.wevaled.cwasm peval.opt_locals.wevaled.cwasm
	hyperfine \
		"./peval.out" \
		"../wasmtime/target/release/wasmtime run --allow-precompiled peval.normal.cwasm" \
		"../wasmtime/target/release/wasmtime run --allow-precompiled peval.wevaled.cwasm" \
		"../wasmtime/target/release/wasmtime run --allow-precompiled peval.opt_locals.wevaled.cwasm"
		# "node wrapper.mjs peval.normal.wasm" \
		# "node wrapper.mjs peval.wevaled.wasm" \
		# "~/.bun/bin/bun wrapper.mjs peval.normal.wasm" \
		# "~/.bun/bin/bun wrapper.mjs peval.wevaled.wasm"

clean:
	rm -f *.out *.wasm *.cwasm *.wat

.PHONY: bench clean
