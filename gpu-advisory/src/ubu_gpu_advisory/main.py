from __future__ import annotations

import json
import sys

from .noop import advise


def main() -> int:
    request = json.load(sys.stdin)
    response = advise(request)
    json.dump(response, sys.stdout, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
