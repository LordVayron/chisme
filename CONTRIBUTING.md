# Contributing to Chisme

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Reporting a bug

[Open an issue](https://github.com/LordVayron/chisme/issues/new/choose). Include:
- Your Windows version.
- Your GPU.
- Chisme's version.
- What you did, what you expected, and what happened.

For capture problems, the output of `chisme devices` helps.

## Making a change

1. Set up the tools in [Building from source](README.md#building-from-source).
2. Make your change in a branch.
3. Run `cargo test` and `cargo clippy`, and try the app.
4. Open a pull request that explains what changed and why.

**Style:**
- Format with `rustfmt --config max_width=200,use_small_heuristics=Max`.
- Write comments that say *why*, not what.
- Every string shown to people needs both languages: `l("English", "Español")`, or
  `t!("{} files", "{} archivos", n)` with arguments.
- Keep video on the GPU, and keep audio and video on the same clock (`win::now`). For anything
  that touches timing, check sync with `scripts/synccheck.py`.

## Code of conduct

Be kind and assume good intent. We follow the
[Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).
