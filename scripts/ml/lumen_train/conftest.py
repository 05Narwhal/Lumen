import sys
from pathlib import Path

# `python -m pytest lumen_train` from scripts/ml, or `pytest` from anywhere: the package
# imports as `lumen_train`.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
