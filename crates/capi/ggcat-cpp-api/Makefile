
.PHONY: all ggcat-capi ggcat-source

all: lib/libggcat_api.a

clean:
	cargo clean
	rm -r build/ lib/

lib/libggcat_api.a: ./lib/libggcat_cpp_bindings.a
	mkdir -p build/
	g++ -std=c++11 -O3 -I./include -I./src -c ./src/ggcat.cc -lggcat_cpp_bindings -lggcat_cxx_interop -o build/ggcat.o -Wall -Wextra
	ar cr lib/libggcat_api.a build/ggcat.o

./lib/libggcat_cpp_bindings.a: ggcat-source
	cargo build --release --package ggcat-cpp-bindings
	cp ../../../target/release/libggcat_cpp_bindings.a ./lib/
