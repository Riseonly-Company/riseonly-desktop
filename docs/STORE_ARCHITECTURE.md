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

## How Swift constructs become Rust

The rules: a store is a
gpui entity reached through the kit's `rise_ui::shared`, the way iOS uses
`static let shared`; an iOS `actor` is a struct over `rise_core::RiseIsolated`;
account reset is ported 1 to 1 through `AccountStateResetCoordinator`, in the
same order as iOS. Precedence: 01 > 02 > 03 > this document.

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
