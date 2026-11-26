# Dependencies

Every third-party dependency, with license and reason. Allowed licenses are listed in deny.toml.

| Crate | Version | License | Used by | Reason |
| --- | --- | --- | --- | --- |
| proptest | 1.12.0 (default-features off, `std`) | MIT OR Apache-2.0 | archviz-core (dev) | Property tests for invariants (CLAUDE.md). Test-only; transitive: bitflags, cfg-if, chacha20, core_detect, cpufeatures, getrandom, num-traits, rand, rand_core, rand_xorshift, regex-syntax, unarray, all MIT OR Apache-2.0. |
