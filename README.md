# spore

Lua runtime with plugin system for the Rhizome ecosystem.

## Crates

| Crate | Description |
|-------|-------------|
| `rhizome-spore-core` | Core runtime infrastructure |
| `rhizome-spore-lua` | Lua runtime with Integration trait |

### Integrations

| Crate | Description |
|-------|-------------|
| `rhizome-spore-moss` | Adds [Moss](https://github.com/rhizome-lab/moss) code analysis to Lua |
| `rhizome-spore-lotus` | Adds [Lotus](https://github.com/rhizome-lab/lotus) world state to Lua |

## Usage

```rust
use rhizome_spore_lua::Runtime;
use rhizome_spore_moss::MossIntegration;

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
