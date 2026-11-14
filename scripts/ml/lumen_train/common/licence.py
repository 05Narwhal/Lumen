"""The licence gate: every source a model learns from is in `datasets/registry.toml`. A model
meant to ship with `terms="open"` (the default, weights under MIT) learns only from sources that
allow commercial use. A model that learned from a non-commercial source may still ship, but only as
`terms="nc"` weights under CC-BY-NC-4.0, never as open ones."""

import tomllib
from pathlib import Path
from typing import Literal

REGISTRY = Path(__file__).resolve().parent.parent / "datasets" / "registry.toml"


class LicenceError(RuntimeError):
    pass


def load_registry(path: Path | str = REGISTRY) -> dict:
    with open(path, "rb") as f:
        return tomllib.load(f)["sources"]


def weights_licence(terms: Literal["open", "nc"]) -> str:
    """The licence the trained weights are published under."""
    return "CC-BY-NC-4.0" if terms == "nc" else "MIT"


def check(sources: list[str], ship: bool, registry: dict | None = None, terms: Literal["open", "nc"] = "open") -> None:
    reg = load_registry() if registry is None else registry
    unknown = [s for s in sources if s not in reg]
    if unknown:
        raise LicenceError(f"not in the registry (vet it first, docs/THIRD-PARTY-MODELS.md): {', '.join(unknown)}")
    if ship and terms != "nc":
        refused = [s for s in sources if not reg[s].get("commercial", False)]
        if refused:
            raise LicenceError(f"a model that ships as open weights cannot learn from non-commercial sources (ship it as terms=nc, CC-BY-NC-4.0): {', '.join(refused)}")
