# gauntlet-rs

A from-scratch Rust reimplementation of **Gauntlet: Dark Legacy** (Midway, 2001) that
runs on the original game data from your own GameCube disc, the way OpenMW runs Morrowind.
No game assets are included in this repository; nothing works without your disc image.

Goal: a rock-solid, portable version for couch co-op (desktop, and eventually tablets
with Bluetooth controllers) that doesn't depend on an emulator.

## Running

```sh
cargo build --release
./target/release/gauntlet --data "/path/to/Gauntlet - Dark Legacy (USA) (Rev 1).nkit.iso"
```

`--data` accepts the disc image (ISO/GCM, including NKit-scrubbed images) or a folder
extracted with `gdl extract`. If you omit it, the game looks for `$GDL_DATA`, a
`game-data/` folder, or an `.iso` next to the executable. `./run.sh` does the same.

On the join screen each controller presses **Start** (or **A**) to join, picks a hero with
**left/right**, and presses **A** to ready up. The keyboard counts as a controller (Enter).

| Action | Controller | Keyboard |
|---|---|---|
| Move | Left stick / D-pad | WASD / arrows |
| Attack | A (South) | J / Space |
| Turbo | X (West) / RT | L / Shift |
| Back (join screen) | B | K |
| Fullscreen | | F11 |
| Quit | | Esc |

Other options: `--level levelB1` to start elsewhere, `--players WAR,VAL/BLU` to skip the
join screen, `--mute`. See `gauntlet --help`.

## What works

- **Disc access**: reads the GameCube file system directly from the ISO (`crates/gcdisc`).
- **All 67 levels** load with textures, lightmaps, and additive glows. An automated test
  plays every level for 10 seconds without errors.
- **All 16 heroes** (the 4 classics, the Dark Legacy classes, and the secret characters),
  with the game's animations decoded the way the original engine does it (keyframe
  decoding and rotation matrices taken from the decompilation).
- **Monsters**: skeletal and mesh-swap animated (grunts, rats, ghosts, Death, ...), spawned
  by generators that can be destroyed. They chase and attack players.
- **Items**: food, keys, gold, gems, potions, barrels and chests (breakable), locked gates
  (open with a key), exit portals (load the next level), damage tiles.
- **Up to 4 players** on gamepads and keyboard, a Gauntlet-style HUD, health drain,
  kid-friendly respawning, a follow camera that keeps the party together.
- **Music**: each world's soundtrack, decoded from the disc's DSP-ADPCM streams.

## Not done yet (roadmap)

- Sound effects and voices (DCS sound banks, `AUDIO/*.VBK` + `AUDATPS2.ROM`)
- Ranged attacks, magic/potions, the class-specific combo and special moves, power-ups
- Per-area scripted cameras (milestones). Some indoor and cave levels get a bad angle.
- Bosses, moving platforms and elevators, triggers, teleporters, secret areas
- Texture animations (flowing water, flames), particle effects
- Save games, menus, shops
- Android/tablet build (wgpu, winit and gilrs all support Android; the NDK is installed)

## Layout

```
crates/gcdisc       GameCube disc image reader (ISO/GCM/NKit)
crates/gdl-formats  Game file formats: objects/meshes, textures, worlds, animations, audio
crates/gdl-cli      `gdl` tool: ls/extract the disc, inspect and convert assets, read DOL data
crates/gdl-engine   The game: renderer (wgpu), simulation, input, audio, HUD
```

Useful tools while developing:

```sh
gdl extract game.iso game-data          # unpack the disc
gdl objects <dir>/objects.ngc           # list models and textures
gdl textures <dir> out/                 # dump textures to PNG
gdl world LEVELS/levelA1/WORLDS.PS2     # level contents, items, spawns
gdl anim PLAYERS/WAR/ANIM/ANIM.PS2      # skeletons and animation list
gdl dol main.dol 0x80347B10:f32         # read constants out of the executable

gauntlet --snapshot out.png --simulate 600 --bot   # headless test run + screenshot
```

## Credits and references

Format knowledge comes from the community tools, verified against the retail disc:
[MosesofEgypt/gdl_tools](https://github.com/MosesofEgypt/gdl_tools),
[haekb/io_scene_gdl](https://github.com/haekb/io_scene_gdl),
[MeltyTool](https://github.com/MeltyPlayer/MeltyTool), and engine behaviour from the
[GameCube decompilation](https://github.com/sabishii-bit/Gauntlet-Dark-Legacy-Decompilation).
Their code is not copied here. Gauntlet is a trademark of its respective owners.
You need your own copy of the game.
