# example-tree: test plugin for ofd

A signed-plugin guest without any import: a fixed tree of virtual files under
`/@example/<instance>/` (a directory, an empty file, a small file, a 300 KiB
file and a name with a non-UTF-8 byte). An online restore checksums every file
in memory; `CloseFile` answers `bytes=<n> fnv=<hex>`.

It is `no_std`: Rust's std pulls WASI imports into a `wasm32-wasip2`
component, and the host refuses every import. The guest therefore brings its
own allocator, `cabi_realloc`, `memcmp` and panic handler.

```sh
rustup target add wasm32-wasip2
CARGO_TARGET_DIR=../../target/plugins cargo build --release --target wasm32-wasip2
cd ../.. && cargo test -p ofd -- --ignored example_tree
```

The test signs the artifact with a fresh test CA. For a real bundle use
`crates/onesimus-plugins/examples/sign-component.rs` in the build system.
