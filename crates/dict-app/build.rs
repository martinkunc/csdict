//! Embeds the application icon and version info into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/icon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icon/icon.ico");
        res.set("ProductName", "CSDict");
        res.set("FileDescription", "CSDict dictionary");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
