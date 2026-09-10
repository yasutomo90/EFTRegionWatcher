fn main() {
    println!("cargo:rerun-if-env-changed=EFT_RELEASE_REPOSITORY");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.rc");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/icon.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("assets/app.manifest");
        for bin in ["EFTRegionWatcher", "EFTRegionWatcherUpdater"] {
            println!("cargo:rustc-link-arg-bin={bin}=/MANIFEST:EMBED");
            println!(
                "cargo:rustc-link-arg-bin={bin}=/MANIFESTINPUT:{}",
                manifest.display()
            );
        }
    }
}
