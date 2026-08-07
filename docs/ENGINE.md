# RiseEngine contract

The canon is `../../riseonly-ios/Packages/RiseEngine/DOCUMENTATION.md`. Read it
in full. This records the Rust-specific parts.

## Composition

`rise-engine` owns wire, store, sync and upload. It does not own the socket: the
host pumps inbound bytes in and supplies an outbound sender, exactly as the
reference does. One account, one engine, one database, guarded by an in-process
lease.

On desktop that lease is not enough. The app can be launched twice, so
`rise-platform` also takes an advisory file lock on the database and hands
arguments to the running instance over IPC.

## The wire is JSON over one logical socket

Requests are `{id, type: "service_call", service, method, data, timestamp,
metadata}`, correlated by a 64-bit request id. Anything inbound carrying
`request_id` is a response; anything else with a `type` is a push. There is no
subscribe protocol — the server fans out by session and the client dispatches by
event-type string.

Three deviations from the proto are load-bearing, each covered by a test:

- `status` is omitted on success by the deployed gateway, so absence means
  success;
- the payload is under `data` for some services and `result` for others;
- `request_id` arrives as an integer, a float or a string.

DTOs are hand-written. The gateway flattens nested proto objects into `data`, so
prost-generated structs do not match the wire without a custom serde layer.

## Replay policy is a backend guarantee

`ReplayPolicy` is `Never`, `ReadOnly`, `RequestId` or `IdempotencyKey`. A
mutation is idempotent only when the backend contract says so — never because
retrying looks harmless from the client. `Never` is the default for mutations.

## Storage

SQLite in WAL mode on one writer thread, schema created at the current version
directly: this is greenfield, there is no v1 to replay. Values are sealed with
AES-GCM under additional data binding account, domain and identity, so a value
cannot be replayed into another account's row.

Two data shapes, as in the reference: normalized entities plus materialized views
for large realtime sets, and resource values for bounded resources.

The database key lives in the OS credential store through `rise-platform`. Linux
without a running secret service needs a documented fallback, or the app simply
does not start.

Whether this database is ever opened by a client written for another platform is
open decision 6 in the spec. The answer fixes the encryption layout, the AAD
format, position-key encoding and cache filenames as cross-implementation
formats — or leaves all four free to change.
