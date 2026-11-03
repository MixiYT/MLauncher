fn main() {
    println!("cargo:rerun-if-env-changed=MLAUNCHER_OVERLAY_SHA256");
    if let Ok(hash) = std::env::var("MLAUNCHER_OVERLAY_SHA256") {
        println!("cargo:rustc-env=MLAUNCHER_OVERLAY_SHA256={hash}");
    }
    println!("cargo:rerun-if-changed=native/bin/mlauncher-overlay.dll");
    let overlay = std::path::Path::new("native/bin/mlauncher-overlay.dll");
    if std::env::var_os("CARGO_FEATURE_LITE").is_none() && overlay.is_file() {
        use sha2::Digest;
        let bytes = std::fs::read(overlay).expect("Read overlay DLL");
        let actual = format!("{:x}", sha2::Sha256::digest(&bytes));
        if let Ok(expected) = std::env::var("MLAUNCHER_OVERLAY_SHA256") {
            assert!(
                actual.eq_ignore_ascii_case(&expected),
                "Overlay DLL hash differs from build setting"
            );
        }
        println!("cargo:rustc-env=MLAUNCHER_OVERLAY_SHA256={actual}");
        // OUT_DIR is target/<profile>/build/<package>/out (also for custom targets).
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let profile = out.ancestors().nth(3).expect("Cargo profile directory");
        std::fs::copy(overlay, profile.join("mlauncher-overlay.dll"))
            .expect("Copy adjacent overlay DLL");
    }
    #[cfg(windows)]
    {
        let mut resources = winres::WindowsResource::new();
        resources.set_icon("assets/icon.ico");
        let lite = std::env::var_os("CARGO_FEATURE_LITE").is_some();
        resources.set(
            "ProductName",
            if lite { "MLauncher Lite" } else { "MLauncher" },
        );
        resources.set(
            "FileDescription",
            if lite {
                "MLauncher Lite - Minecraft launcher"
            } else {
                "MLauncher - Minecraft launcher"
            },
        );
        let version = format!("{}.0", env!("CARGO_PKG_VERSION"));
        resources.set("ProductVersion", &version);
        resources.set("FileVersion", &version);
        resources.set("LegalCopyright", "MIT License");
        if let Err(error) = resources.compile() {
            println!("cargo:warning=Could not embed Windows icon/version: {error}");
        }
        // winres складывает ресурсы в static-библиотеку; некоторые линковщики
        // GNU выбрасывают неиспользуемую секцию, поэтому передаём объект напрямую.
        if let Ok(out_dir) = std::env::var("OUT_DIR") {
            let object = std::path::Path::new(&out_dir).join("resource.o");
            if object.is_file() {
                println!("cargo:rustc-link-arg={}", object.display());
            }
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
