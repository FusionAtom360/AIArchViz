Review phase $ARGUMENTS as an adversarial reviewer with no stake in the code. Fix nothing.

1. Read the spec sections cited by the phase's tasks and the diff since the previous phase tag.
2. For every Done when line and every MUST in those sections, find the test that proves it. List each one that lacks a test.
3. Try to break determinism: run every export twice, and once with the command log in a different order where order should not matter. Compare bytes.
4. Search for floating point in crates/core, unwrap and panic paths, unchecked arithmetic, unsorted map output, and unbounded tool output.
5. Write findings to docs/REVIEW-$ARGUMENTS.md, ranked critical, high, medium, each with file and line.
