fn main() {
    println!("cargo:rerun-if-changed=icons/icon.ico");

    // Embed the app icon and version info into the Windows executable.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("icons/icon.ico")
            .set("ProductName", "CantiniAutoGPS")
            .set("FileDescription", "Geotag photos from a GPX track")
            .compile()
            .expect("failed to embed Windows resources");
    }
}
