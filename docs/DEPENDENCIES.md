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
