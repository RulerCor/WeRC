fn main() {
    for variable in ["WERC_UPDATE_ENDPOINT", "WERC_UPDATE_PUBLIC_KEY", "WERC_RELEASE_PAGE"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    tauri_build::build()
}
