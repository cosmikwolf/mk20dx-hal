use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let memory_x = if cfg!(feature = "mk20d7") {
        include_bytes!("memory/memory_mk20d7.x").as_slice()
    } else if cfg!(feature = "mk20d5") {
        include_bytes!("memory/memory_mk20d5.x").as_slice()
    } else {
        panic!("Either mk20d5 or mk20d7 feature must be enabled");
    };

    fs::write(out_dir.join("memory.x"), memory_x).unwrap();

    println!("cargo:rustc-link-search={}", out_dir.display());
    println!("cargo:rerun-if-changed=memory/memory_mk20d5.x");
    println!("cargo:rerun-if-changed=memory/memory_mk20d7.x");
    println!("cargo:rerun-if-changed=build.rs");
}
