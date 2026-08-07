# Architectural boundaries

Three rules. `scripts/check-boundaries.py` fails CI on any violation, because
each one decays silently if only a human watches it.

## 1. `rise-engine` never imports gpui or an OS crate

The engine is the layer a future iOS, Android or wasm consumer would adopt. The
moment it knows about a UI toolkit or a platform API, that option closes and the
only way back is a rewrite. It also keeps the engine testable headless, which is
why its whole test suite runs without a display.

Checked in the manifest and in every source file of the crate.

## 2. `#[cfg(target_os)]` lives only in `rise-platform` and `rise-media`

A feature module must never learn which OS it is on. Once a `cfg` appears in a
screen, cross-platform behaviour stops being a property of the design and
becomes three diverging branches that nobody builds on the other two OSes.

`rise-media` is the second exception only because hardware decode genuinely
differs per platform below the abstraction.

Note that gpui itself is not uniform: on Windows `hide_other_apps` and
`unhide_other_apps` are live `unimplemented!()` calls, and `start_external_drag`
does not exist. Go through `rise_platform::gpui_shim`, never gpui directly.

## 3. `stores/` never imports `rise_engine`; `engine/` never imports `gpui`

This is `STORE_ARCHITECTURE.md` expressed as a check. A store is a facade: it
issues an intent and reads a prepared snapshot. The moment it can reach the
engine, it starts owning transport, cache and response state, which is exactly
the architecture the reference migrated away from.

The reverse direction matters as much: `engine/` owns data, not presentation.

## Deliberate deviations from riseonly-ios

The reference is the source of truth for structure and behaviour, not for crate
boundaries — it has none, because it is a single build target.

- **Modules are crates and never depend on each other.** In the reference `user`
  reaches into 32 other stores and `story` calls `ChatActionsStore`. Between
  crates that graph is a cycle. Cross-module needs go through `rise-navigation`
  (open a screen) or `rise-contracts` (a trait resolved at the composition root).
- **`Core/` is dissolved.** Theme, UI kit, widgets, i18n and runtime plumbing are
  used by every module, so they are crates, not a folder inside the app.
- **Account isolation is ownership, not teardown order.** The reference keeps ~60
  singletons and a coordinator that resets them in a specific sequence. Here a
  per-account root entity is dropped whole on account switch.
- **`RiseCore` is split.** Identifiers went to `rise-core`, the secure key store
  to `rise-platform`, because one is portable and the other is the definition of
  platform-specific.
