// Writes include/azulejo.h next to this crate so the macOS shell can import it.
fn main() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    let config = cbindgen::Config::from_file(format!("{crate_dir}/cbindgen.toml")).unwrap();
    cbindgen::generate_with_config(&crate_dir, config)
        .expect("cbindgen failed")
        .write_to_file(format!("{crate_dir}/include/azulejo.h"));
}
