fn main() {
    let host = std::env::var("CARGO_FEATURE_HOST").is_ok();
    let target = std::env::var("TARGET").unwrap_or_default();
    if host && target.contains("apple-darwin") {
        cc::Build::new()
            .file("src/mac_app_protocol.m")
            .flag("-fobjc-arc")
            .compile("anycode_cef_mac_app");
        println!("cargo:rerun-if-changed=src/mac_app_protocol.m");
    }
}
