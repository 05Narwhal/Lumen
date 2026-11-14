"""Training configuration: a TOML file, then `section.key=value` overrides from the command
line. Values in overrides are read as TOML (`3e-4`, `true`, `"text"`), bare words as text."""

import tomllib
from pathlib import Path
from typing import Any


def _value(text: str) -> Any:
    try:
        return tomllib.loads(f"v = {text}")["v"]
    except tomllib.TOMLDecodeError:
        return text


def load(path: Path | str, overrides: list[str] | None = None) -> dict:
    with open(path, "rb") as f:
        config = tomllib.load(f)
    for item in overrides or []:
        if "=" not in item:
            raise ValueError(f"an override is section.key=value, not {item!r}")
        key, text = item.split("=", 1)
        *sections, last = key.strip().split(".")
        node = config
        for s in sections:
            node = node.setdefault(s, {})
        node[last] = _value(text.strip())
    return config
