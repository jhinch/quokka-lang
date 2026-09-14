use std::path::{Path, PathBuf};

fn main() {
    compile_sameboy();
    bindgen_sameboy();
}

fn compile_sameboy() {
    let mut build = cc::Build::new();
    build.define("GB_VERSION", read_sameboy_version().as_str());
    build.define("GB_INTERNAL", None);
    // we don't control the source code, so we are not responsible for the warnings
    build.warnings(false);

    println!("cargo:rerun-if-changed=SameBoy/Core");
    for file in Path::new("SameBoy/Core")
        .read_dir()
        .unwrap_or_else(|e| panic!("Failed to read directory SameBoy/Core. Cause: {e}"))
    {
        let file = file.unwrap().path();
        if !file.extension().is_some_and(|e| e == "c") {
            continue;
        }
        build.file(file);
    }
    build.compile("sameboy")
}

fn read_sameboy_version() -> String {
    let version_file_contents = std::fs::read_to_string("SameBoy/version.mk")
        .unwrap_or_else(|e| panic!("Failed to read SameBoy/version.mk. Cause: {e}"));
    let version = version_file_contents
        .trim()
        .strip_prefix("VERSION := ")
        .expect("VERSION not found in SameBoy/version.mk");
    format!("\"{version}\"")
}

fn bindgen_sameboy() {
    let bindings = bindgen::builder()
        .header("SameBoy/Core/gb.h")
        .allowlist_type("GB_.*")
        .allowlist_function("GB_.*")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .unwrap_or_else(|e| panic!("Failed to generate bindings. Cause: {e}"));

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR env is not set"));
    let bindings_path = out.join("bindings.rs");
    bindings.write_to_file(&bindings_path).unwrap_or_else(|e| {
        panic!(
            "Failed to write binding to {}. Cause: {e}",
            bindings_path.display()
        )
    });
}
