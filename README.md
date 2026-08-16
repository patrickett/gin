# The Gin Programming Language

> **⚠️ Experimental** — Gin is in active development. Everything is subject to change. Not yet ready for production use.

Gin is a systems programming language where every type is a user-defined algebraic data type — there are no built-in primitives. It features structural traits, inferred compile-time evaluation (no explicit `const`/`comptime` keywords), and indentation-sensitive syntax. See the [development roadmap](docs/dev/ROADMAP.md) for planned milestones.

## Example

```gin
use core.Int
use core.default.Default

--- A tagged union with literal variants
LogLevel is 'debug'
         or 'info'
         or 'warn'
         or 'error'

--- A record type providing Default
Task has Default
    name     String
    priority Int
    level    LogLevel

    Default.default: (name: 'unnamed', priority: 0, level: 'info')

--- Compile-time-inferred function via `:=` (constant binding)
log_prefix(level LogLevel) String := when level is
    'debug' then '[DBG]'
    'info'  then '[INF]'
    'warn'  then '[WRN]'
    'error' then '[ERR]'

--- Function with control flow
main:
    task := Task.default
    prefix := log_prefix(task.level)
    log := prefix + ' ' + task.name
return 0
```
