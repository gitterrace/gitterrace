# gitterrace

A local, read-only git browser with statistics. Run it inside a repository and
open the printed address.

```bash
gitterrace            # serves the repo you are in on 127.0.0.1:4242
gitterrace ~/src/foo --addr 127.0.0.1:9000
```

Browsing reads git directly. Stats come from a cache in `.git/gitterrace/` built in
the background, one commit at a time; delete it at any time.
