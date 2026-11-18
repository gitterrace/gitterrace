// The demo corpus, shared by the screenshots (scripts/release/site/site-shots-run.mjs) and the
// live demo (scripts/release/demo-export.mjs). Not run on its own.
// - PINS: jq, ripgrep and gitterrace at the releases `[workspace.metadata.demo] repos` in
//   Cargo.toml names.
//   An xtask test fails when the two drift, so the demo and the screenshots show the same trees.
// - checkout(): clones each into ~/.cache/gitterrace-shots (reused when present) and puts a
//   `release` branch at the pin with a clean worktree.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

// The shot paths in site-shots fit these commits (ripgrep 068a63e is in 15.2.0, and the blame
// and send files there exist at these tags).
export const PINS = {
  jq: {
    url: 'https://github.com/jqlang/jq',
    tag: 'jq-1.8.2',
    commit: '34f7186b86743a083a589741b6cea95293524108',
  },
  ripgrep: {
    url: 'https://github.com/BurntSushi/ripgrep',
    tag: '15.2.0',
    commit: 'e89fff89ac9af12e8d4ce9d5fd07beb408ca730f',
  },
  // gitterrace itself at its release: the demo shows the code that serves it. Pinned by its
  // tag alone (a commit of this repository is never written in its own tree); checkout()
  // resolves it. Each release moves this pin and the Cargo.toml demo entry to the new tag.
  gitterrace: {
    url: 'https://github.com/gitterrace/gitterrace',
    tag: 'v0.1.1',
  },
};

export const corpus = join(homedir(), '.cache/gitterrace-shots');

const git = (cwd, ...a) =>
  execFileSync('git', ['-C', cwd, ...a], { stdio: ['ignore', 'pipe', 'inherit'] })
    .toString()
    .trim();

/** Clones or reuses every pinned repository and returns its folder by name. */
export function checkout() {
  mkdirSync(corpus, { recursive: true });
  const dirs = {};
  for (const [name, { url, tag, commit: pinned }] of Object.entries(PINS)) {
    const dir = join(corpus, name);
    if (!existsSync(join(dir, '.git'))) {
      execFileSync('git', ['clone', '-q', `${url}.git`, dir], { stdio: 'inherit' });
    }
    // A pin without a commit is a tag of this repository: fetched, then read as its commit.
    if (!pinned) git(dir, 'fetch', '-q', '--force', 'origin', `refs/tags/${tag}:refs/tags/${tag}`);
    const commit = pinned ?? git(dir, 'rev-parse', `refs/tags/${tag}^{commit}`);
    try {
      git(dir, 'cat-file', '-e', `${commit}^{commit}`);
    } catch {
      git(dir, 'fetch', '-q', 'origin', commit);
    }
    // HEAD on `release` at the pin, a clean worktree: every reader sees the same tree.
    git(dir, 'checkout', '-q', '-f', '-B', 'release', commit);
    git(dir, 'clean', '-qfdx', '-e', '.git');
    dirs[name] = dir;
  }
  return dirs;
}
