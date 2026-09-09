# protocol-gateway

Protocol-agnostic **gateway** layer: plugin registry, routing, and upstream spawn/connect specs.

Reusable for DAP adapters, LSP language servers, or any hub that must pick one backend
from many registered plugins.

```toml
protocol-gateway = { path = "../protocol-gateway" }
```
