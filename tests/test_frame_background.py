import importlib.util
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace

import numpy as np
from PIL import Image

spec = importlib.util.spec_from_file_location("frame_background", Path(__file__).parents[1] / "scripts/frame_background.py")
background = importlib.util.module_from_spec(spec)
spec.loader.exec_module(background)


class BackgroundTests(unittest.TestCase):
    def test_border_flood_preserves_an_enclosed_region_of_the_same_color(self):
        pixels = np.full((9, 9, 3), 255, dtype=np.uint8)
        pixels[2:7, 2:7] = 0
        pixels[3:6, 3:6] = 255
        mask = background.border_alpha(Image.fromarray(pixels), 20)
        self.assertEqual(mask.getpixel((0, 0)), 0)
        self.assertEqual(mask.getpixel((4, 4)), 255)
        self.assertEqual(mask.getpixel((2, 2)), 255)

    def test_cleanup_skips_blank_frames_and_crops_objects(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            Image.new("RGB", (20, 20), "white").save(root / "frame_000001.png")
            image = Image.new("RGB", (20, 20), "white")
            image.paste("red", (5, 6, 15, 16))
            image.save(root / "frame_000002.png")
            background.process(SimpleNamespace(folder=root, mode="border", tolerance=20, skip_blank=True, crop=True))
            self.assertFalse((root / "frame_000001.png").exists())
            with Image.open(root / "frame_000002.png") as result:
                self.assertEqual(result.size, (10, 10))
                self.assertEqual(result.getchannel("A").getextrema(), (255, 255))


if __name__ == "__main__":
    unittest.main()
