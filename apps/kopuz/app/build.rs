fn main() {
    println!("cargo:rerun-if-env-changed=KOPUZ_FLAVOR");
    // Explorer, the taskbar and the installer's shortcuts all take the icon from the executable.
    #[cfg(target_os = "windows")]
    {
        let icon = match std::env::var("KOPUZ_FLAVOR").as_deref() {
            Ok("nightly") => "../../../packaging/kopuz-nightly/app.ico",
            _ => "../../legacy/kopuz/assets/app.ico",
        };
        println!("cargo:rerun-if-changed={icon}");
        let mut res = winres::WindowsResource::new();
        res.set_icon(icon);
        res.compile().expect("failed to compile Windows resources");
    }
}
