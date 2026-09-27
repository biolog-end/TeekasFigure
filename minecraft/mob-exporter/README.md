# TeekasFigure Mob Tools

Fabric, Minecraft Java 1.21.1, Java 21. Fabric API is not required.

Put `teekasfigure-mob-tools-0.1.4.jar` in this Minecraft installation's `mods` folder.
Keep only one version of the mod installed.
Open a singleplayer world and press Escape. Two buttons appear in the lower left.

## Mob brushes

**Mob brushes** offers **Export top view** and **Export facing forward**. Both
export transparent 512×512 PNGs of the actual default vanilla mob models,
facing north from above or facing the camera from the front. Sheep remain white
and no recoloring is applied. Captures are saved in
`teekasfigure_exports/mobs_top_.../` or `mobs_front_.../` beside this
installation's `saves` folder. The package records `view: "top"` or
`view: "front"`, so the application and playback mod can distinguish them.
**Open export folder** opens the previous export even after reopening the menu.
**Copy folder path** copies its path to the clipboard. Geometry and mob IDs are
included in `mob_mapping.json`; capture failures are listed in its `errors`.

Some mobs use fixed poses so their shapes read well from above: bats fly with
spread wings, axolotls spread their legs, and axolotl/tadpole fins are slightly
rolled to expose their zero-thickness polygons. The same poses are used during
playback.

## Importing into TeekasFigure

In TeekasFigure, open **Minecraft / scene export → Import mobs from Minecraft**
and select the folder containing `mob_mapping.json`. The application copies the
images into a separate library, selects it, enables original colors and JSON
export, and disables recoloring, nonuniform scale, opacity and fade interpolation.
Rotation is enabled for top-view captures and disabled for front-view captures,
keeping those mobs facing the camera. Choose the target image or video and generate.
The complete `output/scene_.../scene.json` can then be opened in the mod.
No Python, datapack or command block is required.

## Mob video

**Mob video** opens `scene.json` from TeekasFigure's output. Use Browse to select
the file, or paste its path / drop `scene.json` into the screen, then click
**Start scene**. Validation runs on a background thread before changing the world.
The mob budget defaults to 256; you can enter a higher value if Minecraft has
enough memory.

Playback uses actual mob entities with AI, physics, damage, idle animation and
sounds disabled for this session. A black block-display entity provides the
background without modifying world blocks. The mod reads the recorded view.
Top-view scenes lie horizontally under a downward camera. Front-view scenes stand
vertically; the camera faces north and the mobs face south toward it. Later shapes
move toward the camera where their alpha silhouettes overlap. Disjoint silhouettes
can share a depth; transparent holes do not force a separate layer. The whole
stage starts at Y=2048 or at least 1024 blocks above the world's build ceiling,
whichever is higher (and stays above a player already flying higher). Ground is
outside the viewing far plane when the camera resets to the stage.
Orthographic projection matches image sizes across different layer depths.

The player can fly, land, fall and turn normally. Menus close after Start scene.
The HUD is hidden automatically; F1 toggles it. Clouds, entity shadows and view
bobbing are temporarily disabled, particles minimal, view distance at most 2
chunks, simulation distance at most 5, and FPS at most 60. Existing lower values
are preserved. Viewing settings are restored on Stop, disconnect or game exit.

Controls offer pause/resume, reset camera, orthographic/perspective toggle, and
stop. **Stop and restore** removes the session's entities and chunk tickets and
restores the player's starting position and game mode. Playback also stops when
the owner disconnects or the server closes.

## Options

**Options → Delete other loaded entities** is OFF by default. Enabling it
irreversibly discards all currently loaded non-player entities in every dimension
before playback, including mobs, dropped items, item frames, boats and displays.
Discarding does not create drops. Unloaded chunks are not edited. Stop does not
restore these deleted entities.

## Performance

During background validation the decoder packs each frame's alpha-aware layers
once, writing a compact temporary binary cache with an offset index. Playback
bulk-reads cached records on one worker and prefetches the next frame; it does
not parse JSON or repack skipped frames. Only current/prefetched frames are in
RAM. Stop and failed/cancelled loading close and remove the temporary file. The
cache needs temporary disk space with a 256 MiB reserve and a bounded heap budget
for its index.

The playback clock starts after the first frame is assembled, uses elapsed real
time and excludes explicit playback pauses. Outdated frames are skipped if the
world cannot keep up, preserving duration instead of stretching the video.
Unchanged actors keep their IDs; retired actors are reused for new shape IDs of
the same mob type. Only changed transforms, scales and poses are applied. World
updates have a 4 ms budget per server tick; this does not guarantee that every
scene reaches its source FPS. Only playback mobs use a one-tick network tracking
interval; ordinary mobs are unaffected. The player menu reports actual scene
frames/s and skipped frames, which are separate from rendering FPS. Server-driven
entity updates run at most once per server tick (normally 20/s).

Chunk loading is requested with tickets and checked without blocking; a 20-second
chunk timeout cancels playback. Stage scale is uniform, with individual mob scales
at most 8 and total stage height at most 128 blocks.

If the client or server stops responding during playback for ten seconds, a
background diagnostic writes `teekasfigure_exports/playback-stall.txt` containing
thread stacks and deadlock information.

## Limitations

This version supports a local singleplayer world. It does not send scenes to a
remote dedicated server. Resource packs must match those used during capture;
shaders or different lighting can change the result. One default model per mob
type is captured, rather than every skin, baby or pose variant. Audio is not
imported.

## Building

Build the JAR with JDK 21:

```bat
gradlew.bat assemble
```

The JAR is written to `build/libs/`.
