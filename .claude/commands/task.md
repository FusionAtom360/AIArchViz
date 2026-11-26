Implement task $ARGUMENTS from docs/PLAN.md.

1. Read CLAUDE.md, the task entry, and each spec section it cites in docs/SPEC.md. Confirm every dependency task appears in docs/PROGRESS.md.
2. State the files you will create or change and the tests you will write.
3. Write failing tests, implement, and run `cargo xtask check` until it is green.
4. For each line of the task's Done when, quote the command output that proves it.
5. Commit, append to docs/PROGRESS.md, push to origin, and stop. Report what changed, the evidence, and any open questions.

Stop and ask if: the spec is ambiguous, a dependency falls outside the license list, a test fails and you cannot explain why, or the task reaches a human gate.
