fn main() {
    println!("cargo:rerun-if-env-changed=MLAUNCHER_OVERLAY_SHA256");
    if let Ok(hash) = std::env::var("MLAUNCHER_OVERLAY_SHA256") {
        println!("cargo:rustc-env=MLAUNCHER_OVERLAY_SHA256={hash}");
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
