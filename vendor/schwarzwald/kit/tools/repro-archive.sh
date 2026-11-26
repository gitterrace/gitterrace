#!/usr/bin/env bash
# repro-archive.sh: build a .tar.gz whose bytes depend only on the file names and contents.
#
#   repro-archive.sh [--epoch N] [--base DIR] [--prefix DIR] -o OUT.tar.gz PATH...
#
# PATHs are files or directories (walked), relative to --base (default the cwd). The archive holds
# them under that relative name, below --prefix when given.
#   order    sorted by name, each name once
#   mtime    SOURCE_DATE_EPOCH, or --epoch N. One of them is required, the clock is never read.
#   owner    uid and gid 0, no user or group name
#   mode     files 0644, or 0755 when the owner can execute, directories 0755. Symlinks stay links.
#   gzip     no file name, the header time is the epoch, a fixed level
#   format   GNU tar, no extended headers
# It is python's tarfile and gzip, so macOS and Linux write the same bytes (the same zlib gives the
# same compressed stream, so compare archives made on one machine type).
# Exit: 0, 2 on a setup error with a remedy line.
set -euo pipefail

die() { # die <what failed> <remedy>
  echo "repro-archive: $1" >&2
  echo "repro-archive: remedy: $2" >&2
  exit 2
}

epoch=${SOURCE_DATE_EPOCH:-}
base=.
prefix=""
out=""
paths=()
while [ $# -gt 0 ]; do
  case $1 in
  --epoch) epoch=${2:?--epoch needs a number}; shift ;;
  --base) base=${2:?--base needs a directory}; shift ;;
  --prefix) prefix=${2:?--prefix needs a name}; shift ;;
  -o) out=${2:?-o needs a file}; shift ;;
  -h | --help) sed -n '2,18p' "${BASH_SOURCE[0]}"; exit 0 ;;
  -*) die "unknown option $1" "usage: repro-archive.sh [--epoch N] [--base DIR] [--prefix DIR] -o OUT PATH..." ;;
  *) paths+=("$1") ;;
  esac
  shift
done
[ -n "${epoch}" ] || die "no epoch" "set SOURCE_DATE_EPOCH (tools/repro-env.sh) or pass --epoch N"
case ${epoch} in *[!0-9]*) die "bad epoch '${epoch}'" "an epoch is whole seconds" ;; esac
[ -n "${out}" ] || die "no output" "pass -o OUT.tar.gz"
[ ${#paths[@]} -gt 0 ] || die "no files" "name the files or directories to pack"
[ -d "${base}" ] || die "no directory ${base}" "pass an existing --base"
command -v python3 >/dev/null 2>&1 || die "python3 is missing" "install python 3"
out_abs=$(cd "$(dirname "${out}")" && pwd -P)/$(basename "${out}")

cd "${base}"
python3 - "${epoch}" "${prefix}" "${out_abs}" "${paths[@]}" <<'PY'
import gzip, os, stat, sys, tarfile

epoch, prefix, out = int(sys.argv[1]), sys.argv[2].strip("/"), sys.argv[3]
names = set()
for p in sys.argv[4:]:
    p = os.path.normpath(p)
    if p.startswith("..") or os.path.isabs(p):
        print(f"repro-archive: {p} is outside --base", file=sys.stderr)
        print("repro-archive: remedy: name paths below --base", file=sys.stderr)
        sys.exit(2)
    if not os.path.lexists(p):
        print(f"repro-archive: no such file {p}", file=sys.stderr)
        print("repro-archive: remedy: build it first or fix the name", file=sys.stderr)
        sys.exit(2)
    names.add(p)
    if os.path.isdir(p) and not os.path.islink(p):
        for d, dirs, files in os.walk(p):
            for n in dirs + files:
                names.add(os.path.join(d, n))

with open(out, "wb") as raw:
    with gzip.GzipFile(filename="", mode="wb", fileobj=raw, compresslevel=9, mtime=epoch) as gz:  # pin:gzip-mtime
        with tarfile.open(fileobj=gz, mode="w", format=tarfile.GNU_FORMAT) as tar:
            for name in sorted(names):
                st = os.lstat(name)
                ti = tarfile.TarInfo(os.path.join(prefix, name) if prefix else name)
                ti.mtime = epoch  # pin:mtime
                ti.uid = ti.gid = 0
                ti.uname = ti.gname = ""
                if stat.S_ISLNK(st.st_mode):
                    ti.type, ti.linkname, ti.mode = tarfile.SYMTYPE, os.readlink(name), 0o777
                    tar.addfile(ti)
                elif stat.S_ISDIR(st.st_mode):
                    ti.type, ti.mode = tarfile.DIRTYPE, 0o755
                    tar.addfile(ti)
                else:
                    ti.size = st.st_size
                    ti.mode = 0o755 if st.st_mode & stat.S_IXUSR else 0o644
                    with open(name, "rb") as fh:
                        tar.addfile(ti, fh)
PY
