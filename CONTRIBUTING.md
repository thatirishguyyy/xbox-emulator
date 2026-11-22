# Contributing

Contributions are welcome. The most useful thing you can add right now is coverage of **console libraries**: the Xbox system DLLs that titles load. The [library map](docs/LIBRARY_MAP.md) shows which functions are still missing.

## What lives where

| Path | What it is |
|---|---|
| `crates/compat/<dll>/` | One crate per console DLL. It builds a drop-in DLL with the console's exports. |
| `crates/compat/<dll>/exports.txt` | One line per export: `ordinal Name status \| notes`. Status is `implemented`, `forwarded`, `pc`, `title` or `missing`. |
| `crates/compat/compat-common/` | Shared runtime: OS hooks, GDK file system and import routing. |
| `crates/compat/d3d12_xs/`, `d3d12_x/` | Xbox Direct3D 12.x (Series X\|S and One). |
| `crates/tombox-xshader/` | RDNA2 shader translator. |
| `tools/progress/` | Generates the progress map and `docs/` from the crates' `exports.txt` files. |

## Adding or finishing a library

1. Pick a `missing` row in the crate's `exports.txt`, or create a crate for a DLL that has none. Copy the layout of a small crate such as `crates/compat/xfp_gc`.
2. Work out what the console function does. Use your own console DLL (from your own legally obtained dump), public GDK documentation, and open-source references with compatible licences.
3. Implement it with the console's behaviour: same return codes, same edge cases, same output layout.
4. Add a unit test that calls the function and checks the console behaviour.
5. Update the row in `exports.txt` to `implemented` (or `forwarded` when it maps 1:1 to a PC function), with a short note citing where the behaviour came from.
6. Run `cargo test -p <crate>`, then open a pull request.

## Rules

- **Never commit game or console files.** No dumps, no console DLLs, no extracted assets, shaders or data, and no binaries of any kind (`.exe`, `.dll`, `.pdb`, ...). Pull requests containing them will be closed.
- **No fake success.** Don't return S_OK, or accept and ignore a call, when the console would do something. If something genuinely can't be done on PC, leave the row `missing` and explain why in its note.
- **No network access at runtime.** The emulator is offline. Online features are emulated in-process as private sessions only.
- **No decryption, key extraction or DRM circumvention.** The project only runs already-decrypted dumps.
- **Credit your sources.** If you used open-source code, it must have a compatible licence. Record it in the crate's `THIRD_PARTY` file.

## Building

Windows 10/11 x64 with a stable Rust toolchain (MSVC):

```
cargo build -p <crate>
cargo test -p <crate>
```

Some crates also need third-party SDKs (the DirectX Agility SDK, DXC, libopus). Their `build.rs` says which.

## Licence

Contributions are accepted under the project licence: GPL-3.0-or-later. See [LICENSE](LICENSE). Code from GPL-3.0 projects may be used with its copyright notices kept and recorded in the crate's `THIRD_PARTY` file; GPL-2.0-only code is not compatible and must not be copied.
