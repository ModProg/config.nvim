# Template for Neovim Plugins in Rust

## Installation of finished plugin
### `packer.nvim`
depends on `cargo-make`:

```lua
use {
    'ModProg/rust-nvim-template', 
    run = 'makers release',
    config = function()
        require"module_name".setup()
    end
}
```

### Manually

1. Compile the code with `cargo build --release`
2. Copy `target/release/libcrate_name.so` to your nvim directory `lua/module_name.so`
3. Add `require"module_name".setup()` to your nvim initialization (i.e. `init.lua`)
