"""Build a Java 1.21/1.21.1 datapack from a TeekasFigure scene.

Standard library only. Reads frames as a stream; never sends images anywhere.
Living entities only: no texture displays, wool recoloring or AI-generated commands.
"""
import argparse
import json
import math
import re
import shutil
import tempfile
import uuid
from pathlib import Path


RESOURCE = re.compile(r"[a-z0-9_.-]+:[a-z0-9_./-]+\Z")
MOBS = {"minecraft:" + name for name in """allay armadillo axolotl bat bee blaze
bogged breeze camel cat cave_spider chicken cod cow creeper dolphin donkey drowned
elder_guardian enderman endermite evoker fox frog ghast giant glow_squid goat guardian
hoglin horse husk illusioner iron_golem llama magma_cube mooshroom mule ocelot panda
parrot phantom pig piglin piglin_brute pillager polar_bear pufferfish rabbit ravager
salmon sheep shulker silverfish skeleton skeleton_horse slime snow_golem spider squid
stray strider tadpole trader_llama tropical_fish turtle vex villager vindicator
wandering_trader warden witch wither wither_skeleton wolf zoglin zombie zombie_horse
zombie_villager""".split()}


def number(value, label, positive=False):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError(f"{label}: expected a finite number")
    if positive and value <= 0:
        raise ValueError(f"{label}: expected a positive number")
    return float(value)


def vector(value, length, label, positive=False):
    if not isinstance(value, list) or len(value) != length:
        raise ValueError(f"{label}: expected {length} numbers")
    return [number(v, label, positive) for v in value]


def integer(value, label):
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"{label}: expected a non-negative integer")
    return value


def read_mapping(path):
    rows = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(rows, list):
        raise ValueError("minecraft_mapping.json must contain a list")
    result = {}
    for row in rows:
        index = integer(row["brush_index"], "brush_index")
        if index in result:
            raise ValueError(f"Duplicate brush_index {index}")
        result[index] = row
    return result


def read_frames(scene, manifest):
    count = 0
    fps = number(manifest["fps"], "fps", True)
    with (scene / "frames.jsonl").open(encoding="utf-8") as stream:
        while True:
            # Bound a single line's allocation even for malformed input.
            line = stream.readline(64 * 1024 * 1024 + 1)
            if not line:
                break
            if len(line) > 64 * 1024 * 1024 or not line.endswith("\n"):
                raise ValueError("Oversized or incomplete scene frame")
            frame = json.loads(line)
            if frame["frame_index"] != count:
                raise ValueError("Frame indices must be contiguous, starting at zero")
            timestamp = number(frame["time_seconds"], "time_seconds")
            if not math.isclose(timestamp, count / fps, rel_tol=1e-8, abs_tol=1e-8):
                raise ValueError("Frame timestamp disagrees with FPS")
            if not isinstance(frame["shapes"], list):
                raise ValueError("shapes must be a list")
            yield frame
            count += 1
    if count != manifest["frame_count"] or not count:
        raise ValueError("Manifest frame_count disagrees with frames.jsonl, or scene is empty")


