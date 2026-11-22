TARGET := wasm32-unknown-unknown
WASM := target/$(TARGET)/release/gri_wordle.wasm

.PHONY: all app check clean

all: app

app: app.wasm

src/words.bin: src/words.txt tools/make_words.py
	python3 tools/make_words.py

app.wasm: src/lib.rs src/words.rs src/words.bin Cargo.toml
	cargo build --release --target $(TARGET)
	cp $(WASM) app.wasm
	python3 tools/package.py

check:
	cargo build --release --target $(TARGET)

clean:
	cargo clean
	rm -f app.wasm
