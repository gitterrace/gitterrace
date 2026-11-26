#!/usr/bin/env bash
# reach-stats.sh: an on-demand adoption report across products. Never part of the gate.
#
#   reach-stats.sh [--json] [--no-traffic] [product-root ...]
#
# Product roots come from the arguments, else from $REACH_PRODUCTS (whitespace separated).
# Read-only against GitHub, through `gh api` only. The one other call is crates.io through curl,
# and only for a product that sets `crate`. Nothing is sent anywhere.
#
# Config: <root>/schwarzwald/config/reach.toml
#   repo        = "owner/name"      required
#   tap         = "owner/name"      optional, the brew tap repo (its clone traffic is a proxy)
#   crate       = "name"            optional, crates.io crate
#   ci_baseline = 0                 downloads per release that our own CI or release checks cause
#   [assets]                        glob on the asset name = "os/arch"
#   "*darwin*arm64*" = "darwin/arm64"
#
# History: GitHub keeps traffic for 14 days only. Each run writes one snapshot per product and UTC
# day to ${XDG_DATA_HOME:-$HOME/.local/share}/schwarzwald/reach/<owner>__<repo>.jsonl, a second run
# the same day replaces that day's line. Nothing is written inside a repo.
#
# Output: one table, each cell "value (change vs the snapshot nearest 7 days earlier)", "n/a" when
# there is none. Downloads are net of ci_baseline times the number of releases, marked approx.
# Homebrew has no install counts for third-party taps: tap clones and darwin downloads are proxies.
# A 403 on traffic means no push rights: it is reported and the public numbers still print.
# Exit: 0 a report, 2 a setup error (gh missing or not authed, bad config), with a remedy line.
set -uo pipefail

die() { # die <what failed> <remedy>
  echo "reach-stats: $1" >&2; echo "reach-stats: remedy: $2" >&2; exit 2
}

json=0; traffic=1; roots=()
while [ $# -gt 0 ]; do
  case $1 in
    --json) json=1 ;;
    --no-traffic) traffic=0 ;;
    -h | --help) sed -n '2,24p' "${BASH_SOURCE[0]}"; exit 0 ;;
    -*) die "unknown option $1" "usage: reach-stats.sh [--json] [--no-traffic] [product-root ...]" ;;
    *) roots+=("$1") ;;
  esac
  shift
