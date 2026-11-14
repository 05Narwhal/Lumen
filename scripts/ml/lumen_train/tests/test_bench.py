"""The bench: where a training step spends its time, on a few synthetic photos on the CPU."""

from pathlib import Path

from lumen_train.retouch import bench
from lumen_train.tests.test_train import config


def test_bench_reports_where_time_goes(tmp_path: Path):
    from lumen_train.tests.test_dataset import write_sample

    for i in range(12):
        write_sample(tmp_path / "data" / "synthetic-test" / f"p{i:02d}", 96, 64, 0.04 + 0.02 * i)
    cfg = config(tmp_path / "data", tmp_path / "run", ["synthetic-test"])
    r = bench.run(cfg, steps=3, trace=tmp_path / "trace.json")
    assert r["steps"] == 3 and r["step_s"] > 0 and r["data_s"] >= 0
    assert r["top_cpu"] and all(isinstance(n, str) for n, _ in r["top_cpu"])
    assert r["kernels_per_step"] >= 0 and r["peak_vram_gb"] >= 0
    assert (tmp_path / "trace.json").exists()
