"""Training logs: `log.csv` in the run folder (rewritten when a new metric appears; a resumed
run carries on from the rows already there), and TensorBoard when it is installed."""

import csv
from pathlib import Path


class Logger:
    def __init__(self, run_dir: Path | str, tensorboard: bool = True):
        self.dir = Path(run_dir)
        self.dir.mkdir(parents=True, exist_ok=True)
        self.rows: list[dict] = []
        self.keys: list[str] = []
        path = self.dir / "log.csv"
        if path.exists():
            with open(path, newline="", encoding="utf-8") as f:
                r = csv.DictReader(f)
                self.keys = sorted(k for k in (r.fieldnames or []) if k != "step")
                self.rows = [{k: v for k, v in row.items() if v != ""} for row in r]
        self.tb = None
        if tensorboard:
            try:
                from torch.utils.tensorboard import SummaryWriter

                self.tb = SummaryWriter(str(self.dir / "tb"))
            except ImportError:
                self.tb = None

    def log(self, step: int, **metrics: float) -> None:
        self.rows.append({"step": step, **metrics})
        new = sorted(set(self.keys) | set(metrics))
        rewrite = new != self.keys
        self.keys = new
        path = self.dir / "log.csv"
        fields = ["step", *self.keys]
        if rewrite or not path.exists():
            with open(path, "w", newline="", encoding="utf-8") as f:
                w = csv.DictWriter(f, fields)
                w.writeheader()
                w.writerows(self.rows)
        else:
            with open(path, "a", newline="", encoding="utf-8") as f:
                csv.DictWriter(f, fields).writerow(self.rows[-1])
        if self.tb is not None:
            for k, v in metrics.items():
                self.tb.add_scalar(k, v, step)

    def close(self) -> None:
        if self.tb is not None:
            self.tb.close()
