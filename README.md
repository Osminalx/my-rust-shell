# my-rust-shell

A POSIX-compliant shell written in Rust — built from scratch as a learning project to understand shell internals: parsing, job control, pipelines, redirections, and terminal interaction.

## Features

| Feature | Description |
|---------|-------------|
| **Builtins** | `cd`, `echo`, `exit`, `pwd`, `type`, `history`, `declare`, `jobs`, `complete` |
| **Pipelines** | Chain commands with `\|` |
| **Redirections** | `>` (overwrite), `>>` (append), `2>` (stderr redirect) |
| **Job control** | Background processes with `&`, list jobs with `jobs` |
| **Variable expansion** | `$var` and `${var}` syntax |
| **Quoting** | Single quotes (literal), double quotes (expand `$`), escape sequences with `\` |
| **Tab completion** | Commands (builtins + PATH), filenames, custom completers via `complete -C` |
| **History** | Persistent history via `HISTFILE` env var, `history` builtin with read/write/append |
| **REPL** | Interactive prompt with rustyline (readline-like editing) |

## Quick start

```sh
# Build and run
cargo run

# Or build first, then run
cargo build --release
./target/release/my-rust-shell
```

Once inside the shell:

```sh
$ echo "hello world"
hello world
$ ls -la | head -3
$ pwd
/home/user/projects
$ history
    1  echo "hello world"
    2  ls -la | head -3
```

## Usage

### Navigation

```sh
cd /path/to/dir     # change directory
cd ~                # go home
cd                  # same as cd ~
pwd                 # print working directory
```

### Redirections

```sh
echo "log entry" >> file.log     # append to file
echo "new file" > output.txt     # overwrite file
cat missing.txt 2> errors.log    # redirect stderr
```

### Pipelines

```sh
cat large.log | grep error | head -10
ls -la | sort -k5 -n | tail -3
```

### Background jobs

```sh
sleep 30 &
[1] 12345
jobs
[1]  Running    sleep 30 &
```

### Variables

```sh
declare name="world"
echo hello $name                  # → hello world
echo "hello $name"                # → hello world (double-quoted expands)
echo 'hello $name'                # → hello $name (single-quoted literal)
echo pre${name}post               # → preworldpost (brace syntax)
```

### Tab completion

- Type a command prefix and press **Tab** to autocomplete builtins or executables in PATH.
- After a command, Tab completes filenames.
- Register custom completers: `complete -C /path/to/completer command_name`

### History

By default, sessions are ephemeral. To persist history across sessions:

```sh
export HISTFILE=~/.my_shell_history
```

Then use the `history` builtin:

```sh
history              # show all entries
history 10           # show last 10 entries
history -r file      # read history from file
history -w file      # write history to file
history -a file      # append new entries to file
```

## Project structure

```
src/
├── main.rs         # REPL loop, line input, history load
├── args.rs         # Tokenizer, parser, variable expansion
├── commands.rs     # Builtin commands, pipeline execution, PATH lookup
├── completer.rs    # Tab completion (commands, files, custom)
└── jobs.rs         # Background job table and status tracking
```

## Development

```sh
cargo test          # run unit tests
cargo test -- --nocapture  # show test output
```

## License

MIT
