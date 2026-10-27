// [PC] The upstream Windows build embeds rac-rs.manifest in the game exe (the Windows segment heap for the process heap);
// this fork builds for Linux, which has no segment heap and no manifest. The file is archived with the rest of the
// upstream Windows build in windows/src/rac-rs/exe/rac-rs.manifest.

fn main() {
	println!("cargo:rerun-if-changed=build.rs");
}
