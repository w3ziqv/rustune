fn main() {
    cc::Build::new()
        .cpp(true)
        .file("shim.cc")
        .include("vendor")
        .flag_if_supported("-std=c++14")
        .flag_if_supported("-Wno-unused-parameter")
        .compile("cdmshim");
    println!("cargo:rustc-link-lib=dylib=dl");
    println!("cargo:rerun-if-changed=shim.cc");
    println!("cargo:rerun-if-changed=timers.h");
    println!("cargo:rerun-if-changed=vendor");
}
