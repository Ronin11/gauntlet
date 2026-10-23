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

The game follows the GameCube original, rebuilt from its decompiled code: licence screen
and movies, the title (Start / Options), player select, the tower hub with a portal per
level, the map screen and intro movie, the level, then back to the tower.

On player select each controller presses **Start** to join, chooses **New**, types a
name, picks a class (left/right) and colour (up/down), and presses **A**. The keyboard
counts as a controller (Enter). In the tower, a new controller can join with Start.
**Start** during play opens the Game Menu (Tower Menu in the tower).

| Action | GameCube | Controller | Keyboard |
|---|---|---|---|
| Move (run past 3/4 of the stick) | stick | Left stick | WASD / arrows |
| Attack (melee in reach, else throw) | A | A (South) | J / Space |
| Slow attack (after A: power swings) | Y | Y (North) | I |
| Turbo attack (hold, then A) / block | B | B (East) | K / Shift |
| Charge / strafe (hold) / 2-player combo (hold) | L / R / Z | LT / RT / RB | H / U / O |
| Magic (use a potion) | X | X (West) | L |
| Power-up inventory: open / switch on-off | D-pad up | D-pad up | Tab |
| Inventory: previous / next / close | D-pad left / right / down | D-pad | Q / E / R |
| Back in menus | B | B | K / Backspace |
| Menu | Start | Start | Enter / Esc |
| Fullscreen | | | F11 |

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
events per device, so several controllers can play at once; their left sticks are read from
the joystick motion events, and sound plays through AAudio. Touch controls are not done. The APK is signed with a personal key created
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
- **Items** as in items.c: food heals its value (spoiled food poisons; it stays put at full
  health), gold adds its value, keys and potions (coloured) stop at 9, chests open with a
  key, barrels break, gates take a key, scrolls show the level's hints, crystals open the
  tower's barriers.
- **Levers and pressure pads**: walk into a lever or onto a pad to raise elevators, sink
  spiked barricades and reveal bridges, with the original's latch/toggle/hold rules.
- **Up to 4 players** on gamepads and keyboard, with the original's status panels. No
  health drain (the GameCube version has none); dead players wait in the tower.
- **Combat** from the decompiled rules: speed, damage, armour and magic come from each
  class's stats and level; experience and level-ups (+100 health, +5 stats); single-target
  melee within reach of the aim cone, otherwise a throw; potions spread a ring as wide as
  the magic stat; the turbo meter fills over time and with melee, and B+A spends it on a
  turbo attack; holding B blocks.
- **Music**: each world's soundtrack, decoded from the disc's DSP-ADPCM streams.
- **Sound effects and announcer**: weapon hits, monster deaths, pickups, gates, hero pain
  and death voices, and "Red Warrior needs food, badly!", decoded from the `.VBK` banks
  using the sound names in `AUDATPS2.ROM`.
- **Camera**: the GameCube's NEWCAM gameplay camera (each area's camera locator, framing
  every player, holding the party inside the view), the boss cameras, camera shakes, and
  the attract-mode level flybys.
- **Bosses**: all eleven Guardians with their attacks, health meters, safe rocks, Items of
  Legend (the holder's throw and the Guardian's weakened phase), the boss key and the good
  wizard; golems, gargoyles and generals as critters.
- **World hazards**: lava and fire floors and walls, damage tiles, falling and sinking
  rocks, and heroes who fall out of the world come back beside a partner.
- **Thrown weapons** fly like the original's: per-class weight and arc, 3-way and 5-way
  fans, launch-time wall checks, wall-hit sounds.
- **Texture animation**: lava, water, torches, skies and glowing models.
- **Save games** on a virtual memory card, with the original's load/save screens.
- **Power-ups** wait in each player's inventory until switched on from the D-pad, as on
  GameCube; the name shows over the player's panel, glowing while it is on.
- **Sumner** in the tower: the welcome story, his speeches and the stained-glass and
  runestone scenes when shards and runestones are brought back, level-up congratulations,
  the crystal and gargoyle-gate messages, and his hint menu at the lectern.
- **Shop** after each level and from the Tower Menu; **map screen** and intro movies.

## Not done yet (roadmap)

- Particle effects and most hit/burst effect models
- Android touch controls (Bluetooth controllers work)

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

Headless test hooks (environment variables): `GDL_MUTE=1` (no audio), `GDL_FUZZ=seed`
(random inputs), `GDL_LEGEND=1` (the party holds the realm's Item of Legend), `GDL_INV=1`
(power-up inventory), `GDL_PUP=9:80,lvl:85` (power-ups switched on by type:hex flags, hero level), `GDL_SUMNER=welcome|runes=..|shards=..|exp=N`, `GDL_FLYBY=1`,
`GDL_NO_BOSS_CAMERA=1`, `GDL_DEBUG_CAM=1`.

## Credits and references

Format knowledge comes from the community tools, verified against the retail disc:
[MosesofEgypt/gdl_tools](https://github.com/MosesofEgypt/gdl_tools),
[haekb/io_scene_gdl](https://github.com/haekb/io_scene_gdl),
[MeltyTool](https://github.com/MeltyPlayer/MeltyTool), and engine behaviour from the
[GameCube decompilation](https://github.com/sabishii-bit/Gauntlet-Dark-Legacy-Decompilation).
Their code is not copied here. Gauntlet is a trademark of its respective owners.
You need your own copy of the game.
