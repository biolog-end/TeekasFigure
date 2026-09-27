import importlib.util
import json
import re
import tempfile
import unittest
from pathlib import Path


SPEC = importlib.util.spec_from_file_location("minecraft_datapack", Path(__file__).resolve().parents[1] / "scripts/minecraft_datapack.py")
mc = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mc)


def shape(ident, index=0, x=16, rotation=0, opacity=1):
    return {"id": ident, "brush_index": index, "center_px": [x, 16],
            "size_px": [16, 16], "rotation_radians": rotation, "opacity": opacity,
            "original_colors": True, "hue_turns": 0, "saturation": 1, "brightness": 1}


class DatapackTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.scene = Path(self.tmp.name) / "Мобы"
        self.scene.mkdir()
        self.output = Path(self.tmp.name) / "pack"
        self.frames = [[shape(42), shape(7)], [shape(7, x=32)], [shape(7, x=32), shape(99, 1)]]
        self.mapping = [{"brush_index": 0, "source_name": "крипер.png", "entity_type": "minecraft:creeper",
                         "reference_size_blocks": [1, 1], "depth_blocks": 2,
                         "yaw_offset_degrees": 90, "pivot_blocks": [0, 0, 0]},
                        {"brush_index": 1, "source_name": "rabbit.png", "entity_type": "minecraft:rabbit",
                         "reference_size_blocks": [1, 1], "depth_blocks": 0.5}]
        self.save()

    def save(self, fps=20, complete=True, is_video=True):
        (self.scene / "scene.json").write_text(json.dumps({"schema_version": 1, "complete": complete,
            "frame_count": len(self.frames), "fps": fps, "is_video": is_video}), encoding="utf-8")
        (self.scene / "frames.jsonl").write_text("".join(json.dumps({"frame_index": i,
            "time_seconds": i / fps, "shapes": shapes}) + "\n" for i, shapes in enumerate(self.frames)), encoding="utf-8")
        (self.scene / "minecraft_mapping.json").write_text(json.dumps(self.mapping, ensure_ascii=False), encoding="utf-8")

    def build(self, *flags):
        args = mc.parser().parse_args([str(self.scene), "--output", str(self.output), *flags])
        output, namespace = mc.generate(args.scene, args.output, args)
        self.functions = output / "data" / namespace / "function"
        self.namespace = namespace
        return output

    def commands(self, index):
        return (self.functions / "frames" / f"{index:06}.mcfunction").read_text(encoding="utf-8")

    def test_survivors_reuse_entities_and_new_layers_clear_existing_models(self):
        output = self.build()
        frame0, frame1, frame2 = [self.commands(i) for i in range(3)]
        self.assertIn(f'tag={self.namespace}_7,limit=1]', frame1)
        self.assertIn(f'execute unless entity @e[type=minecraft:creeper', frame1)
        self.assertNotIn(f'kill @e[tag={self.namespace}_mob,tag={self.namespace}_7]', frame1)
        self.assertIn(f'kill @e[tag={self.namespace}_mob,tag={self.namespace}_42]', frame1)
        self.assertIn('~1 ~2.05 ~1 90 0', frame0)
        # Rabbit is a short new mob: its FEET are placed above the old
        # creeper's full height, so it cannot be hidden behind it from above.
        self.assertIn('~1 ~2.05 ~1 0 0', frame2)
        self.assertIn('NoAI:1b,NoGravity:1b', frame0)
        self.assertIn('collisionRule never', (self.functions / 'start.mcfunction').read_text())
        stop = (self.functions / 'stop.mcfunction').read_text()
        self.assertNotRegex(stop, r'kill @e(?:\s|$)')
        self.assertNotIn('gamerule', frame0 + stop)
        self.assertNotIn('forceload', frame0 + stop)
        self.assertEqual(json.loads((output / 'pack.mcmeta').read_text())['pack']['pack_format'], 48)

    def test_binary_dispatch_selects_each_frame_once_at_the_correct_tick(self):
        self.save(fps=5)
        self.build()
        root_line = (self.functions / 'step.mcfunction').read_text().splitlines()[0]
        root = root_line.split(':', 1)[1]

        def dispatch(name, tick):
            leaves = []
            for line in (self.functions / (name + '.mcfunction')).read_text().splitlines():
                match = re.search(r'matches (\d+)\.\.(\d+) run function \w+:(.+)', line)
                if match:
                    if int(match[1]) <= tick <= int(match[2]):
                        leaves.extend(dispatch(match[3], tick))
                else:
                    leaves.append(int(line.rsplit('/', 1)[1]))
            return leaves

        for tick in range(12):
            self.assertEqual(dispatch(root, tick), [tick])
            expected_x = 2 if tick >= 4 else 1
            if tick % 4 == 0:
                self.assertIn(f'~{expected_x} ~', self.commands(tick))
            else:
                self.assertEqual(self.commands(tick).strip(), '')
        self.assertIn('matches ..11', (self.functions / 'step.mcfunction').read_text())

    def test_side_camera_rejects_sprite_rotation_and_uses_depth(self):
        self.build('--view', 'side')
        self.assertIn('~1 ~-1 ~-1 90 0', self.commands(0))
        self.assertIn('~1 ~-1 ~-3.05 90 0', self.commands(0))
        self.output = self.output.with_name('bad_rotation')
        self.frames[0][0]['rotation_radians'] = 0.1
        self.save()
        with self.assertRaisesRegex(ValueError, 'Side views require rotation'):
            self.build('--view', 'side')
        self.assertFalse(self.output.exists())

    def test_invalid_dropped_frame_is_still_rejected_without_partial_pack(self):
        self.frames[1][0]['opacity'] = 0.5
        self.save(fps=40)
        with self.assertRaisesRegex(ValueError, 'original colors'):
            self.build()
        self.assertFalse(self.output.exists())

    def test_unmapped_mob_and_command_injection_are_rejected(self):
        for entity in [None, 'minecraft:creeper\nkill @e', 'minecraft:item_display', 'minecraft:boat']:
            self.mapping[0]['entity_type'] = entity
            self.save()
            with self.assertRaisesRegex(ValueError, 'Fill entity_type'):
                self.build()
            self.assertFalse(self.output.exists())

    def test_partial_export_requires_explicit_option_and_existing_pack_is_preserved(self):
        self.save(complete=False)
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            self.build()
        self.build('--allow-partial')
        sentinel = self.output / 'user.txt'
        sentinel.write_text('preserve')
        with self.assertRaisesRegex(ValueError, 'already exists'):
            self.build('--allow-partial')
        self.assertEqual(sentinel.read_text(), 'preserve')

    def test_replay_budget_can_be_explicitly_increased(self):
        with self.assertRaisesRegex(ValueError, 'Default replay budget'):
            self.build('--max-mobs', '1')
        self.build('--max-mobs', '2')
        self.assertTrue(self.output.is_dir())

    def test_static_image_is_shown_once_and_nonuniform_scaling_is_rejected(self):
        self.frames = [self.frames[0]]
        self.save(is_video=False)
        self.build()
        self.assertEqual(len(list((self.functions / 'frames').iterdir())), 1)
        self.assertIn('matches ..0', (self.functions / 'step.mcfunction').read_text())
        self.output = self.output.with_name('bad_scale')
        self.frames[0][0]['size_px'] = [16, 32]
        self.save(is_video=False)
        with self.assertRaisesRegex(ValueError, 'different scales'):
            self.build()
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
