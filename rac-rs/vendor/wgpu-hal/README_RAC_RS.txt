wgpu-hal 30.0.1 as shipped on crates.io, with ONE change for rac-rs (MIT OR Apache-2.0, see LICENSE.MIT / LICENSE.APACHE).

What changed
  src/dx12/instance.rs, in `Instance::init`: `let factory_media = lib_dxgi.create_factory_media().ok();` became
  `let factory_media = None;` (plus comment lines), and `#[allow(dead_code)]` on `create_factory_media` in src/dx12/mod.rs
  (the function is no longer called). Nothing else differs from the published crate (compare with
  `diff -r` against ~/.cargo/registry/src/*/wgpu-hal-30.0.1; only the .cargo-ok marker file is missing here).

Why
  wgpu-hal's DirectX 12 start-up always calls CreateDXGIFactory1(IID_IDXGIFactoryMedia). That factory is used only for
  SurfaceTarget::SurfaceHandle (composition surface handles), never for window (HWND) surfaces, which is all rac-rs uses.
  ReShade 6.8.0 hooks CreateDXGIFactory1 in its dxgi.dll proxy and its hook crashes (access violation) on that interface id,
  so the game could not start on DirectX 12 with ReShade installed. A SurfaceHandle surface now fails with wgpu's own error
  "IDXGIFactoryMedia not found"; rac-rs does not use one. wgpu has no option that avoids the call (checked in 30.0.1).

How it is used
  rac-rs/Cargo.toml: [patch.crates-io] wgpu-hal = { path = "vendor/wgpu-hal" }. In a release tree whose cargo workspace root
  is the tree root (relayout.py), the same [patch.crates-io] line with path = "rac-rs/vendor/wgpu-hal" must be in the ROOT
  Cargo.toml: cargo ignores [patch] in a member manifest.

How to re-apply on a wgpu upgrade
  1. Copy the new wgpu-hal-X.Y.Z from the cargo registry over this folder (leave out .cargo-ok).
  2. In src/dx12/instance.rs find `create_factory_media()` in `Instance::init` and replace the call with `None`.
  3. If a wgpu release stops creating the factory unconditionally (or ReShade's hook survives it), delete this folder and the
     [patch] lines instead.
