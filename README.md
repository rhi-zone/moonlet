# moonlet

Lua runtime with plugin system for the rhi ecosystem.

## Crates

| Crate | Description |
|-------|-------------|
| `moonlet-core` | Core runtime infrastructure |
| `moonlet-lua` | Lua runtime with Integration trait |

### Integrations

| Crate | Description |
|-------|-------------|
| `moonlet-moss` | Adds [Moss](https://github.com/rhizome-lab/moss) code analysis to Lua |
| `moonlet-lotus` | Adds [Lotus](https://github.com/rhizome-lab/lotus) world state to Lua |

## Usage

```rust
use moonlet_lua::Runtime;
use moonlet_moss::MossIntegration;

let runtime = Runtime::new()?;
runtime.register(&MossIntegration::new("."))?;

// Now Lua scripts can use moss.view(), moss.edit(), moss.analyze.*, etc.
runtime.run_file(Path::new("scripts/analyze.lua"))?;
```

## Development

```bash
nix develop        # Enter dev shell
cargo test         # Run tests
cargo clippy       # Lint
```

## License

MIT OR Apache-2.0
