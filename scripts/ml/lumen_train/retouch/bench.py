"""Where a training step spends its time.

    python -m lumen_train.retouch.bench data.root=C:/lumen-train [steps=30] [trace=C:/lumen-train/runs/bench/trace.json]

Runs `steps` training steps of the configured run (the same data, model and loss as
`train`) under `torch.profiler`, and prints the data time against the step time, the top
ops by self CUDA and self CPU time, kernel launches per step and peak VRAM. A Chrome trace
(chrome://tracing, or Perfetto) goes to `trace`, by default `<run.dir>/bench-trace.json`."""

import argparse
import sys
import time
from pathlib import Path

import torch
from torch.profiler import ProfilerActivity, profile

from ..common import config as config_mod
from . import train

TOP = 12


def _sync(device: torch.device) -> None:
    if device.type == "cuda":
        torch.cuda.synchronize(device)


def run(cfg: dict, steps: int = 30, trace: Path | None = None) -> dict:
    b = train._build(cfg)
    metric = train.train_metric(cfg["loss"], 0, b.epochs)
    b.train_ds.set_epoch(0)
    b.model.train()
    cuda = b.device.type == "cuda"
    if cuda:
        torch.cuda.reset_peak_memory_stats(b.device)

    def batches():
        while True:
            yield from b.loader

    it = batches()
    # One untimed step first: cuDNN autotuning and allocator growth are not the steady state.
    loss, _ = train._forward(b, next(it), cfg["loss"], metric)
    train._update(b, loss)
    _sync(b.device)

    # Data loading is timed but not profiled: the profiler keeps a record of every op, and the
    # CPU synthesis of a 64-photo batch is millions of them (one profile of 30 steps with it
    # grew past 30 GB). Each step gets its own profile; the last one is the trace.
    data_s = step_s = 0.0
    cpu, dev, kernels = {}, {}, 0
    activities = [ProfilerActivity.CPU] + ([ProfilerActivity.CUDA] if cuda else [])
    for _ in range(steps):
        t0 = time.perf_counter()
        batch = next(it)
        t1 = time.perf_counter()
        with profile(activities=activities, record_shapes=False) as prof:
            loss, _ = train._forward(b, batch, cfg["loss"], metric)
            train._update(b, loss)
            _sync(b.device)
        step_s += time.perf_counter() - t1
        data_s += t1 - t0
        for e in prof.key_averages():
            cpu[e.key] = cpu.get(e.key, 0.0) + e.self_cpu_time_total / 1e6
            dev[e.key] = dev.get(e.key, 0.0) + getattr(e, "self_device_time_total", 0) / 1e6
        if cuda:
            kernels += sum(1 for e in prof.events() if e.device_type == torch.autograd.DeviceType.CUDA)

    top_cpu = sorted(cpu.items(), key=lambda x: -x[1])[:TOP]
    top_cuda = sorted(dev.items(), key=lambda x: -x[1])[:TOP] if cuda else []
    if trace is None:
        trace = Path(cfg["run"]["dir"]) / "bench-trace.json"
    trace.parent.mkdir(parents=True, exist_ok=True)
    prof.export_chrome_trace(str(trace))
    return {
        "steps": steps,
        "data_s": data_s / steps,
        "step_s": step_s / steps,
        "top_cuda": [(n, s / steps) for n, s in top_cuda],
        "top_cpu": [(n, s / steps) for n, s in top_cpu],
        "kernels_per_step": kernels / steps,
        "peak_vram_gb": torch.cuda.max_memory_allocated(b.device) / 1024 ** 3 if cuda else 0.0,
        "batch": int(cfg["train"]["batch"]),
        "device": str(b.device),
        "trace": str(trace),
    }


def table(r: dict) -> str:
    """The bench result as a Markdown table (pasted into plans' As built)."""
    lines = [
        f"{r['steps']} steps, batch {r['batch']} on {r['device']}: data {r['data_s'] * 1e3:.1f} ms/step, "
        f"step {r['step_s'] * 1e3:.1f} ms/step, {r['kernels_per_step']:.0f} CUDA kernels/step, peak VRAM {r['peak_vram_gb']:.2f} GB",
        "",
    ]
    for title, rows in (("self CUDA", r["top_cuda"]), ("self CPU", r["top_cpu"])):
        if not rows:
            continue
        lines += [f"| op (by {title}) | ms/step |", "|---|---:|"]
        lines += [f"| `{n[:60]}` | {s * 1e3:.2f} |" for n, s in rows]
        lines.append("")
    return "\n".join(lines)


def main() -> None:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--config", type=Path, default=train.DEFAULT_CONFIG)
    ap.add_argument("overrides", nargs="*", help="section.key=value, plus steps=N and trace=PATH")
    a = ap.parse_args()
    steps, trace, rest = 30, None, []
    for o in a.overrides:
        if o.startswith("steps="):
            steps = int(o.split("=", 1)[1])
        elif o.startswith("trace="):
            trace = Path(o.split("=", 1)[1])
        else:
            rest.append(o)
    cfg = config_mod.load(a.config, rest)
    print(table(run(cfg, steps=steps, trace=trace)))


if __name__ == "__main__":
    main()
