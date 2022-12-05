# Config file loader for Neovim

This replaces the need to use Vimscript or Lua for parts of the configuration e.g. keybindings.

## Installation

### Using a package manager

For the installation the [`install`](install)([`install.bat`](install.bat)) script can be used.

Using [packer](https://github.com/wbthomason/packer.nvim) this looks like this:

#### Using the binary
```lua
{
    'ModProg/config.nvim',
    run = function(plugin)
        local await = require 'packer.async'.wait
        local jobs = require 'packer.jobs'

        local neovim_version = vim.version().major .. '.' .. vim.version().minor
        local os
        if jit.os == "Linux" then
            os = "linux"
        elseif jit.os == "Windows" then
            os = "windows"
        else
            os = "macos"
        end
        await(jobs.run({ "./install", "download", os, neovim_version, "latest" }, { cwd = plugin.install_path }))
    end,
}
```

#### Building from source
```lua
{
    'ModProg/config.nvim',
    run = function(plugin)
        local await = require 'packer.async'.wait
        local jobs = require 'packer.jobs'

        local neovim_version = vim.version().major .. '.' .. vim.version().minor
        await(jobs.run({ "./install", "build", neovim_version}, { cwd = plugin.install_path }))
    end,
}
```


### Manually

#### Using the binary

Download the correct version from the [latest release](https://github.com/ModProg/config.nvim/releases/latest), i.e. `linux-0.8.zip` for Linux and Neovim version 0.8.
Extract that folder to somewhere on your runtime path. Make sure the `config.so` stays in the lua folder.

For Linux/OSX you could put it into:

```
~/.local/share/nvim/site/pack/build/start/config.nvim/lua/config.so
```
or
```
~/.config/nvim/lua/pack/plugins/start/config.so
```
or
```
~/.config/nvim/lua/config.so
```

For Windows:
```
~/AppData/Local/nvim/pack/plugins/start/config.nvim/lua/config.dll
```
or
```
~/AppData/Local/nvim/lua/config.dll
```

#### Building from source
##### With [go-task](https://github.com/go-task/task)

```console
task install
```

#### Without go-task

Replace `NEOVIM_VERSION` with your version e.g. 0.8
```console
cargo build --release -F NEOVIM_VERSION --no-default-features
```
Then take the correct file acordion to your operating system and put it in the directory explained [above](#using-the-binary-1):
```console
# On Linux
target/release/libconfig.so -> lua/config.so
# On Windows
target/release/libconfig.dll -> lua/config.dll
# On OSX
target/release/libconfig.dylib -> lua/config.so
```

## Confiuration

User configuration are placed in `~/.config/nvim/config/*.{yaml,toml}`

### Keys

The configuration for keybindings are in the array `keys`.
Using `toml` they can be defined like that:
```toml
[[keys]]
modes = [] # list of modes `modes = ["i", "n", "!"]` or for a single mode also `modes = "n"`
recursive = true # false means `noremap`
command = false # wrapps all bindings in `<CMD>...<CR>`
silent = false # make mappings silent
unique = false # error on duplicate mappings
expression = false # make rhs of mappings evaluated as an expression
leader = "" # prefixes all lhs mappings
# And than add the mappings
j = "gj"
# Should you want to map one of the above, you can put it in the field `mappings`
[keys.mappings]
silent = "supper silent"
```
And in `yaml`
```yaml
keys:
  - modes:
      - n
      - i
    recursive: true
    command: false
    silent: false
    unique: false
    expression: false
    leader: ""
    j: gj
    mappings:
      silent: super silent
```

### Sets
There are a multitude of possibilities to define `set` configurations.

The simplest way is as a list, exspecially in yaml:
```yaml
set:
  - expandtab # Configuring flags like so
  - nonumber # "false" by "no" is also supported
  - number: false # same as this
  - mouse: a # You can directly set values
    completeopt: ["menuone"] # Even multiple in one list item
  - shortmess: # or modify
      append: c
      "+": c
      prepend: c
      "^": c
      remove: c
      "-": c
      # To assign a map there are two ways, either just specify the keys directly
  - listchars:
      # if one of the keys does not match the special keys
      # [append, +, prepend, ^ remove, -, assign, value, =]
      # it will automaticly be assigned as a map
      tab: "<->"
      # The alternative would be to assign using one of the `assign` keys
      assign: {tab: "<->"}
      value: {tab: "<->"}
      "=": {tab: "<->"}
```
In `toml` that would look like this:
```toml
set = [
    "expandtab",
    "nonumber",
    {mouse = "a", completeopt = ["menuone"]},
    {shortmess = {append = "c", "+": ...}},
]
```
therefor the prefered syntax in `toml` would be:
```toml
[set]
flags = [
    "expandtab", # Configuring flags like so
    "nonumber" # "false" by "no" is also supported
]
number = false # same as this
mouse = "a" # You can directly set values
completeopt = ["menuone"]

[set.shortmess] # or modify
append = "c"
"+" = "c"
prepend = "c"
"^" = "c"
remove = "c"
"-" = "c"

# As you normally only want to modify a value once, you can use punctuated keys
# and avoid the extra table
set.shortmess.append = "c"

# To assign a map there are two ways, either just specify the keys directly
[set.listchars]
# if one of the keys does not match the special keys
# [append, +, prepend, ^ remove, -, assign, value, =]
# it will automaticly be assigned as a map
tab = "<->"
# The alternative would be to assign using one of the `assign` keys
assign = { tab = "<->" }
value = { tab = "<->" }
"=" = { tab = "<->" }
# Or using a punctuated header to set `value` directly
[set.listchars.value]
tab = "<->"
```
