// [PC] Embeds rac-rs.manifest in the game exe (Windows only): the Windows segment heap for the process heap (Windows 10 2004+; older
// Windows and Wine ignore it). With the default NT heap every allocation got slower after each level switch: after 10 Rilgar <->
// Blarg trips the main thread's frame took 2.5x as long (hero, props, upload alike: 2.7 -> 7.7 ms), and a live game after ~15 trips
// spent ~65% of its main and gs time in RtlAllocateHeap / RtlFreeHeap. With the segment heap the frame stays at 2.6-2.7 ms.
// embed-manifest needs no external tools (rc.exe / windres): MSVC link options or a COFF resource object for the GNU toolchain.
fn main() {
	println!("cargo:rerun-if-changed=rac-rs.manifest");
	println!("cargo:rerun-if-changed=build.rs");
	if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
		let m = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("rac-rs.manifest");
		embed_manifest::embed_manifest_file(&m).expect("embedding rac-rs.manifest");
	}
}