done
if [ ${#roots[@]} -eq 0 ] && [ -n "${REACH_PRODUCTS:-}" ]; then
  # shellcheck disable=SC2206
  roots=($REACH_PRODUCTS)
fi
[ ${#roots[@]} -gt 0 ] || die "no product given" "pass one or more product roots, or set REACH_PRODUCTS"

PY=""
for cand in python3.15 python3.14 python3.13 python3.12 python3.11 python3; do
  if command -v "$cand" >/dev/null 2>&1 && "$cand" -c 'import tomllib' 2>/dev/null; then PY=$cand; break; fi
done
[ -n "$PY" ] || die "no python with tomllib (3.11 or newer)" "install python 3.11 or newer"
command -v gh >/dev/null 2>&1 || die "gh is not installed" "install the GitHub CLI (https://cli.github.com) and re-run"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated" "run: gh auth login"

exec "$PY" - "$json" "$traffic" "${roots[@]}" <<'PY'
import fnmatch, json, os, subprocess, sys, tomllib
from datetime import datetime, timedelta, timezone

as_json, traffic = sys.argv[1] == "1", sys.argv[2] == "1"
roots = sys.argv[3:]


def fail(what, remedy):
    print(f"reach-stats: {what}", file=sys.stderr)
    print(f"reach-stats: remedy: {remedy}", file=sys.stderr)
    sys.exit(2)


def load_config(root):
    path = os.path.join(root, "schwarzwald", "config", "reach.toml")
    if not os.path.exists(path):
        fail(f"{path} not found", "create it with at least: repo = \"owner/name\"")
    try:
        with open(path, "rb") as fh:
            cfg = tomllib.load(fh)
    except tomllib.TOMLDecodeError as e:
        fail(f"cannot read {path}: {e}", "fix the TOML syntax")
    repo = cfg.get("repo")
    if not isinstance(repo, str) or repo.count("/") != 1 or not all(repo.split("/")):
        fail(f"{path} names no valid repo", "set repo = \"owner/name\"")
    tap = cfg.get("tap")
    if tap is not None and (not isinstance(tap, str) or tap.count("/") != 1):
        fail(f"{path}: tap must be \"owner/name\"", "fix tap or remove it")
    base = cfg.get("ci_baseline", 0)
    if not isinstance(base, int) or isinstance(base, bool) or base < 0:
        fail(f"{path}: ci_baseline must be a non-negative integer", "fix ci_baseline")
    assets = cfg.get("assets", {})
    if not isinstance(assets, dict) or not all(isinstance(v, str) and v.count("/") == 1 for v in assets.values()):
        fail(f"{path}: [assets] maps a glob to \"os/arch\"", "fix the [assets] table")
    return {"repo": repo, "tap": tap, "crate": cfg.get("crate"), "ci_baseline": base, "assets": assets}


configs = [load_config(r) for r in roots]  # validate every product before any call


class Forbidden(Exception):
    pass


def gh(*args):
    p = subprocess.run(["gh", *args], capture_output=True, text=True)
    if p.returncode != 0:
        err = p.stderr + p.stdout
        if "403" in err:
            raise Forbidden(err)
        raise RuntimeError(err.strip() or "gh failed")
    return json.loads(p.stdout) if p.stdout.strip() else None


notes = []
today = datetime.now(timezone.utc).date()
today_s = os.environ.get("REACH_TODAY") or today.isoformat()
today = datetime.strptime(today_s, "%Y-%m-%d").date()
hist_dir = os.path.join(os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share"),
                        "schwarzwald", "reach")


def traffic_pair(repo, kind, label):
    try:
        d = gh("api", f"repos/{repo}/traffic/{kind}")
        return d.get("count"), d.get("uniques")
    except Forbidden:
        raise
    except RuntimeError as e:
        notes.append(f"{repo}: {label} unavailable ({e.splitlines()[0] if e else 'error'})")
        return None, None


def collect(cfg):
    repo = cfg["repo"]
    owner, name = repo.split("/")
    snap = {"day": today_s, "repo": repo}
    r = gh("api", f"repos/{repo}")
    snap.update(stars=r.get("stargazers_count"), forks=r.get("forks_count"),
                watchers=r.get("subscribers_count"), open_issues=r.get("open_issues_count"))
    snap["discussions"] = None
    try:
        q = "query($o:String!,$n:String!){repository(owner:$o,name:$n){hasDiscussionsEnabled discussions{totalCount}}}"
        g = gh("api", "graphql", "-f", f"query={q}", "-f", f"o={owner}", "-f", f"n={name}")
        rep = ((g or {}).get("data") or {}).get("repository") or {}
        if rep.get("hasDiscussionsEnabled"):
            snap["discussions"] = (rep.get("discussions") or {}).get("totalCount")
    except (RuntimeError, Forbidden):
        pass  # discussions off or not readable: skip cleanly
    snap.update(views=None, views_uniques=None, clones=None, clones_uniques=None,
                referrers=None, paths=None, tap_clones=None, tap_uniques=None)
    if traffic:
        try:
            snap["views"], snap["views_uniques"] = traffic_pair(repo, "views", "views")
            snap["clones"], snap["clones_uniques"] = traffic_pair(repo, "clones", "clones")
            snap["referrers"] = gh("api", f"repos/{repo}/traffic/popular/referrers")
            snap["paths"] = gh("api", f"repos/{repo}/traffic/popular/paths")
            if cfg["tap"]:
                snap["tap_clones"], snap["tap_uniques"] = traffic_pair(cfg["tap"], "clones", "tap clones")
        except Forbidden:
            print(f"traffic needs push access to {repo}; run with a token that has it, or skip with --no-traffic")
            notes.append(f"{repo}: traffic needs push access")
        except RuntimeError as e:
            notes.append(f"{repo}: traffic unavailable ({e.splitlines()[0] if e else 'error'})")
    rels = []
    try:
        rels = gh("api", f"repos/{repo}/releases?per_page=100") or []
    except (RuntimeError, Forbidden) as e:
        notes.append(f"{repo}: releases unavailable ({str(e).splitlines()[0] if str(e) else 'error'})")
    per_tag, by_target, gross, unmatched = {}, {}, 0, {}
    for rel in rels:
        tag = rel.get("tag_name")
        per_tag[tag] = {}
        for a in rel.get("assets", []):
            n = a.get("download_count", 0)
            target = next((v for k, v in cfg["assets"].items() if fnmatch.fnmatch(a.get("name", ""), k)), None)
            if target is None:
                # Checksums, installers and the like: listed, never counted as a download.
                unmatched[a.get("name", "")] = unmatched.get(a.get("name", ""), 0) + n
                continue
            per_tag[tag][a["name"]] = n
            by_target[target] = by_target.get(target, 0) + n
            gross += n
    # ci_baseline is per release over all counted assets, so it comes off the total only.
    mac = sum(n for t, n in by_target.items() if t.split("/")[0].lower() in ("darwin", "macos", "mac", "osx", "apple"))
    snap.update(releases=len(rels), downloads_gross=gross,
                downloads_net=max(0, gross - cfg["ci_baseline"] * len(rels)), ci_baseline=cfg["ci_baseline"],
                downloads_by_tag={t: sum(v.values()) for t, v in per_tag.items()},
                downloads_by_target=by_target, downloads_unmatched=unmatched,
                brew_install_proxy=mac,
                crate_downloads=None)
    if cfg["crate"]:
        ua = f"schwarzwald-reach-stats (+https://github.com/{repo})"
        p = subprocess.run(["curl", "-sS", "-f", "-m", "30", "-A", ua,
                            f"https://crates.io/api/v1/crates/{cfg['crate']}"], capture_output=True, text=True)
        try:
            snap["crate_downloads"] = json.loads(p.stdout)["crate"]["downloads"]
        except Exception:
            notes.append(f"{repo}: crates.io lookup for {cfg['crate']} failed")
    return snap


def history(repo):
    path = os.path.join(hist_dir, repo.replace("/", "__") + ".jsonl")
    rows = []
    if os.path.exists(path):
        for line in open(path):
            line = line.strip()
            if line:
                try:
                    rows.append(json.loads(line))
                except ValueError:
                    pass
    return path, rows


def save(path, rows, snap):
    rows = [r for r in rows if r.get("day") != snap["day"]] + [snap]
    rows.sort(key=lambda r: r["day"])
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as fh:
        for r in rows:
            fh.write(json.dumps(r, sort_keys=True) + "\n")
    os.replace(tmp, path)


def nearest(rows):
    target = today - timedelta(days=7)
    older = [r for r in rows if r.get("day", "") < today_s]
    if not older:
        return None
    return min(older, key=lambda r: abs((datetime.strptime(r["day"], "%Y-%m-%d").date() - target).days))


results = []
for cfg in configs:
    try:
        snap = collect(cfg)
    except Forbidden as e:
        fail(f"GitHub refused access to {cfg['repo']}", "use a token that can read the repo")
    except RuntimeError as e:
        fail(f"cannot read {cfg['repo']}: {e}", "check the repo name in reach.toml and your gh login")
    path, rows = history(cfg["repo"])
    prev = nearest(rows)
    save(path, rows, snap)
    delta = {}
    for k, v in snap.items():
        pv = prev.get(k) if prev else None
        if isinstance(v, int) and not isinstance(v, bool) and isinstance(pv, int):
            delta[k] = v - pv
        elif isinstance(v, int) and not isinstance(v, bool):
            delta[k] = None
    results.append({"repo": cfg["repo"], "snapshot": snap, "delta": delta,
                    "baseline_day": prev["day"] if prev else None, "history": path})

if as_json:
    json.dump({"day": today_s, "products": results, "notes": notes}, sys.stdout, indent=1)
    print()
    sys.exit(0)


def cell(res, key, key2=None, approx=False):
    s, d = res["snapshot"], res["delta"]
    v = s.get(key)
    if v is None:
        return "-"
    out = str(v) + ("~" if approx else "")
    if key2:
        out += "/" + (str(s.get(key2)) if s.get(key2) is not None else "-")
    dv = d.get(key)
    out += f" ({dv:+d})" if dv is not None else " (n/a)"
    return out


cols = [("repo", None), ("stars", ("stars",)), ("forks", ("forks",)), ("watch", ("watchers",)),
        ("issues", ("open_issues",)), ("disc", ("discussions",)),
        ("views 14d", ("views", "views_uniques")), ("clones 14d", ("clones", "clones_uniques")),
        ("downloads approx", ("downloads_net", None, True)),
        ("tap clones 14d", ("tap_clones", "tap_uniques")), ("brew proxy", ("brew_install_proxy",)),
        ("crate", ("crate_downloads",))]
table = [[c[0] for c in cols]]
for res in results:
    table.append([res["repo"] if a is None else cell(res, *a) for _, a in cols])
widths = [max(len(r[i]) for r in table) for i in range(len(cols))]
for i, row in enumerate(table):
    print("  ".join(c.ljust(widths[j]) for j, c in enumerate(row)).rstrip())
    if i == 0:
        print("  ".join("-" * w for w in widths))
print()
for res in results:
    s = res["snapshot"]
    print(f"{res['repo']}: change is against {res['baseline_day'] or 'no earlier snapshot'}; "
          f"{s['releases']} releases, ci_baseline {s['ci_baseline']} each, downloads net approx {s['downloads_net']} (gross {s['downloads_gross']})")
    for t, n in sorted(s["downloads_by_target"].items()):
        print(f"  downloads {t}: {n}")
    for t, n in s["downloads_by_tag"].items():
        print(f"  release {t}: {n}")
    if s.get("downloads_unmatched"):
        print(f"  not counted (no [assets] pattern): {sum(s['downloads_unmatched'].values())} over "
              f"{len(s['downloads_unmatched'])} assets, e.g. {', '.join(sorted(s['downloads_unmatched'])[:3])}")
    for ref in (s.get("referrers") or [])[:5]:
        print(f"  referrer {ref.get('referrer')}: {ref.get('count')}/{ref.get('uniques')}")
    for p in (s.get("paths") or [])[:5]:
        print(f"  path {p.get('path')}: {p.get('count')}/{p.get('uniques')}")
print("Homebrew has no install counts for third-party taps: tap clones and darwin downloads are proxies only.")
for n in notes:
    print(f"note: {n}")
PY
