fn main() {
    // Explorer, the taskbar and the installer's shortcuts all take the icon from the executable.
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../legacy/kopuz/assets/app.ico");
        res.compile().expect("failed to compile Windows resources");
    }
}
