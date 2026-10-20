// Copyright (C) 2026 Toit contributors.
fn main() {
    let mut build = cc::Build::new();
    build.file("native/bridge.c").warnings(true);
    for lib in ["harfbuzz", "freetype2", "cairo", "poppler-glib"] {
        let info = pkg_config::Config::new()
            .probe(lib)
            .unwrap_or_else(|_| panic!("Missing native development library {lib}; see README.md"));
        for path in info.include_paths {
            build.include(path);
        }
    }
    build.compile("unredact_native");
    println!("cargo:rerun-if-changed=native/bridge.c");
}
