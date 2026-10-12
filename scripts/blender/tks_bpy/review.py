"""Image review, comparison, and metrics against references."""

from pathlib import Path

import bpy
import numpy as np


def stats(path, grid=3):
    """Compute image statistics using display-referred pixel data.

    Returns dict with keys:
    - mean_luma, p01, p99, clip_pct, mean_sat, mean_rgb
    - cells: grid x grid nested list of mean luma values, where row 0 is the top of the image.
    """
    img = bpy.data.images.load(str(path), check_existing=False)
    try:
        w, h = img.size[0], img.size[1]
        channels = img.channels
        raw = np.empty(w * h * channels, dtype=np.float32)
        img.pixels.foreach_get(raw)
        arr = raw.reshape((h, w, channels))
        # Blender stores row 0 at the bottom; flip vertically so row 0 is the top.
        top_down = np.flipud(arr)
        rgb = top_down[:, :, :3]

        luma = 0.2126 * rgb[:, :, 0] + 0.7152 * rgb[:, :, 1] + 0.0722 * rgb[:, :, 2]
        mean_luma = float(np.mean(luma))
        p01 = float(np.percentile(luma, 1))
        p99 = float(np.percentile(luma, 99))

        clipped_mask = np.any(rgb >= 0.995, axis=-1)
        clip_pct = float(100.0 * np.count_nonzero(clipped_mask) / clipped_mask.size)

        cmax = np.max(rgb, axis=-1)
        cmin = np.min(rgb, axis=-1)
        sat = np.where(cmax > 1e-6, (cmax - cmin) / cmax, 0.0)
        mean_sat = float(np.mean(sat))
        mean_rgb = [float(np.mean(rgb[:, :, c])) for c in range(3)]

        cells = []
        r_step = h / grid
        c_step = w / grid
        for r in range(grid):
            row_vals = []
            r_start = int(round(r * r_step))
            r_end = int(round((r + 1) * r_step))
            for c in range(grid):
                c_start = int(round(c * c_step))
                c_end = int(round((c + 1) * c_step))
                sub = luma[r_start:r_end, c_start:c_end]
                row_vals.append(round(float(np.mean(sub)), 4))
            cells.append(row_vals)

        return {
            "mean_luma": round(mean_luma, 4),
            "p01": round(p01, 4),
            "p99": round(p99, 4),
            "clip_pct": round(clip_pct, 2),
            "mean_sat": round(mean_sat, 4),
            "mean_rgb": [round(v, 4) for v in mean_rgb],
            "cells": cells,
        }
    finally:
        bpy.data.images.remove(img)


def compare(reference, render, out=None, grid=3, height=540):
    """Write side-by-side composite (<render>_vs_ref.png) and return comparison metrics.

    Reference is placed on the left, render on the right, both scaled to the same height.
    """
    ref_stats = stats(reference, grid=grid)
    rnd_stats = stats(render, grid=grid)

    cell_luma_ratio = []
    for r in range(grid):
        row_ratios = []
        for c in range(grid):
            denom = max(1e-4, ref_stats["cells"][r][c])
            row_ratios.append(round(rnd_stats["cells"][r][c] / denom, 3))
        cell_luma_ratio.append(row_ratios)

    sat_delta = round(rnd_stats["mean_sat"] - ref_stats["mean_sat"], 4)
    clip_delta = round(rnd_stats["clip_pct"] - ref_stats["clip_pct"], 2)

    if out is None:
        rnd_path = Path(render)
        out = rnd_path.parent / f"{rnd_path.stem}_vs_ref.png"
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)

    ref_img = bpy.data.images.load(str(reference), check_existing=False)
    rnd_img = bpy.data.images.load(str(render), check_existing=False)
    comp_img = None
    try:
        ref_w = max(2, int(round(ref_img.size[0] * height / max(1, ref_img.size[1]))))
        rnd_w = max(2, int(round(rnd_img.size[0] * height / max(1, rnd_img.size[1]))))
        ref_img.scale(ref_w, height)
        rnd_img.scale(rnd_w, height)

        ref_raw = np.empty(ref_w * height * 4, dtype=np.float32)
        ref_img.pixels.foreach_get(ref_raw)
        ref_arr = ref_raw.reshape((height, ref_w, 4))

        rnd_raw = np.empty(rnd_w * height * 4, dtype=np.float32)
        rnd_img.pixels.foreach_get(rnd_raw)
        rnd_arr = rnd_raw.reshape((height, rnd_w, 4))

        composed = np.concatenate([ref_arr, rnd_arr], axis=1)
        total_w = ref_w + rnd_w

        comp_img = bpy.data.images.new("comp_temp", width=total_w, height=height, alpha=True)
        comp_img.pixels.foreach_set(composed.ravel())
        comp_img.filepath_raw = str(out)
        comp_img.file_format = "PNG"
        comp_img.save()
    finally:
        bpy.data.images.remove(ref_img)
        bpy.data.images.remove(rnd_img)
        if comp_img is not None:
            bpy.data.images.remove(comp_img)

    return {
        "ref": ref_stats,
        "render": rnd_stats,
        "cell_luma_ratio": cell_luma_ratio,
        "sat_delta": sat_delta,
        "clip_delta": clip_delta,
        "path": str(out),
    }
