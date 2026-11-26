# Dependencies

Every third-party dependency, with license and reason. Allowed licenses are listed in deny.toml.

| Crate | Version | License | Used by | Reason |
| --- | --- | --- | --- | --- |
| proptest | 1.12.0 (default-features off, `std`) | MIT OR Apache-2.0 | archviz-core (dev) | Property tests for invariants (CLAUDE.md). Test-only; transitive: bitflags, cfg-if, chacha20, core_detect, cpufeatures, getrandom, num-traits, rand, rand_core, rand_xorshift, regex-syntax, unarray, all MIT OR Apache-2.0. |
| uuid | 1.29.3 (default-features off, `v5`) | Apache-2.0 OR MIT | archviz-core | UUID type and UUIDv5 derivation (spec D-12). No `v4`, so no RNG. |
| sha1_smol | 1.0.1 | BSD-3-Clause | archviz-core (via uuid) | SHA-1 for UUIDv5; pulled in by the uuid `v5` feature. |
