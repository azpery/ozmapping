// Intègre l'icône de l'app dans l'exécutable Windows.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "OZ Mapping");
        res.set("FileDescription", "OZ Mapping");
        res.compile().expect("ressources Windows (icône)");
    }
}