def transform(shape, mapping, options, layer):
    index = integer(shape["brush_index"], "brush_index")
    if index not in mapping:
        raise ValueError(f"No mapping for brush {index}")
    row = mapping[index]
    entity = row.get("entity_type")
    if not isinstance(entity, str) or entity not in MOBS:
        raise ValueError(f"Fill entity_type for brush {index} ({row.get('source_name', '?')}), e.g. minecraft:creeper")
    reference = vector(row.get("reference_size_blocks"), 2, f"Brush {index}: reference_size_blocks", True)
    pivot = vector(row.get("pivot_blocks", [0, 0, 0]), 3, "pivot_blocks")
    yaw_offset = number(row.get("yaw_offset_degrees", 0), "yaw_offset_degrees")
    depth = number(row.get("depth_blocks"), f"Brush {index}: depth_blocks", True)
    x, y = vector(shape["center_px"], 2, "center_px")
    w, h = vector(shape["size_px"], 2, "size_px", True)
    rotation = number(shape["rotation_radians"], "rotation_radians")
    opacity = number(shape["opacity"], "opacity")
    hue = number(shape["hue_turns"], "hue_turns")
    saturation = number(shape["saturation"], "saturation")
    brightness = number(shape["brightness"], "brightness")
    if shape["original_colors"] is not True or abs(opacity - 1) > 1e-4 or abs(hue) > 1e-4 or abs(saturation - 1) > 1e-4 or abs(brightness - 1) > 1e-4:
        raise ValueError("Real mobs need original colors, opacity 1 and unchanged HSV. Use the app's mob settings; interpolation fades need to be disabled.")
    scales = [w / options.pixels_per_block / reference[0], h / options.pixels_per_block / reference[1]]
    if not math.isclose(scales[0], scales[1], rel_tol=0.02, abs_tol=1e-6):
        raise ValueError(f"Brush {index}: width and height require different scales. Check reference_size_blocks and use uniformly scaled square brush textures.")
    scale = sum(scales) / 2
    max_scale = 3 if entity == "minecraft:shulker" else 16
    if not 0.0625 <= scale <= max_scale:
        raise ValueError(f"Mob scale {scale:g} is outside the supported range for {entity}. Adjust pixels_per_block or brush calibration.")
    if options.view == "side":
        if abs(math.remainder(rotation, math.tau)) > 1e-4:
            raise ValueError("Side views require rotation disabled in the app")
        pos = [x / options.pixels_per_block, -y / options.pixels_per_block, -layer * options.layer_gap]
        yaw = yaw_offset
    else:
        pos = [x / options.pixels_per_block, layer * options.layer_gap, y / options.pixels_per_block]
        yaw = math.degrees(rotation) + yaw_offset
    # pivot is the entity's feet relative to the normalized brush center at
    # scale 1. Rotate its horizontal part with the entity yaw.
    theta = math.radians(yaw)
    px = pivot[0] * math.cos(theta) - pivot[2] * math.sin(theta)
    pz = pivot[0] * math.sin(theta) + pivot[2] * math.cos(theta)
    pos = [pos[0] + px * scale, pos[1] + pivot[1] * scale, pos[2] + pz * scale]
    return entity, pos, (yaw + 180) % 360 - 180, scale, depth * scale


def format_number(value):
    text = f"{value:.6f}".rstrip("0").rstrip(".")
    return "0" if text in ("", "-0") else text


def coordinates(pos):
    return " ".join("~" + format_number(v) for v in pos)


