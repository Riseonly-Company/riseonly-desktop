# Store architecture, in Rust ownership terms

The canon is `../../riseonly-ios/STORE_ARCHITECTURE.md`. Read it in full. This
document records only what changes when the same design is expressed in Rust and
GPUI instead of Swift and SwiftUI.

## The data flow is unchanged

```
View -> InteractionsStore -> ActionsStore -> account-scoped domain
     -> actor repository -> engine RPC / RiseStore
     -> immutable presentation snapshot -> ServicesStore -> View
```

A view calls only `InteractionsStore`. A store never owns transport, database,
engine lifecycle or a server cache. The source of truth for server data is
RiseStore.

## What Swift constructs become

| Swift | Rust |
|---|---|
| `actor Repository` | a tokio task owning its state, driven by an `mpsc` command channel |
| `@MainActor @Observable final class` | `Entity<T>` on the GPUI thread |
| `static let shared` | a field on the per-account root entity |
| `Task.detached(priority:)` | `tokio::spawn`, or `spawn_blocking` for disk and CPU |
| `DispatchQueue` (serial) | a dedicated thread plus a channel |
| `AsyncStream.bufferingNewest(n)` | `tokio::sync::broadcast`; map `Lagged` to an explicit overflow error |
| `CheckedContinuation` | `oneshot` |
| `deinit` | `Drop` for cheap cleanup, plus an explicit `close().await` — `Drop` cannot await |
| `precondition` in an id wrapper | `TryFrom` at the boundary, `debug_assert!` inside |

A real actor — a task plus a command channel — rather than a mutex, because the
canon's guarantees depend on operations being serialised. A mutex gives mutual
exclusion but not ordering, and every generation and reconciliation rule assumes
ordering.

## Singletons become ownership

The reference keeps roughly sixty registered stores and an
`AccountStateResetCoordinator` that tears them down in a specific order. That
order is a correctness invariant: getting it wrong leaks one account's data into
another.

Here there is one root entity per account, dropped whole on account switch.
Isolation stops being a property of calling reset correctly and becomes a
property of ownership, which the compiler already enforces.

Stores must still be classified: `NotifierStore` and window presenters are
process-global, `ChatServicesStore` and `MusicServicesStore` are per-account. The
reference list mixes both.

## The failure mode inverts

SwiftUI tracks dependencies automatically and the risk is over-invalidation —
reading one observable field in a heavy subtree rediffs it.

GPUI has no automatic tracking. `cx.notify()` is explicit, and a forgotten call
shows stale data with no warning at all. The discipline is the opposite one:
every mutation of observed state ends in a notify, and `Entity::cached` is the
only structural performance lever.

Element identity has the same weight as list identity in the reference. A
`GlobalElementId` drives element state and accessibility node identity, and
duplicate ids are silently dropped in release builds. Use stable server ids,
never indices.
