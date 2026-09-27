"""Local frame cleanup. ONNX Runtime runs U²-Net without importing rembg's CLI stack.

Model/preprocessing: https://github.com/xuebinqin/U-2-Net
Compatible with the u2net.onnx / u2netp.onnx models used by rembg.
Never downloads a model or sends images to a server.
"""
import argparse
from collections import Counter, deque
from pathlib import Path

import numpy as np
from PIL import Image


def border_alpha(image, tolerance):
    rgb = np.asarray(image.convert("RGB"), dtype=np.int16)
    edge = np.concatenate((rgb[0], rgb[-1], rgb[:, 0], rgb[:, -1]))
    bins = (edge // 16).astype(np.uint8)
    dominant = Counter(map(tuple, bins)).most_common(1)[0][0]
    selected = np.all(bins == dominant, axis=1)
    color = np.median(edge[selected], axis=0)
    match = np.max(np.abs(rgb - color), axis=2) <= tolerance
    h, w = match.shape
    removed = np.zeros((h, w), dtype=bool)
    queue = deque()

    def visit(y, x):
        if 0 <= y < h and 0 <= x < w and match[y, x] and not removed[y, x]:
            removed[y, x] = True
            queue.append((y, x))

    for x in range(w):
        visit(0, x)
        visit(h - 1, x)
    for y in range(h):
        visit(y, 0)
        visit(y, w - 1)
    while queue:
        y, x = queue.popleft()
        visit(y - 1, x)
        visit(y + 1, x)
        visit(y, x - 1)
        visit(y, x + 1)
    return Image.fromarray(np.where(removed, 0, 255).astype(np.uint8))


class Foreground:
    def __init__(self, model):
        import onnxruntime as ort
        options = ort.SessionOptions()
        options.intra_op_num_threads = 2
        options.inter_op_num_threads = 1
        options.add_session_config_entry("session.intra_op.allow_spinning", "0")
        options.add_session_config_entry("session.inter_op.allow_spinning", "0")
        self.session = ort.InferenceSession(str(model), sess_options=options, providers=["CPUExecutionProvider"])
        self.input_name = self.session.get_inputs()[0].name

    def alpha(self, image):
        pixels = np.asarray(image.convert("RGB").resize((320, 320), Image.Resampling.LANCZOS), dtype=np.float32)
        pixels /= max(float(pixels.max()), 1e-6)
        pixels = (pixels - np.array([.485, .456, .406], dtype=np.float32)) / np.array([.229, .224, .225], dtype=np.float32)
        tensor = np.transpose(pixels, (2, 0, 1))[None].copy()
        prediction = self.session.run(None, {self.input_name: tensor})[0][0, 0]
        minimum, maximum = float(prediction.min()), float(prediction.max())
        if maximum - minimum < 1e-7:
            prediction = np.zeros_like(prediction)
        else:
            prediction = (prediction - minimum) / (maximum - minimum)
        mask = Image.fromarray((np.clip(prediction, 0, 1) * 255).astype(np.uint8))
        return mask.resize(image.size, Image.Resampling.LANCZOS)


def process(args):
    files = sorted(args.folder.glob("frame_*.png"))
    foreground = Foreground(args.model) if args.mode == "ai" else None
    kept = 0
    for index, path in enumerate(files):
        with Image.open(path) as opened:
            image = opened.convert("RGBA")
        rgb = np.asarray(image.convert("RGB"))
        uniform = all(int(rgb[:, :, c].max()) - int(rgb[:, :, c].min()) <= 2 for c in range(3))
        if args.skip_blank and uniform:
            path.unlink()
        else:
            if args.mode == "border":
                alpha = border_alpha(image, args.tolerance)
            elif foreground:
                alpha = foreground.alpha(image)
            else:
                alpha = Image.new("L", image.size, 255)
            original = np.asarray(image.getchannel("A"), dtype=np.uint16)
            mask = (np.asarray(alpha, dtype=np.uint16) * original // 255).astype(np.uint8)
            image.putalpha(Image.fromarray(mask))
            box = image.getchannel("A").getbbox()
            if box is None:
                path.unlink()
            else:
                if args.crop:
                    image = image.crop(box)
                # Atomic replacement prevents a cancelled write becoming a broken PNG.
                temporary = path.with_suffix(".work")
                image.save(temporary, format="PNG")
                temporary.replace(path)
                kept += 1
        print(f"PROGRESS\t{index + 1}\t{len(files)}\t{kept}", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("folder", type=Path)
    parser.add_argument("--mode", choices=["none", "border", "ai"], default="none")
    parser.add_argument("--model", type=Path)
    parser.add_argument("--tolerance", type=int, default=40)
    parser.add_argument("--crop", action="store_true")
    parser.add_argument("--skip-blank", action="store_true")
    options = parser.parse_args()
    try:
        process(options)
    except Exception as error:
        print(f"ERROR: {type(error).__name__}: {error}", flush=True)
        raise SystemExit(1)
