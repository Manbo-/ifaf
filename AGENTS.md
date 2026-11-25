# Project guidance

## Project scope

Build a native 64-bit Susie image plugin that extracts an embedded preview from Affinity `.af` files. Do not add support for `.afphoto`, `.afdesign`, or `.afpub` unless the project scope is explicitly changed. The plugin must not depend on Affinity or launch external processes. Treat input files as read-only.

## Preserve the supplied fixture

- `sample.af` is a large input fixture (about 36 MB). Never edit, replace, rename, or delete it as part of implementation or testing.
- Keep generated binaries, extracted previews, temporary files, and build output separate from source and fixtures; do not add generated artifacts to source control unless requested.
- The current directory may not be a Git repository. Check the actual repository/worktree state before using Git commands; do not initialize a repository without a request.

## Required research before implementation

Before designing a parser or choosing a preview, inspect and document evidence from these projects:

- Affinity Thumbnail Extractor
- Affinity File Preview and Image Extractor
- `nextcloud-preview-affinity`
- Asset-Hoard / `afphoto`

Record the sources and versions/commits inspected, `.af` container structure, PNG discovery and validation, preview selection behavior when multiple PNGs exist, and behavior when none exists. Check each project's license and reuse conditions. Do not infer an undocumented container layout or selection rule; mark unknowns as unknown and seek corroborating implementation or fixture evidence.

Confirm the Susie API definitions, ABI, calling convention, bitness, ownership/allocation rules, and error behavior from authoritative documentation or headers before implementing exports. Document any incompatibility between the desired 64-bit target and the API/host actually supported by the inspected Susie environment.

## Implementation constraints

- Use Rust for the x64 DLL and parser. The current development environment has a working `x86_64-pc-windows-msvc` Rust toolchain; Rust's bounded byte parsing is a good fit for untrusted binary input. Use a maintained PNG decoder crate and a small, explicit Windows/Susie FFI boundary. Revisit this choice only if a concrete Susie ABI or library constraint makes Rust impractical.
- Keep the first implementation focused on `.af` preview extraction and the required Susie exports: `GetPluginInfo`, `IsSupported`, `GetPictureInfo`, and `GetPicture`. Add optional APIs only when evidence shows they are needed.
- Use a maintained PNG decoding library with a clear license. Minimize dependencies and record their versions and licenses.
- Validate file identity and container bounds before parsing. Bound input size, image dimensions, decoded allocation, offsets, and lengths; use overflow-safe arithmetic and reject truncated or malformed data without crashing.
- Validate PNG structure and decoded data before returning an image. Return a normal unsupported/error result when there is no usable preview.
- Select the first valid embedded PNG in file order. This is the agreed product behavior; do not reorder candidates by dimensions or file size.
- Keep parsing independent from UI/API glue so the extractor can be reviewed and exercised separately.

## Development sequence

1. Inventory the workspace and preserve existing files and user changes.
2. Complete the repository, format, license, and Susie API research; write down findings and unresolved questions before committing to a parser design.
3. Inspect `sample.af` read-only and determine whether it is suitable for validating the researched format behavior. Do not treat one fixture as proof of general format semantics.
4. Build a minimal extraction prototype and validate its output before integrating the Susie entry points.
5. Integrate the plugin, document build steps and limitations, then run the checks appropriate to the changes and report exactly what was and was not verified.

## Validation and documentation

Where suitable fixtures and tooling are available, cover valid `.af` files, multiple embedded PNGs, no preview, truncation/corruption, renamed non-Affinity files, size/resource limits, Unicode paths, and long paths. Verify display in an actual compatible Susie host when available; do not describe static checks or a parser-only test as host validation. Clearly label unavailable fixtures, hosts, or toolchains as unverified.

Keep a README with supported extension/version scope, researched format behavior and sources, preview selection rationale, build prerequisites/steps, dependency and license notes, safety limits, known limitations, and a concise test/host-validation record. Do not claim support for behavior that has not been evidenced.
