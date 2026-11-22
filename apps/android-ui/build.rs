fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        slint_build::compile_with_config(
            "ui/app.slint",
            slint_build::CompilerConfiguration::new().with_style("material-dark".into()),
        )
        .expect("Slint UI");
        // Slint's Android backend renders with Skia (C++). Link the NDK's
        // libc++ statically so no extra libc++_shared.so has to be shipped.
        println!("cargo:rustc-link-arg=-lc++_static");
        println!("cargo:rustc-link-arg=-lc++abi");
    }
}
