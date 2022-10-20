# Config file loader for Neovim

This replaces the need to use Vimscript or Lua for parts of the configuration e.g. keybindings.

## Installation

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
