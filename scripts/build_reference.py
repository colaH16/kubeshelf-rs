#!/usr/bin/env python3
"""Build the original Go implementation as a differential-test oracle.

The original repository is only *read*: the pinned commit is exported with
`git archive` into a fresh temporary directory, the two adapter files from
tests/go-reference/ are added there, and `go build` runs in that copy.

Usage:
  python3 scripts/build_reference.py --source PATH_TO_ORIGINAL_CHECKOUT --output BIN

The source may be any clone (local or `actions/checkout`) that contains the
pinned commit; uncommitted changes in it are ignored because only the commit
object is exported.
"""
import argparse
import os
import pathlib
import shutil
import subprocess
import sys
import tarfile
import tempfile

PINNED_COMMIT = "957bdd081db04ea2a205672e5f8e4d968c5ed193"
ROOT = pathlib.Path(__file__).resolve().parent.parent
ADAPTERS = {
    "export_reference.go": "internal/shelf/export_reference.go",
    "main.go": "cmd/reference/main.go",
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--source", default=os.environ.get("KUBESHELF_GO_SOURCE"), help="original KubeShelf clone")
    parser.add_argument("--commit", default=PINNED_COMMIT)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    if not args.source:
        parser.error("--source or KUBESHELF_GO_SOURCE is required")
    source = pathlib.Path(args.source).resolve()
    output = pathlib.Path(args.output).resolve()

    resolved = subprocess.run(
        ["git", "-C", str(source), "rev-parse", "--verify", f"{args.commit}^{{commit}}"],
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    if resolved != args.commit:
        print(f"commit mismatch: {resolved} != {args.commit}", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory(prefix="kubeshelf-go-reference-") as tmp:
        work = pathlib.Path(tmp) / "src"
        work.mkdir()
        archive = pathlib.Path(tmp) / "source.tar"
        with archive.open("wb") as handle:
            subprocess.run(["git", "-C", str(source), "archive", "--format=tar", args.commit], check=True, stdout=handle)
        with tarfile.open(archive) as tar:
            tar.extractall(work, filter="data")
        for name, target in ADAPTERS.items():
            destination = work / target
            if destination.exists():
                print(f"refusing to overwrite original file {target}", file=sys.stderr)
                return 1
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / "tests" / "go-reference" / name, destination)
        output.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["go", "build", "-trimpath", "-o", str(output), "./cmd/reference"], cwd=work, check=True)
    print(f"built Go reference from {args.commit} -> {output.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
