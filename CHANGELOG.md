# v0.1.3

- Fix performance regression for sequential keys.
    - Ensure Node256 is page-aligned, otherwise we encounter kernel-level
      contention when threads page fault on different Node256s that share
      the same page.
- Replace `memcpy` call with inline assembly `mov` for unsized keys.
- Fix edge case when removing unsized, non-null keys.

# v0.1.2

- Fix inverted `opt-no-path` feature flag.

# v0.1.1

- Fix badge links in `README.md`.

# v0.1.0

Initial release.
