fn main() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/blue-water-sea-and-sky/sky");
    println!(
        "cargo:rustc-env=BLUE_WATER_SHADER_ROOT={}",
        source.display()
    );
    println!("cargo:rustc-env=BLUE_WATER_SHADER_PACKAGE=naval-sky");
    println!("cargo:rerun-if-changed=build.rs");
}
