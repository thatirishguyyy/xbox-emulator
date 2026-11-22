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
- **Credit your sources.** If you used open-source code, it must have a compatible licence. Record it in `THIRD_PARTY/CREDITS`.

## How pull requests are reviewed and merged

1. Open the pull request against `main`. Keep it to one change, based on the current `main`; rebase when it falls behind.
2. CI runs on every pull request. All of these checks must pass before review:
   - **PR gate**: no binaries (`.exe`, `.dll`, `.pdb`, `.lib`, `.bin`, `.dxil`, `.cso`, `.spv`, `.xvd`, `.pak`, archives, images, audio and similar), no game or console data (dump paths, package manifests, extracted assets), no file over 1 MiB, no personal data (absolute paths into a user profile or home directory, machine names, tokens, keys, webhook URLs) and **no comment lines** added to `.rs`, `.py` or `.ps1` files. The project keeps code free of comment lines: say what the code does through names and tests, and put the explanation in the pull request description or the `exports.txt` note.
   - **Rust**: `cargo check` and the unit and WARP tests for the crates that build without console files.
   - **actionlint** for workflow changes.
3. A maintainer reviews the code, checks it against the console behaviour and runs the tests that need real hardware or a dump locally.
4. Accepted pull requests are merged with a merge commit, never squashed or re-implemented, so **your commits keep your name and email** and you show up as a contributor. Use the name and email you want credited in your commits.
5. Changes to `.github/` (workflows and the gate script) are reviewed by hand; the gate always runs the script from `main`, not the copy in your pull request.

Pull requests that add binaries or game data, or that copy code under an incompatible licence, are closed without review.

## Credits for third-party code

When you use code or data tables from another project, add **one line per project** to `THIRD_PARTY/CREDITS` at the repository root. There are no per-crate credit files and no credits in code. When you copy code, add its upstream licence text beside that code (as `crates/compat/xcrdapi/src/bcpack` does) or under `THIRD_PARTY/licenses/`:

```
<project> (<url>, <licence>): <what was used>; <where>
```

GPL-3.0, LGPL, MIT, BSD, Apache-2.0 and zlib projects are fine. Keep their copyright notices. GPL-2.0-only code cannot be used.

## Building

Windows 10/11 x64 with a stable Rust toolchain (MSVC):

```
cargo build -p <crate>
cargo test -p <crate>
```

The GNU target (`x86_64-pc-windows-gnu`) is not supported: its software `fmaf` is not correctly rounded, so the FMA reference values in the tombox-xshader tests come out one ULP off (see #126).

Some crates also need third-party SDKs (the DirectX Agility SDK, DXC, libopus). Their `build.rs` says which.

## Licence

Contributions are accepted under the project licence: GPL-3.0-or-later. See [LICENSE](LICENSE). Code from GPL-3.0 projects may be used with its copyright notices kept and recorded in `THIRD_PARTY/CREDITS`; GPL-2.0-only code is not compatible and must not be copied.
