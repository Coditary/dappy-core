# protocol-mux

Protocol-agnostic **multiplexer**: attach N clients to one upstream session, remap
sequence numbers, broadcast events.

Inspired by [Dapper](https://github.com/facebookexperimental/dapper) / [dap-mux](https://github.com/dap-mux/dap-mux).

## API

| Type | Role |
|------|------|
| `MultiplexSession` | Async event loop: upstream channel + client attach/detach |
| `ClientEndpoint` | Send requests, receive responses (`recv`) and events (`events`) |
| `MessageRemapper` | Map editor client seq ↔ upstream seq |
| `translate_cancel` | Rewrite `cancel.requestId` on the editor path |
| `LateJoinCache` | Replay `initialized` + last `stopped` to late attachers |

### Client roles

- **Editor** — seq remapping on requests/responses; receives events on `recv`
- **Control / Agent / Other** — use upstream seq directly; events via `events` broadcast

## Example (in-memory test channels)

```rust
use protocol_mux::{ClientRole, MultiplexSession, request};
use tokio::sync::mpsc;

let (to_upstream, from_upstream) = mpsc::unbounded_channel();
let (session, _join) = MultiplexSession::start(from_upstream, to_upstream);

let mut editor = session.attach(ClientRole::Editor).await?;
editor.send(request(1, "initialize", None))?;
```

## Tests

```bash
cargo test -p protocol-mux
```

Integration tests in `tests/multiplex_e2e.rs` cover two clients + fake upstream,
per-client response routing, cancel translation, and late-join replay.

```toml
protocol-mux = { path = "../protocol-mux" }
```
