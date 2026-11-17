# RiseEngine contract

The canon is `../../riseonly-ios/Packages/RiseEngine/DOCUMENTATION.md`. Read it
in full. `crates/rise-engine` is its file-for-file twin
(`Packages/RiseEngine/Sources/RiseEngine/<path>` -> `crates/rise-engine/src/<path>`).
This document records only what the Rust twin adds or where it differs.

## Composition

`RiseEngine` (`rise_engine.rs`) owns wire, store, sync and upload, as on iOS:
`rise_wire/`, `rise_store/`, `rise_sync/`, `rise_upload/`, `rise_core/`. It does
not own the socket: the host pumps inbound frames in and supplies the outbound
`RiseWireFrameSender`. One account, one engine, one database. Opening an engine
claims the database path for the account in-process (`ownership.release()` on
every exit path of `open`, the Swift `defer`).

Every engine is scoped to `RiseWireAccountContext { account_id, epoch }`. The
epoch is the UUID of `SaiAccountCacheScope`; work for a previous epoch never
publishes into the next one. The desktop home of `Task.detached` is
`desktop_runtime::RiseAccountRuntime` (desktop-only): one tokio runtime per active
account, shut down on logout or switch, so every pending future of the old
account is dropped. iOS fences the same work in
`AccountStateResetCoordinator.hardReset`, which the app ports in iOS order.

A second launch is a separate concern of the app, not of the engine:
`rise_platform::single_instance` takes an advisory `fs4` lock on
`<runtime dir>/instance.lock` and hands argv to the running instance
(`app/src/app/shell/handover.rs`).

## The wire

Frames are JSON over one logical socket. Keys, the outbound frame type, the
success status and request id parsing come from the canon (`rise_canon::rise_wire`,
`rise_canon::rise_wire_canon`, `RiseWireRequestMetadata`), never from literals in
the engine.

- `RiseWireRequestEnvelope` is `{id, type, service, method, data, timestamp,
  metadata}`, written with sorted keys as the Swift encoder with `.sortedKeys`
  writes it. The id is a `RiseRequestID` from the shared rise-rpc allocator.
- `RiseWireFrameDecoder` treats a frame with `request_id` as a response and
  anything else with a type as a push. The tolerances of the deployed gateway are
  kept and tested: a success reply may carry no `status`; the payload sits under
  any of the canon `PAYLOAD_KEYS`; `request_id` arrives as an integer, a float or
  a string (canon `parse_request_id`).
- There is no subscribe protocol: the server fans out by session and the client
  dispatches pushes by event type (`RiseWireEvent`, `RiseSyncPushHandler`).

### HTTP is a second transport

`RiseWireTransport` is `Socket` or `Http`. `Http` posts the same request frame to
`POST /api/rpc` through the host's `RiseWireHttpSender` and needs no socket: the
calls a suspended app still makes. Which methods go that way is the canon's
`RiseEndpoint.httpRoute`, not a client choice. The host owns the address, the
access token and its refresh; the wire owns framing, deadlines and decoding, so
both transports answer the same.

### Models come from the canon

There are no hand-written DTOs. Request bodies, typed replies and views, push
payloads and method names are the client Rust branch of the riseonly-tools emitters in the
`rise-canon` facade: `rise_canon::rise_proto`,
`rise_response`, `rise_request`, `rise_wire_methods`, and so on, with the same
type names as Swift `RiseCanon`. A value the canon lacks is added to the canon, never as a
local struct.

## Replay policy is a backend guarantee

`RiseWireMethodDescriptor` carries `service`, `method`, `timeout` and
`replay_policy`, the twin of the iOS endpoint descriptor (DOCUMENTATION.md
section 4). `RiseWireReplayPolicy` is `Never`, `ReadOnly`, `RequestID` or
`IdempotencyKey { field }` and maps onto the rise-rpc `ReplayPolicy` through
`canon_policy()`. A mutation is replayable only when the backend contract says
so, never because retrying looks harmless from the client; `Never` is the
default for mutations. Durable operations (`RiseSyncDurableEndpoint`) accept only
`RequestID` and `IdempotencyKey`.

## Durable operations

The outbox is durable, as on iOS: an operation is written to the account
database before it is sent (`RiseDurableOperation`, `RiseDurableClaim` in
`rise_store`), claimed and replayed by `RiseSyncDurableHandler` after a restart,
and reported through `durable_lifecycle_events()`, a newest-wins buffered stream
that ends with `Gap` when it drops events, like the Swift
`AsyncStream.bufferingNewest`.

## Storage

SQLite (`rise_sq_lite.rs`) in WAL mode on one writer, schema created at the
current version: greenfield, there is no older desktop database to migrate.
Limits, write policy, preconditions and view keys come from the canon
(`rise-store`). Values are sealed by `RiseAESGCMStoreCipher` with frame bounds and
key length from `rise-crypto`; the additional data binds account, domain and
identity, so a value cannot be replayed into another account's row.

Two data shapes, as on iOS: normalized entities with materialized views for large
realtime sets, and resource values for bounded resources.

## The key and the OS secret store

`RiseKeychainKeyStore` keeps each account's engine key in the OS secret store
under service `net.riseonly.engine` through `rise_platform::secure_store`
(Keychain, Windows Credential Manager, Secret Service on Linux). The desktop
store is shared by every program of the user, so entries carry the bundle
identifier as a namespace, the twin of the iOS access group.

There is no plain-text fallback (01: tokens are never stored in the clear). When
nothing serves the store (on Linux: no owner of `org.freedesktop.secrets`),
`SecretStoreUnavailable::NoService` reaches the sign-in screen as translated text
and the session is not kept past this run.

## HTTP outside the wire

Uploads (`rise_upload/`) talk to the slice of `URLSession` in
`rise_url_session.rs`: one shared reqwest client, the only HTTP the engine opens,
so the system proxy and trust roots of the host plug in once (P22, X24).

## Cross-implementation formats

The desktop database is not opened by any other client. The encryption layout,
the AAD format, position-key encoding and cache filenames are therefore free to
change with the iOS twin; where iOS fixes one of them, the desktop follows iOS.
