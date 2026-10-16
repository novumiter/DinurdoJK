fn main() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/blue-water-sea-and-sky/clouds");
    println!(
        "cargo:rustc-env=BLUE_WATER_SHADER_ROOT={}",
        source.display()
    );
    println!("cargo:rustc-env=BLUE_WATER_SHADER_PACKAGE=naval-clouds");
    println!("cargo:rerun-if-changed=build.rs");
}
