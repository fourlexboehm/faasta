use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    let mut build = cc::Build::new();

    if target_arch == "x86_64" {
        let source = Path::new("../kvmserver/src/api/libkvmserverguest.c");
        if !source.exists() {
            panic!("kvmserver guest source not found at {}", source.display());
        }
        build.file(source);
    } else {
        let stub_path = PathBuf::from(&out_dir).join("libkvmserverguest_stub.c");
        fs::write(
            &stub_path,
            "#include <sys/types.h>\n\
             #include <stddef.h>\n\
             size_t kvmserverguest_remote_resume(void *buffer, ssize_t len) {\n\
                 (void)buffer;\n\
                 (void)len;\n\
                 return (size_t)-1;\n\
             }\n\
             size_t kvmserverguest_storage_wait_paused(void **req, ssize_t len) {\n\
                 (void)req;\n\
                 (void)len;\n\
                 return (size_t)-1;\n\
             }\n",
        )
        .expect("failed to write stub for libkvmserverguest");
        build.file(&stub_path);
    }

    build
        .include("../kvmserver/src/api")
        .flag_if_supported("-fPIC")
        .compile("kvmserverguest");

    println!("cargo:rustc-link-search=native={out_dir}");
}
