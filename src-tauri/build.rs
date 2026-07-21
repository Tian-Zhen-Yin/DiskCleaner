fn main() {
    let mut attrs = tauri_build::Attributes::new();
    #[cfg(windows)]
    {
        attrs = attrs.windows_attributes(
            tauri_build::WindowsAttributes::new().app_manifest(
                std::fs::read_to_string("app.manifest")
                    .expect("failed to read app.manifest"),
            ),
        );
    }
    if let Err(e) = tauri_build::try_build(attrs) {
        panic!("failed to run tauri-build: {e}");
    }
}
