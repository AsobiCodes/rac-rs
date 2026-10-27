// Windows: embeds the icon, the version info and the manifest (DPI aware, UTF-8, long paths) into the exe.
fn main() {
    println!("cargo:rerun-if-changed=assets/launcher.rc");
    println!("cargo:rerun-if-changed=assets/launcher.manifest");
    println!("cargo:rerun-if-changed=assets/rac-rs.ico");
    #[cfg(windows)]
    {
        let _ = embed_resource::compile("assets/launcher.rc", embed_resource::NONE);
    }
}
