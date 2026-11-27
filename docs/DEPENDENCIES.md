# Dependencies

Every third-party dependency, with license and reason. Allowed licenses are listed in deny.toml.

| Crate | Version | License | Used by | Reason |
| --- | --- | --- | --- | --- |
| proptest | 1.12.0 (default-features off, `std`) | MIT OR Apache-2.0 | archviz-core (dev) | Property tests for invariants (CLAUDE.md). Test-only; transitive: bitflags, cfg-if, chacha20, core_detect, cpufeatures, getrandom, num-traits, rand, rand_core, rand_xorshift, regex-syntax, unarray, all MIT OR Apache-2.0. |
| uuid | 1.29.3 (default-features off, `v5`) | Apache-2.0 OR MIT | archviz-core | UUID type and UUIDv5 derivation (spec D-12). No `v4`, so no RNG. |
| sha1_smol | 1.0.1 | BSD-3-Clause | archviz-core (via uuid) | SHA-1 for UUIDv5; pulled in by the uuid `v5` feature. |
| serde (+ serde_derive) | 1.0.229 (`std`, `derive`) | MIT OR Apache-2.0 | archviz-core | Serialization of the document model (T-104). |
| serde_json | 1.0.154 (`std`) | MIT OR Apache-2.0 | archviz-core | JSON values for the canonical writer and parser; the writer sorts keys itself and rejects non-integers. |
| schemars (+ schemars_derive) | 1.2.3 (`std`, `derive`, `uuid1`) | MIT | archviz-core | Generates schemas/document.schema.json from the model types. |
| insta | 1.49.0 | Apache-2.0 | archviz-core (dev) | Golden-file snapshots (CLAUDE.md). Test-only. |
| jsonschema | 0.63.2 (default-features off) | MIT | archviz-core (dev) | Validates the golden document against the generated schema. Test-only; its transitive crates are MIT, Apache-2.0, BSD, Zlib or Unlicense-or-MIT, checked by cargo deny. |
| uuid (`v4`) | 1.29.3 | Apache-2.0 OR MIT | archviz-cli | Random ids for `new` when the request gives none (Q-49); pulls getrandom 0.4.3 (MIT OR Apache-2.0). Only the CLI enables v4; core never generates random ids. |
| serde, serde_json, schemars | as above | as above | archviz-cli | Request and response types and their schemas in schemas/cli/. |
| insta | 1.49.0 | Apache-2.0 | archviz-cli (dev) | Golden CLI transcripts. |
| archviz-core | (workspace) | GPL-3.0-or-later | archviz-export | The document model and geometry the DXF writer draws. |
| dxf | 0.6.2 | MIT | archviz-export (dev) | Independent DXF reader that checks our writer output in tests (Q-51). Test-only; its 33 transitive crates pass cargo deny. |
| insta, proptest | as above | as above | archviz-export (dev) | DXF golden file and number-format property test. |
| serde_json | 1.0.154 (`std`) | MIT OR Apache-2.0 | archviz-export | Builds the GLB JSON chunk (T-119). |
| gltf | 1.4.1 (default-features off, `names`, `utils`) | MIT OR Apache-2.0 | archviz-export (dev) | Independent glTF reader that checks the GLB writer in tests (Q-56). Test-only; pulls gltf-json and gltf-derive (MIT OR Apache-2.0) and their proc-macro crates, checked by cargo deny. |
| sha2 | 0.10.9 (default-features off) | MIT OR Apache-2.0 | archviz-export | SHA-256 for the Scene Package manifest.sha256 (T-120, Q-58). Pulls digest, block-buffer, crypto-common, generic-array, typenum and cpufeatures (MIT or MIT OR Apache-2.0), checked by cargo deny. |
| serde, schemars | as above | as above | archviz-export | scene.json types and schemas/scene.schema.json (T-120, Q-59). |
| jsonschema | 0.63.2 (default-features off) | MIT | archviz-export (dev) | Validates scene.json against the generated schema. Test-only. |
| archviz-export | (workspace) | GPL-3.0-or-later | archviz-xtask | `cargo xtask golden` writes schemas/scene.schema.json. |
| serde_json | 1.0.154 (`std`) | MIT OR Apache-2.0 | archviz-xtask (dev) | Reads CLI responses in the `cargo xtask slice` tests (T-122). Test-only; already in the lock file. |
| interprocess | 2.4.10 (default-features off) | 0BSD OR Apache-2.0 | archviz-cli | The local named pipe the host serves and the CLI attaches through (spec 3.1, T-201). Pulls recvmsg and doctest-file (0BSD, allowed by DEC-12) and windows-sys and widestring (MIT OR Apache-2.0 or similar), checked by cargo deny. |
| getrandom | 0.4.3 (default-features off) | MIT OR Apache-2.0 | archviz-cli | The random pipe name and 256-bit session token in host.json (DEC-13). Already in the lock file through uuid v4. |
| eframe (+ egui, epaint, egui-winit, egui-wgpu, winit, wgpu) | 0.37.0 (default-features off, `wgpu`, `clipboard`), wgpu 30.0.2 | MIT OR Apache-2.0 | archviz-app | The egui app (spec D-14, T-201); wgpu is the renderer D-14 names for the 3D viewer. No `default_fonts`, so epaint_default_fonts (OFL-1.1, Ubuntu-font-1.0) is not built; the app loads a Windows font at run time (DEC-12). The clipboard pulls arboard, clipboard-win and error-code (BSL-1.0, allowed by DEC-12). About 290 transitive crates (winit, wgpu, naga, windows bindings and others), all checked by cargo deny. |
| proptest | as above | as above | archviz-app (dev) | Property tests that every snap and every drag command value lies on the 1/16 in grid (T-202). |
| archviz-cli, archviz-core | (workspace) | GPL-3.0-or-later | archviz-app | The app is a client of the command API (spec C5): it hosts `store::Project` and `host::serve`, and adds no business logic. |
| serde_json, uuid (`v4`) | as above | as above | archviz-app | Collection counts in the project view; random ids for commands the app hosts (Q-49). |
