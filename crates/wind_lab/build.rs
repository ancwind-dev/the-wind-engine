//! Значок движка в самом файле `.exe` (виден в проводнике и на ярлыках).
//! Значок окна и панели задач ставится отдельно — в `main.rs` через `with_icon`.

fn main() {
    let ico = concat!(env!("CARGO_MANIFEST_DIR"), "/../../icon/wind.ico");
    println!("cargo:rerun-if-changed={ico}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico);
        res.compile().expect("значок exe: нужен rc.exe из Windows SDK");
    }
}
