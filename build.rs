// Embeds the app icon into FireCraft.exe (Explorer, taskbar pin, Alt+Tab).
fn main() {
    println!("cargo:rerun-if-changed=assets/logo.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/logo.ico");
        res.set("FileDescription", "FireCraft");
        res.set("ProductName", "FireCraft");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the app icon: {e}");
        }
    }
}
