#!/usr/bin/env python3
"""Benchmark harness for gitterrace on large public repos (stdlib, git, the release binary).

Run by `make bench`. Reads bench/corpus.toml, clones each repo into $GITTERRACE_BENCH_DIR
(default ~/.cache/gitterrace-bench), starts `git-terrace` on a free port per repo, measures it,
and writes bench/results/<date>-<host>.json. Methodology: docs/benchmarks.md.

  scripts/bench-run.py [--repos a,b] [--large] [--runs N] [--bin PATH] [--idle-secs S]
  scripts/bench-run.py --synthetic      # tiny generated repo, one run: the cheap guard
"""
import argparse, datetime, http.client, json, os, platform, re, shutil, socket, statistics
import subprocess, sys, tempfile, threading, time
from pathlib import Path
from urllib.parse import quote

ROOT = Path(__file__).resolve().parent.parent
STATS = ["", "/types", "/languages", "/frequency", "/hotspots", "/ownership", "/patterns",
         "/contributors"]
TEXT_BAD = {".png", ".jpg", ".jpeg", ".gif", ".ico", ".woff", ".woff2", ".pdf", ".zip", ".gz",
            ".bin", ".svg", ".lock", ".so", ".a", ".o", ".wasm", ".pack", ".idx", ".mp4"}


