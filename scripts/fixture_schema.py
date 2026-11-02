"""Read the exact target source schema; never infer it from an installed DB."""
from pathlib import Path
import re
import sys


def source_schema(root: Path) -> int:
    for relative in ("src-tauri/src/store/mod.rs", "src-tauri/src/store.rs"):
        path = root / relative
        if path.is_file():
            values = re.findall(r"(?m)^pub const SCHEMA_VERSION:\s*u32\s*=\s*([0-9]+)\s*;", path.read_text(encoding="utf-8"))
            if len(values) != 1 or int(values[0]) < 1:
                raise ValueError("The target source must define one positive SCHEMA_VERSION")
            return int(values[0])
    raise ValueError("Target Store source is missing")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Usage: fixture_schema.py <checked-out-source-root>")
    print(source_schema(Path(sys.argv[1]).resolve(strict=True)))
