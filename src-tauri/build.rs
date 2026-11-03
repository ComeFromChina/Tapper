fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    println!("cargo:rerun-if-changed={}/../dist", manifest_dir);
    println!("cargo:rerun-if-changed={}/../dist/assets", manifest_dir);

    tauri_build::build()
}
