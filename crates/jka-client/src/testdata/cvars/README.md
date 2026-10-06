# Cvar regression fixtures

These expectations were captured from the implementation before the full registry conversion. Tests never update them automatically.

Each matrix row starts with the cvar name, followed by one FNV-1a digest for each input in `cvar_compatibility::INPUTS`. Digests cover complete typed settings and parser results rather than just the one field being set. The runtime matrix covers every exposed getter after each setter, plus config-dirty, JAPRO debounce and command-pacing flags. It is captured with a windowless App and replayed on Windows.

`default.cfg` and `changed.cfg` also check exact archive order, quoting, spelling and number formatting. Intentional removal of obsolete config keys is asserted separately; the old changed fixture's fog override is explicitly adjusted in the assertion, not silently regenerated. The config matrix excludes obsolete keys and tests HDR/tonemap independence separately.

The fixture names include current console cvars and the former config-only names, so unknown-name handling is also exercised. The original implementation used to capture runtime expectations was temporary and is absent from production and test builds.

`con_notifylines` and its settings field were added concurrently after capture. They are projected out of the original fixtures and checked in a separate load/archive test; `con_notifytime` retains its runtime-only policy.