def generate(scene, output, options):
    scene, output = Path(scene).resolve(), Path(output).resolve()
    if output.exists():
        raise ValueError(f"Output already exists: {output}. Choose a new folder.")
    manifest = json.loads((scene / "scene.json").read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1:
        raise ValueError("Unsupported scene schema")
    if not manifest.get("complete") and not options.allow_partial:
        raise ValueError("Scene is incomplete. To use saved frames, explicitly pass --allow-partial.")
    mapping = read_mapping(Path(options.mapping) if options.mapping else scene / "minecraft_mapping.json")
    number(options.pixels_per_block, "pixels_per_block", True)
    number(options.layer_gap, "layer_gap", True)
    if not RESOURCE.fullmatch(options.dimension):
        raise ValueError("Invalid dimension ID")
    if options.max_mobs <= 0:
        raise ValueError("max_mobs must be positive")
    fps = number(manifest["fps"], "fps", True)
    count = integer(manifest["frame_count"], "frame_count")
    # Validate EVERY frame before creating output, including frames that would
    # be dropped by the game's 20-tick-per-second timing.
    previous = {}
    peak = 0
    for frame in read_frames(scene, manifest):
        peak = max(peak, len(frame["shapes"]))
        if peak > options.max_mobs:
            raise ValueError(f"Scene has {peak} mobs. Default replay budget is {options.max_mobs}; set --max-mobs explicitly to try a larger scene.")
        current = {}
        for layer, shape in enumerate(frame["shapes"]):
            ident = integer(shape["id"], "id")
            if ident in current:
                raise ValueError("Duplicate shape ID in frame")
            entity, _, _, _, _ = transform(shape, mapping, options, layer)
            if ident in previous and previous[ident] != entity:
                raise ValueError("A surviving ID changed its entity type")
            current[ident] = entity
        previous = current
    output.parent.mkdir(parents=True, exist_ok=True)
    namespace = "tf_" + uuid.uuid4().hex[:10]
    group, anchor, team = namespace + "_mob", namespace + "_origin", namespace
    duration_ticks = max(1, math.ceil(count * 20 / fps)) if manifest["is_video"] else 1
    if duration_ticks > options.max_ticks or duration_ticks >= 2**31:
        raise ValueError(f"Replay needs {duration_ticks} ticks; budget is {options.max_ticks}. Set --max-ticks explicitly to try a longer video (maximum 2^31-1).")
    with tempfile.TemporaryDirectory(prefix=".teekasfigure-", dir=output.parent) as staging:
        root = Path(staging) / "pack"
        functions = root / "data" / namespace / "function"
        functions.mkdir(parents=True)

        def write(name, commands):
            if shutil.disk_usage(root.parent).free < 256 * 1024 * 1024:
                raise ValueError("Stopped: less than 256 MiB disk reserve")
            path = functions / (name + ".mcfunction")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("\n".join(commands) + "\n", encoding="utf-8")

        stop = [f"schedule clear {namespace}:step",
                f"execute in {options.dimension} as @e[tag={group}] at @s run tp @s ~ -2048 ~",
                f"execute in {options.dimension} run kill @e[tag={group}]",
                f"execute in {options.dimension} run kill @e[type=minecraft:marker,tag={anchor}]",
                f"team remove {team}", f"scoreboard objectives remove {namespace}"]
        write("stop", stop)
        write("start", [f"function {namespace}:stop", f"team add {team}",
              f"team modify {team} collisionRule never",
              f"scoreboard objectives add {namespace} dummy",
              f"scoreboard players set #tick {namespace} 0",
              f'execute in {options.dimension} run summon minecraft:marker ~ ~ ~ {{Tags:["{anchor}"]}}',
              f"function {namespace}:step"])

        # Binary dispatch: log2(ticks) score checks instead of testing every
        # video frame each game tick. Scheduler always uses a fixed name, so
        # stop can reliably cancel playback.
        def dispatch(lo, hi):
            name = f"dispatch/{lo}_{hi}"
            if lo == hi:
                write(name, [f"execute in {options.dimension} at @e[type=minecraft:marker,tag={anchor},limit=1] run function {namespace}:frames/{lo:06}"])
            else:
                mid = (lo + hi) // 2
                left, right = dispatch(lo, mid), dispatch(mid + 1, hi)
                write(name, [f"execute if score #tick {namespace} matches {lo}..{mid} run function {namespace}:{left}",
                             f"execute if score #tick {namespace} matches {mid + 1}..{hi} run function {namespace}:{right}"])
            return name

        root_dispatch = dispatch(0, duration_ticks - 1)
        write("step", [f"function {namespace}:{root_dispatch}",
              f"scoreboard players add #tick {namespace} 1",
              f"execute if score #tick {namespace} matches ..{duration_ticks - 1} run schedule function {namespace}:step 1t replace"])

        previous = {}
        iterator = iter(read_frames(scene, manifest))
        frame = next(iterator)
        next_frame = next(iterator, None)
        total_commands = 0
        previous_frame_index = None
        for tick in range(duration_ticks):
            # Nearest previous source frame at the tick's time. >20 FPS is
            # resampled; lower FPS holds the same frame for multiple ticks.
            source_index = min(count - 1, math.floor(tick * fps / 20 + 1e-9))
            while next_frame is not None and next_frame["frame_index"] <= source_index:
                frame, next_frame = next_frame, next(iterator, None)
            if frame["frame_index"] == previous_frame_index:
                # NoGravity + no collisions: an unchanged source frame needs
                # no teleports, scale updates or repeated entity selectors.
                write(f"frames/{tick:06}", [])
                continue
            shapes = frame["shapes"]
            current = {shape["id"]: shape for shape in shapes}
            commands = []
            for ident in sorted(previous.keys() - current.keys()):
                dead = f"@e[tag={group},tag={namespace}_{ident}]"
                # Hide the entity before kill to keep vanilla's death animation
                # out of the video. Only entities belonging to this pack move.
                commands.extend([f"execute as {dead} at @s run tp @s ~ -2048 ~", f"kill {dead}"])
            stack_depth = 0.0
            for layer, shape in enumerate(shapes):
                ident = shape["id"]
                entity, pos, yaw, scale, depth = transform(shape, mapping, options, 0)
                # Spatial separation guarantees ordering when depth_blocks
                # encloses the actual visual model. Spawn order is irrelevant.
                if options.view == "top":
                    pos[1] = stack_depth
                else:
                    pos[2] = -(stack_depth + depth / 2)
                stack_depth += depth + options.layer_gap
                tag = f"{namespace}_{ident}"
                selector = f"@e[type={entity},tag={group},tag={tag},limit=1]"
                place = coordinates(pos)
                nbt = f'{{Tags:["{group}","{tag}"],NoAI:1b,NoGravity:1b,Silent:1b,Invulnerable:1b,PersistenceRequired:1b,IsImmuneToZombification:1b,DeathLootTable:"minecraft:empty"}}'
                commands.extend([
                    f"execute unless entity {selector} run summon {entity} {place} {nbt}",
                    f"team join {team} {selector}",
                    f"tp {selector} {place} {format_number(yaw)} 0",
                    f"attribute {selector} minecraft:generic.scale base set {format_number(scale)}",
                ])
            write(f"frames/{tick:06}", commands)
            total_commands += len(commands)
            previous = current
            previous_frame_index = frame["frame_index"]
        (root / "pack.mcmeta").write_text(json.dumps({"pack": {
            "pack_format": 48, "description": "TeekasFigure living mob video (Java 1.21/1.21.1)"
        }}, indent=2), encoding="utf-8")
        instructions = f"""Minecraft Java 1.21 / 1.21.1 only. No mods required.
Copy this folder into <world>/datapacks/, then run /reload.
Stand at the top-left origin of the image and run:
  /function {namespace}:start
Stop and remove ONLY this player's mobs:
  /function {namespace}:stop
Run start again to replay. The last frame remains visible at the end.
View: {options.view}; pixels per block: {options.pixels_per_block}; layer gap: {options.layer_gap} blocks.
Top: look vertically down; newer/higher layers have larger Y.
Side: look towards +Z; newer/higher layers have smaller Z.
Keep the origin and ALL mobs in loaded chunks in {options.dimension}.
Do not rotate the camera during playback. Perspective, lighting, entity poses
and transparency inside source PNGs can differ from the app's sprite preview.
No global gamerules or force-loaded chunks are changed.
Peak mobs: {peak}. Ticks: {duration_ticks}. Source FPS: {fps:g}. Command count: {total_commands}.
Game playback is 20 ticks/second, so higher source FPS is resampled.
Large mob counts can lag Minecraft; --max-mobs is an explicit replay budget.
Pack structure and generated commands tested automatically; in-game execution
must still be checked in your world. No Minecraft server is bundled.
"""
        (root / "PLAY.txt").write_text(instructions, encoding="utf-8")
        # TemporaryDirectory owns all staging files. Never delete/rewrite an
        # existing destination, including when validation or disk writes fail.
        if output.exists():
            raise ValueError("Output appeared during conversion; refusing to replace it")
        root.rename(output)
    return output, namespace


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("scene", type=Path)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--mapping", type=Path)
    p.add_argument("--view", choices=("top", "side"), default="top")
    p.add_argument("--pixels-per-block", type=float, default=16)
    p.add_argument("--layer-gap", type=float, default=0.05)
    p.add_argument("--dimension", default="minecraft:overworld")
    p.add_argument("--max-mobs", type=int, default=256)
    p.add_argument("--max-ticks", type=int, default=72000)
    p.add_argument("--allow-partial", action="store_true")
    return p


def main():
    args = parser().parse_args()
    try:
        output, namespace = generate(args.scene, args.output, args)
    except (ValueError, KeyError, TypeError, OSError, StopIteration) as error:
        raise SystemExit(f"Datapack not created: {error}") from error
    print(f"Created {output}\nPlay: /function {namespace}:start\nStop: /function {namespace}:stop")


if __name__ == "__main__":
    main()
