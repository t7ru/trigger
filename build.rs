fn main() {
    println!("cargo:rerun-if-changed=assets/trigger.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/trigger.ico");
        res.compile().expect("failed to compile Windows resources");
    }
}
