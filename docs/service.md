# Run it as a service

`git terrace service` writes a systemd user unit (Linux) or a launchd
LaunchAgent (macOS) that runs one gitterrace server for you.

## Why

- The server is up at login, so its address always answers. Check the
  address with `git terrace config`.
- It restarts on failure and logs to the platform log.
- One server holds every repository. `git terrace` in a repository joins it,
  and the pages and the MCP `--served` registry see it at once.

It keeps the default loopback address and never sets `--public`.

## Install

Give the repositories, or the folders that hold them (searched `multi.depth`
levels down). `--addr` is optional.

```sh
git terrace service print ~/src            # show the unit, write nothing
git terrace service install ~/src          # write it, print the start command
git terrace service install ~/src --now    # write it and start it
```

It records the path of the running program and absolute folders. Paths with
spaces or quotes are escaped. A file it did not write is never replaced.

### Linux (systemd)

The unit is `~/.config/systemd/user/gitterrace.service`.

```sh
systemctl --user enable --now gitterrace.service   # what install prints
systemctl --user status gitterrace.service
journalctl --user -u gitterrace.service -f
```

To start at boot rather than at login: `loginctl enable-linger $USER`.

### macOS (launchd)

The agent is `~/Library/LaunchAgents/dev.gitterrace.server.plist`.

```sh
launchctl load -w ~/Library/LaunchAgents/dev.gitterrace.server.plist
launchctl list | grep gitterrace
tail -f ~/Library/Logs/gitterrace.log
```

### Windows

Not supported: `git terrace service` says so and exits. Run `git terrace` in a
terminal, or start it from Task Scheduler.

## Add and remove a repository live

Run `git terrace` in a repository: it joins the running server and opens its
page. `git terrace remove` takes the current one off. The folders given at
install are scanned when the server starts, so a new clone there appears after
a restart or when you run `git terrace` in it.

## Upgrade

The service keeps running the old program until restarted.

```sh
brew update && brew upgrade gitterrace   # or however you installed
systemctl --user restart gitterrace.service
launchctl kickstart -k gui/$(id -u)/dev.gitterrace.server
```

The other ways to upgrade (the prebuilt download, then cargo) are in
[Getting started](getting-started.md#upgrade). If the program moved, run `git terrace service install` again. It rewrites its
own file.

## Uninstall

```sh
git terrace service uninstall --now
```

It stops the service and removes only the file `install` wrote. Without
`--now` it removes the file and prints the stop command.

## Point an MCP client at it

An MCP client starts its own `git terrace mcp` over stdio. With the service
running, `--served` also reads every repository the server holds. A client
spawns the program without a shell, so the config names `git` and passes
`terrace` as the first argument:

```json
{ "mcpServers": { "gitterrace": { "command": "git", "args": ["terrace", "mcp", "--served"] } } }
```

See [Agents and MCP](mcp.md) and the [API](api.md#mcp-stdio).