def parse_toml(text):
    """The small TOML subset corpus.toml uses: [[array]], [a.b] tables, scalars."""
    out, cur = {}, None
    for raw in text.splitlines():
        line = re.sub(r"\s+#.*$", "", raw).strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[["):
            cur = {}
            out.setdefault(line[2:-2].strip(), []).append(cur)
        elif line.startswith("["):
            cur = out
            for part in line[1:-1].strip().split("."):
                cur = cur.setdefault(part, {})
        else:
            k, v = (s.strip() for s in line.split("=", 1))
            cur[k] = json.loads(v) if v[0] in '"[0123456789' else v == "true"
    return out


def run(cmd, cwd=None, check=True):
    return subprocess.run(cmd, cwd=cwd, check=check, capture_output=True, text=True).stdout


def free_gb(path):
    return shutil.disk_usage(path).free / 2**30


def dir_mb(path):
    n = 0
    for p in Path(path).rglob("*"):
        try:
            if p.is_file() and not p.is_symlink():
                n += p.stat().st_size
        except OSError:
            pass
    return n / 2**20


def rss_mb(pid):
    if Path(f"/proc/{pid}/status").exists():
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) / 1024
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout
    return int(out.strip() or 0) / 1024


def cpu_secs(pid):
    """Total CPU seconds of a process, from /proc on Linux and ps on macOS."""
    stat = Path(f"/proc/{pid}/stat")
    if stat.exists():
        f = stat.read_text().rsplit(")", 1)[1].split()
        return (int(f[11]) + int(f[12])) / os.sysconf("SC_CLK_TCK")
    t = subprocess.run(["ps", "-o", "time=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    secs = 0.0
    for part in t.split(":"):
        secs = secs * 60 + float(part)
    return secs


def stats(xs):
    return {"median": statistics.median(xs), "min": min(xs), "max": max(xs), "n": len(xs)}


def get(port, path, timeout=600):
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    t0 = time.perf_counter()
    c.request("GET", path)
    r = c.getresponse()
    t1 = time.perf_counter()
    body = r.read()
    t2 = time.perf_counter()
    c.close()
    return r.status, (t1 - t0) * 1000, (t2 - t0) * 1000, len(body)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Server:
    def __init__(self, binary, repo, logdir):
        self.port = free_port()
        self.log = open(Path(logdir) / f"server-{self.port}.log", "w")
        self.t0 = time.perf_counter()
        self.proc = subprocess.Popen([binary, str(repo), "--addr", f"127.0.0.1:{self.port}"],
                                     stdout=self.log, stderr=self.log, cwd=repo)
        self.pid = self.proc.pid

    def wait_ready(self, timeout=14400):
        """Index build: seconds until /api/progress says ready, and peak RSS meanwhile."""
        peak, listening = 0.0, None
        while time.perf_counter() - self.t0 < timeout:
            if self.proc.poll() is not None:
                raise RuntimeError(f"server exited early, see {self.log.name}")
            peak = max(peak, rss_mb(self.pid))
            try:
                _, _, _, _ = (None,) * 4
                c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
                c.request("GET", "/api/progress")
                body = json.loads(c.getresponse().read())
                c.close()
                listening = listening or time.perf_counter() - self.t0
                if body.get("ready"):
                    return time.perf_counter() - self.t0, listening, peak
            except (OSError, ValueError):
                pass
            time.sleep(0.2)
        raise RuntimeError("index did not become ready in time")

    def stop(self):
        self.proc.terminate()
        try:
            self.proc.wait(30)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()


def ensure_clone(repo, base, logdir):
    """Bare, blobless clone of the pinned sha, then every blob of its history."""
    sha, dest = repo["sha"], Path(base) / f"{repo['name']}.git"
    mark = dest / "bench-ready"
    if mark.exists() and mark.read_text().strip() == sha:
        return dest
    if repo.get("local"):
        return (ROOT / repo["local"]).resolve()
    need = repo.get("need_gb", 1)
    if free_gb(base) < need:
        sys.exit(f"refusing to clone {repo['name']}: {free_gb(base):.1f} GB free in {base}, needs {need} GB")
    if not dest.exists():
        run(["git", "init", "-q", "--bare", str(dest)])
        run(["git", "remote", "add", "origin", repo["url"]], cwd=dest)
    print(f"fetching {repo['name']} @ {sha[:10]} into {dest}", flush=True)
    run(["git", "fetch", "-q", "--filter=blob:none", "origin", sha], cwd=dest)
    run(["git", "update-ref", "refs/heads/bench", sha], cwd=dest)
    run(["git", "symbolic-ref", "HEAD", "refs/heads/bench"], cwd=dest)
    print(f"backfilling blobs of {repo['name']}", flush=True)
    run(["git", "backfill"], cwd=dest)
    mark.write_text(sha + "\n")
    return dest


def discover(repo_dir, rev):
    """Probe defaults: largest text file, deepest folder, newest commit."""
    files = []
    for line in run(["git", "ls-tree", "-r", "-l", "-z", rev], cwd=repo_dir).split("\0"):
        if "\t" not in line:
            continue
        meta, path = line.split("\t", 1)
        size = meta.split()[-1]
        if size.isdigit() and Path(path).suffix.lower() not in TEXT_BAD:
            files.append((int(size), path))
    files.sort(reverse=True)
    deep = max((p.rsplit("/", 1)[0] for _, p in files if "/" in p), key=lambda d: d.count("/"), default="")
    return {"blob": files[0][1], "blame": files[0][1], "deep_tree": deep,
            "commit": run(["git", "rev-parse", rev], cwd=repo_dir).strip()}


def pages(probe, search):
    p = {"overview": "/", "tree-root": "/tree/HEAD", "log": "/log/HEAD", "refs": "/refs",
         "commit-largest": f"/commit/{probe['commit']}", "diff-fragment": f"/diff/{probe['commit']}/0",
         "search": f"/search?q={quote(search)}"}
    if probe.get("deep_tree"):
        p["tree-deep"] = f"/tree/HEAD/{quote(probe['deep_tree'])}"
    p["blob-largest"] = f"/blob/HEAD/{quote(probe['blob'])}"
    p["blame-long-history"] = f"/blame/HEAD/{quote(probe['blame'])}"
    for s in STATS:
        p["stats" + (s or "-overview")] = "/stats" + s
    return p


def measure_pages(port, plan, runs):
    out = {}
    for label, path in plan.items():
        status, ttfb, total, size = get(port, path)
        cold = {"ttfb_ms": ttfb, "total_ms": total}
        ttfbs, totals = [], []
        for _ in range(runs):
            _, a, b, _ = get(port, path)
            ttfbs.append(a)
            totals.append(b)
        out[label] = {"path": path, "status": status, "bytes": size, "cold": cold,
                      "warm_ttfb_ms": stats(ttfbs), "warm_total_ms": stats(totals)}
        print(f"  {label:<22} {status} cold {total:8.0f} ms  warm {statistics.median(totals):7.0f} ms", flush=True)
    return out


def idle_cpu(server, secs, with_events):
    stop = threading.Event()
    if with_events:
        def hold():
            try:
                c = http.client.HTTPConnection("127.0.0.1", server.port, timeout=5)
                c.request("GET", "/events")
                r = c.getresponse()
                while not stop.is_set():
                    try:
                        r.fp.readline()
                    except OSError:
                        pass
            except OSError:
                pass
        threading.Thread(target=hold, daemon=True).start()
        time.sleep(1)
    a, t = cpu_secs(server.pid), time.perf_counter()
    time.sleep(secs)
    b, t2 = cpu_secs(server.pid), time.perf_counter()
    stop.set()
    return 100 * (b - a) / (t2 - t)


def bench_repo(repo, repo_dir, binary, runs, idle_secs, logdir):
    name = repo["name"]
    print(f"== {name} ({repo['class']}) @ {repo['sha'][:10]}", flush=True)
    rev = repo["sha"] if not repo.get("local") else "HEAD"
    probe = discover(repo_dir, rev)
    probe.update({k: repo[k] for k in ("blob", "blame", "deep_tree", "commit") if k in repo})
    # The cache lives under the git dir: .git/gitterrace, or ./gitterrace in a bare clone.
    cache = Path(repo_dir) / (".git" if (Path(repo_dir) / ".git").exists() else "") / "gitterrace"
    shutil.rmtree(cache, ignore_errors=True)
    # One untimed read warms the OS cache so cold means the first request, not the disk.
    run(["git", "rev-list", "--count", rev], cwd=repo_dir)
    srv = Server(binary, repo_dir, logdir)
    res = {"sha": repo["sha"], "class": repo["class"], "probe": probe}
    try:
        secs, listen, peak = srv.wait_ready()
        res["index"] = {"secs": secs, "listen_secs": listen, "peak_rss_mb": peak}
        print(f"  index {secs:.1f} s, peak RSS {peak:.0f} MB", flush=True)
        rss_warm = rss_mb(srv.pid)
        res["pages"] = measure_pages(srv.port, pages(probe, repo.get("search", "fn")), runs)
        res["memory"] = {"rss_after_index_mb": rss_warm, "rss_after_pages_mb": rss_mb(srv.pid)}
        res["index"]["cache_mb"] = dir_mb(cache)
        res["idle"] = {"secs": idle_secs, "cpu_pct": idle_cpu(srv, idle_secs, False),
                       "cpu_pct_events_client": idle_cpu(srv, idle_secs, True)}
        print(f"  idle {res['idle']['cpu_pct']:.2f}% / with /events {res['idle']['cpu_pct_events_client']:.2f}%", flush=True)
    finally:
        srv.stop()
    if repo["class"] in ("small", "medium"):
        out = Path(tempfile.mkdtemp(prefix="gt-export-", dir=logdir))
        t = time.perf_counter()
        r = subprocess.run([binary, "export", str(out)], cwd=repo_dir, capture_output=True, text=True)
        res["export"] = {"secs": time.perf_counter() - t, "out_mb": dir_mb(out), "ok": r.returncode == 0}
        shutil.rmtree(out, ignore_errors=True)
    return res


def judge(res, budgets):
    """Budget status per metric class: pass or over."""
    b, out = budgets.get(res["class"], {}), {}
    def put(key, value, limit):
        if limit is not None:
            out[key] = {"value": value, "limit": limit, "status": "pass" if value <= limit else "over"}
    pg = res["pages"].values()
    put("page_warm_ms", max(p["warm_total_ms"]["median"] for p in pg), b.get("page_warm_ms"))
    put("page_cold_ms", max(p["cold"]["total_ms"] for p in pg), b.get("page_cold_ms"))
    put("index_secs", res["index"]["secs"], b.get("index_secs"))
    put("idle_cpu_pct", max(res["idle"]["cpu_pct"], res["idle"]["cpu_pct_events_client"]), b.get("idle_cpu_pct"))
    put("rss_mb", res["memory"]["rss_after_pages_mb"], b.get("rss_mb"))
    if "export" in res:
        put("export_secs", res["export"]["secs"], b.get("export_secs"))
    return out


def machine(binary):
    ram = 0
    try:
        ram = os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES") / 2**30
    except (ValueError, OSError):
        pass
    cpu = platform.processor()
    try:
        cpu = run(["sysctl", "-n", "machdep.cpu.brand_string"]).strip()
    except (OSError, subprocess.CalledProcessError):
        for line in Path("/proc/cpuinfo").read_text().splitlines() if Path("/proc/cpuinfo").exists() else []:
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    return {"host": platform.node().split(".")[0], "os": f"{platform.system()} {platform.release()}",
            "arch": platform.machine(), "cpu": cpu, "cpus": os.cpu_count(), "ram_gb": round(ram, 1),
            "git": run(["git", "--version"]).strip(), "gitterrace": run([binary, "--version"]).strip(),
            "gitterrace_sha": run(["git", "rev-parse", "HEAD"], cwd=ROOT).strip()}


def synthetic(base):
    d = Path(base) / "synthetic"
    run(["git", "init", "-q", str(d)])
    env = ["-c", "user.name=b", "-c", "user.email=b@example.com"]
    for i in range(30):
        f = d / "src" / "a" / "b" / f"f{i % 5}.rs"
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text("".join(f"fn item_{i}_{n}() {{}}\n" for n in range(i + 3)))
        run(["git", "add", "-A"], cwd=d)
        run(["git", *env, "commit", "-q", "-m", f"commit {i}"], cwd=d)
    sha = run(["git", "rev-parse", "HEAD"], cwd=d).strip()
    return {"name": "synthetic", "class": "small", "sha": sha, "local": str(d), "search": "item"}, d


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--repos", help="comma list of corpus names (default: all but opt-in)")
    ap.add_argument("--large", action="store_true", help="include opt-in repos (also BENCH_LARGE=1)")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--idle-secs", type=int, default=60)
    ap.add_argument("--bin", default=str(ROOT / "target/release/git-terrace"))
    ap.add_argument("--corpus", default=str(ROOT / "bench/corpus.toml"))
    ap.add_argument("--out-dir", default=str(ROOT / "bench/results"))
    ap.add_argument("--fetch-only", action="store_true", help="clone and backfill, measure nothing")
    ap.add_argument("--synthetic", action="store_true", help="tiny generated repo, one run (guard)")
    a = ap.parse_args()
    a.bin = str(Path(a.bin).resolve())
    if not Path(a.bin).exists():
        sys.exit(f"no binary at {a.bin}, run `make build` (release) first")
    corpus = parse_toml(Path(a.corpus).read_text())
    base = Path(os.environ.get("GITTERRACE_BENCH_DIR", Path.home() / ".cache/gitterrace-bench"))
    base.mkdir(parents=True, exist_ok=True)
    logdir = Path(tempfile.mkdtemp(prefix="gt-bench-"))
    todo = []
    if a.synthetic:
        a.runs, a.idle_secs = 1, 1
        repo, d = synthetic(logdir)
        todo = [(repo, d)]
    else:
        large = a.large or os.environ.get("BENCH_LARGE") == "1"
        want = a.repos.split(",") if a.repos else None
        for r in corpus["repo"]:
            if (want and r["name"] not in want) or (not want and r.get("opt_in") and not large):
                continue
            if r.get("opt_in") and not large:
                sys.exit(f"{r['name']} is opt-in: set BENCH_LARGE=1 (about {r.get('need_gb')} GB)")
            todo.append((r, ensure_clone(r, base, logdir)))
    if a.fetch_only:
        return 0
    result = {"when": datetime.datetime.now().isoformat(timespec="seconds"), "runs": a.runs,
              "idle_secs": a.idle_secs, "machine": machine(a.bin), "repos": {}}
    over = []
    for repo, d in todo:
        res = bench_repo(repo, d, a.bin, a.runs, a.idle_secs, logdir)
        res["budget"] = judge(res, corpus.get("budgets", {}))
        over += [f"{repo['name']}: {k} {v['value']:.1f} > {v['limit']}" for k, v in res["budget"].items() if v["status"] == "over"]
        result["repos"][repo["name"]] = res
    out_dir = Path(a.out_dir if not a.synthetic else logdir)
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"{datetime.date.today()}-{result['machine']['host']}.json"
    path.write_text(json.dumps(result, indent=1) + "\n")
    print(f"wrote {path}")
    if a.synthetic:
        shutil.rmtree(logdir, ignore_errors=True)
    for line in over:
        print("OVER BUDGET", line)
    return 1 if over and os.environ.get("BENCH_STRICT") == "1" else 0


sys.exit(main())
