fn main() {
    println!("cargo:rerun-if-changed=assets/zcord.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    winresource::WindowsResource::new()
        .set_icon("assets/zcord.ico")
        .compile()
        .expect("failed to embed the Windows icon");
}
