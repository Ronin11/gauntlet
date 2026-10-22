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

Press **Start** on the title screen. On the hero select screen each controller presses
**Start** (or **A**) to join, picks a hero with **left/right**, and presses **A** to ready
up. The keyboard counts as a controller (Enter). **Start** (or Esc) pauses the game; the
pause menu can change heroes, go back to the title, or quit. The title, hero select,
loading screen, pause menu and HUD use the original game's art and fonts from the disc.

| Action | Controller | Keyboard |
|---|---|---|
| Move | Left stick / D-pad | WASD / arrows |
| Attack | A (South) | J / Space |
| Turbo | X (West) / RT | L / Shift |
| Potion / back | B | K |
| Pause | Start | Enter / Esc |
| Fullscreen | | F11 |

Other options: `--level levelB1` to start elsewhere, `--players WAR,VAL/BLU` to skip the
join screen, `--mute`. See `gauntlet --help`.

## Android / tablets

```sh
scripts/build-android.sh                       # aarch64 APK for real devices
scripts/build-android.sh x86_64-linux-android  # for the emulator
adb install -r target/release/apk/gauntlet-rs.apk
adb shell mkdir -p /sdcard/Android/data/com.gauntletrs.game/files
adb push game.iso /sdcard/Android/data/com.gauntletrs.game/files/game.iso
```

Tested in the Android 15 tablet emulator: the join screen, joining with a gamepad's A
button, and gameplay all work. Bluetooth controllers report their D-pad and buttons as key
events per device, so several controllers can play at once. Analog sticks, sound and touch
controls are not hooked up on Android yet. The APK is signed with a personal key created
in `~/.android/gauntlet-rs.keystore`.

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
- **Levers and pressure pads**: walk into a lever or onto a pad to raise elevators, sink
  spiked barricades and reveal bridges, with the original's latch/toggle/hold rules.
- **Up to 4 players** on gamepads and keyboard, a Gauntlet-style HUD, health drain,
  kid-friendly respawning, a follow camera that keeps the party together.
- **Combat**: melee when something is in reach, otherwise a thrown weapon, at the original
  animation speeds (wind-up, release, recovery; quick-attack combos); potions (B/K) blast
  every monster nearby.
- **Music**: each world's soundtrack, decoded from the disc's DSP-ADPCM streams.
- **Sound effects and announcer**: weapon hits, monster deaths, pickups, gates, hero pain
  and death voices, and "Red Warrior needs food, badly!", decoded from the `.VBK` banks
  using the sound names in `AUDATPS2.ROM`.
- **Camera** follows each area's original camera heading and cuts away roofs and walls
  between it and the players.

## Not done yet (roadmap)

- Class-specific combos and special moves, power-ups (only basic pickups work so far)
- Scripted cameras for boss arenas, and some dark arena/boss levels
- Bosses, moving platforms and elevators, triggers, teleporters, secret areas
- Texture animations (flowing water, flames), particle effects
- Save games, the main menu (new/load game, options), shops
- Android: analog sticks (D-pad and buttons work), audio, touch controls

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
