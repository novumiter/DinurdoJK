"""Create a cumulative, verifiable source checkpoint from staged changes."""
from datetime import datetime
from argparse import ArgumentParser
from hashlib import sha256
from pathlib import Path
from subprocess import check_output, run
from zipfile import ZIP_DEFLATED, ZipFile


def main() -> None:
    parser = ArgumentParser(description=__doc__)
    parser.add_argument("--label", default="ocean-original-surface")
    args = parser.parse_args()
    if not args.label or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in args.label):
        parser.error("--label must contain only lowercase letters, digits and hyphens")
    root = Path(__file__).resolve().parents[1]
    out = root / "artifacts" / "blue-water"
    out.mkdir(parents=True, exist_ok=True)
    stem = args.label + "-" + datetime.now().strftime("%Y%m%d-%H%M%S")
    names = [
        path.decode("utf-8")
        for path in check_output(
            ["git", "diff", "--cached", "--name-only", "-z"], cwd=root
        ).split(b"\0")
        if path
    ]
    if not names or not all((root / name).is_file() for name in names):
        raise RuntimeError("Staged files are missing or not regular files")
    (out / f"{stem}-files.txt").write_text(
        "\n".join(names) + "\n", encoding="utf-8"
    )
    archive = out / f"{stem}.zip"
    with ZipFile(archive, "w", ZIP_DEFLATED, compresslevel=6) as bundle:
        for name in names:
            bundle.writestr(name, check_output(["git", "show", f":{name}"], cwd=root))
    with (out / f"{stem}.patch").open("wb") as patch:
        run(["git", "diff", "--cached", "--binary"], cwd=root, stdout=patch, check=True)
    digest = sha256(archive.read_bytes()).hexdigest()
    (out / f"{stem}.zip.sha256").write_text(
        f"{digest}  {archive.name}\n", encoding="ascii"
    )
    print(f"{stem}: {len(names)} files, SHA-256 {digest}")


if __name__ == "__main__":
    main()
