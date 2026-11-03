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
join screen, `--mute`. See `gauntlet --help`. Sound starts at 20% of the original's level (the Audio options scale on top); `GDL_VOLUME=0.5` sets another level.

## Playing with a loyal follower

```sh
gauntlet --companion VAL                       # Title screen as usual; it joins you, ready
gauntlet --players WAR --companion VAL/BLU     # Straight into a level
```

A computer-played hero, for when there is nobody else to play with. It has no powers you lack:
it sees the game and presses the same buttons, so a game with it records and replays like any
other, and it can run out of health like you can. It stays close, fights what comes near you,
and leaves the loot to you, except food when it is hurt worse than you are. It goes to an exit
only once you are waiting on one, and never takes a secret level by itself. If you fall and it
is still standing, it makes for the nearest exit so the party can go on.

It doesn't solve the level for you: keys, doors and levers are yours. Heroes can't walk out of
the camera's view, so if it ever gets cut off (a lift it missed), go back for it. It can't be
added from the menus yet.

## Playing over the network

```sh
gauntlet --host 7777                           # Wait for one other player on port 7777
gauntlet --host 7777 --net-players 4           # ...or up to three
gauntlet --join 192.168.1.20:7777              # Join them
```

Everyone needs the same build, the same disc and asset packs, and the same rules; the host
turns away anyone who differs and says why. The host needs the port open to the others, TCP and
UDP (forwarded on the router, over the internet). Then everyone chooses heroes on the usual
player select screen, and plays.

It is lockstep: every machine runs the whole game, and a tick runs once everyone's buttons for
it have arrived. So all see the same game, menus included (one player's pause menu pauses and
shows for all, as on one television), and what you press takes effect a few ticks later, chosen
from the connection's round trip (`--net-delay` sets it by hand, in sixtieths of a second). A
slow machine or connection slows everyone; nobody runs ahead. Each machine keeps its own
volume, and a recording of each level (`--record`), the same on every machine.

Machines compare a fingerprint of the game every two seconds, and "OUT OF SYNC" on the screen
and in the log means they differ, which is a bug worth reporting with the recordings.

- **Leaving:** choose Quit Game in the menu and you leave at once; the others play on. Heroes of
  those who have gone drop out of the level and the party.
- **A lost connection:** after 15 seconds the machine is given up on. With two playing the other
  plays on. With three or four the game ends for everyone: nothing can tell which of the last
  ticks each machine got that player's buttons for, and they could no longer be the same game.
- **Heroes:** a network game starts with new heroes and saves nothing to the memory card, and
  nobody new can join once it has started.

## Asset packs

The disc is the original game. An **asset pack** is a folder laid out like the disc's
`Gauntlet/` folder (`LEVELS/levelA1/textures.ngc`, `AUDIO/MUSIC.VBK`, ...); every file
in it replaces the disc's file of the same name, and the rest still comes from the disc.
Packs never change the game's rules, only what it looks and sounds like, so the original
is always one flag away.

```sh
gauntlet --assets my-pack                 # one pack (repeat the flag; the first wins)
GDL_ASSETS=hd-textures:new-music gauntlet # packs from the environment
gauntlet --no-assets                      # the disc alone
```

Without flags, every folder inside an `asset-packs/` directory beside the game data, beside
the executable or in the working directory is a pack, applied in name order (number them,
`10-textures`, `20-music`, to choose the layering). On Android the folder sits beside the
disc image. The log names each pack as it is added.

The rules are the other half of the split. `--rules classic` (the default) plays the
GameCube original; `--rules remix` selects the modernised rules, which are the classic
values except where a remix feature changes one (so far: poison gas keeps hurting the
monsters standing in it, every half second as it does heroes, where the original hits
them once per phase of the cloud). Rules are a value the simulation reads
(`crates/gdl-sim/src/rules.rs`), so a tunable moves in there only when something needs
to change it, keeping its original value as the classic default.

## Determinism

Two copies of the game given the same inputs stay identical to the bit, on any platform:
that is what networked play will stand on. The simulation is a fixed 60 Hz tick, takes its
trigonometry from the pure-Rust `libm` crate rather than the platform's C library, and
changes only through values that can be shared (`LevelSetup` before the first tick, `Command`
between ticks). The debug menu and headless tools still write into the game directly.

```sh
# A digest per level after 30 s of bots playing: compare two builds with diff.
cargo run -p gdl-sim --release --example sim_digest -- --data game-data > a.txt
# Add --sections to name what differs.

# What two copies must share to play together: build, game files, rules, debug switches.
gauntlet --compat

# Every level, the whole state, twice over.
cargo test -p gdl-sim --release -- --ignored every_level
```

### The session

Everything from the party's first level to game over runs in `gdl_sim::session::Session`:
the party and what it carries, level ends, the shop, the map and movie timing, secret levels
and the freeze of a pause. It moves only on one tick's inputs and on recorded decisions, it can
be cloned, and `Session::digest` fingerprints it, so peers walking the same road stay the same.
The engine draws what it reads back and acts on the events the session reports. Player select
and the pause menu stay in the engine, since they read a player's own memory card and options;
they report what was chosen as decisions.

## Replays

A level's recording is how it started and what every player pressed. Played on the same
build and files it is the same game to the bit; played on a newer build, it shows whether a
fix changed what happened. Sticks reach the simulation at 255 positions per axis, so nothing
is lost in the recording.

```sh
gauntlet --record replays/             # Keep a recording of each level played
gauntlet --replay FILE                 # Watch one; Esc quits. Nothing is saved
gauntlet --replay-check FILE           # Headless: compare, exit 1 if it differs
```

The check names the second in which a game drifted (the file holds a fingerprint per second)
and the parts of the final state that differ. It says when the file came from another build
or other game files, but plays it anyway. If the debug menu changes the running game (warp
aside, which starts a new level), the recording stops at that tick and says so.

## Debug menu (developer tool)

Not part of the original. **F1** on the keyboard, or on a gamepad **hold L + R + Z and
press Start** (LT + RT + RB), opens it from the title screen, the tower or a level; the
game is frozen while it is open. Up/down choose, left/right change a value, A / Enter
applies, B goes back a page, Esc or F1 closes. Pages:

- **Warp to level**: any level of any realm (secret levels, bosses, the tower) with the
  current party (from the title: one Warrior on the controller that opened it).
- **Progress**: open everything; per realm levels beaten; boss runes, runestones,
  crystals per colour, gargoyle items, legend weapons; reset. Changes go into the party's
  progress (and the level's copy), so saving afterwards keeps them; values stay in the
  game's own ranges. Changed in the tower, the tower reloads on closing.
- **Unlocks and cheats**: secret heroes (all, or one by one; also offered on player
  select this session), and the name cheats given to the party.
- **Party**: per player or all: level, experience, gold, keys, potions, full health,
  any power-up, clear power-ups, god mode (no damage; "GOD" shows while on), class and
  hidden skin (the level reloads).
- **Level**: kill every monster, destroy every generator, kill the boss, finish the
  level, X-ray and Compass.
- **Save / load state**: save the running level to slot 1-3, or load slot 1-3 or the
  autosave (also from the title screen). See below.

- **File bug report**: optionally tick quick tags with A or left/right ("stuck", "can't
  reach", "wrong look", "wrong sound", "crash-ish", "other"), then **Save bug report now**.
  The menu answers "Bug report saved: <folder>".

`gauntlet --screen debug[:main|warp|progress|unlocks|party|level|bug|states[:row]] --snapshot out.png`
renders it headless over `--level`.

### Bug reports

Each report is a folder `<YYYYMMDD-HHMMSS>-<level>` (UTC) of plain text and PNG, written to
`$XDG_DATA_HOME/gauntlet-rs/bugs/` (default `~/.local/share/gauntlet-rs/bugs/`) on desktop,
and to the app's external files folder on Android, pulled with
`adb pull /sdcard/Android/data/com.gauntletrs.game/files/bugs`. Code: `bugreport.rs`.

- `replay.txt`: the level from its start up to the report, to watch or check (see Replays).
- `report.md`: time, build (`git describe`), screen, tags, level/wave/realm, every player
  (class, colour, slot, level, health, gold, keys, potions, power-ups, state, position,
  yaw, floor triangle and the object it belongs to, animation), camera eye/at, progress,
  options and control styles, tick count, and a headless command that reproduces the spot
  (`gauntlet --level .. --players .. --spawn x,y,z --eye .. --at .. --snapshot out.png`).
- `nearby.txt`: within 40 units of each hero: entities (kind, the placed item they came
  from with its info name/type/subtype, position, animation and frame, health, model,
  state), placed item instances, movers (elevators, bridges, barricades: active, held,
  offset, shown) with the triggers that drive them, transporter pads, world objects with
  collision (flags, hidden, triangles enabled/disabled), the nearest locators and camera
  locators, and every exit and generator in the level.
- `events.log`: the last 2 minutes, kept all the time while playing: log lines, messages
  (msgPost index and text), sounds, trigger presses and mover changes, pickups, health
  changes, kills, player state changes (deaths, exits), level starts/ends, debug menu
  actions, and a per-second input summary per player.
- `screenshot.png` (the level with its HUD, without the menu), `save.txt` (heroes and
  progress as text) and a copy of the memory card file.

Headless: `GDL_BUGREPORT=<frame> [GDL_BUGTAGS=stuck,other] gauntlet --level levelA1 --bot
--simulate 3000 --snapshot out.png` stops the simulation at that frame and files a report
(the snapshot is its screenshot).

## Save states (developer tool)

Not part of the original: a quick save for testing that a **new build** can load, so a
fix can be swapped in and play continued where it was. The easy path:

```sh
# play; quit the game (close the window, Esc on the title, or Quit Level / Quit Game)
cargo build --release
target/release/gauntlet --load-state autosave
```

The `autosave` slot is written when a level is quit or the game is closed from a level,
and every minute of play. The debug menu's **Save / load state** page writes and reads
slots `1`-`3`. `--load-state` takes a slot name or a `.state` file; states live in
`$XDG_DATA_HOME/gauntlet-rs/states/` (`~/.local/share/gauntlet-rs/states/`; on Android
the app's files folder). The file is plain text, so it can be read and edited.

A state is not a byte-exact snapshot: loading starts the level fresh and applies what
changed, matching placed items by their instance in the level file (not by code-made
ids), so it survives code changes. Unknown or missing fields are ignored or defaulted.

- **Kept**: the level and difficulty; each hero's slot, controller (by pad order),
  class, colour, skin, name, level, experience, shop bonuses, health, turbo, gold, keys,
  potions, power-ups (with time left and on/off), inventory box, totals, place and
  facing; the progress (levels beaten, runes, shards, crystals, gargoyle items, legend
  weapons, Sumner's bank); placed items taken, broken or opened (doors, chests,
  barrels, killed placed monsters, destroyed generators) with their collision;
  placed monsters' and generators' health, tier and place; triggers pressed; trigger
  targets (elevators, bridges, gates) on/off and height; world animations' frames and
  rotators' turns; items lying loose; the level clock. A hero saved inside geometry
  is moved to the nearest floor.
- **Not kept**: monsters made by generators (the generators make new ones), missiles
  and effects, monster AI and critter (golem, boss) health, exits half open, fallen
  rocks, the camera (it starts on the heroes), messages already seen in the level, and
  the level a secret level was entered from (a state saved in a secret level loads
  just that secret level).

Headless: `--save-state [N:]<slot|file>` (or `GDL_SAVE_STATE`) saves during
`--simulate` on frame N (default: at the end), and `--load-state` starts the snapshot
from a state, e.g.

```sh
GDL_MUTE=1 gauntlet --level levelA1 --bot --simulate 3600 --save-state a1.state --snapshot a.png
GDL_MUTE=1 gauntlet --load-state a1.state --simulate 2 --snapshot b.png
```

## Level runner (developer tool)

Not part of the original: a bot that reads the game state (not pixels) and plays a level
headless at full simulation speed from its start to its exit, then says where and why it
failed. Code: `crates/gdl-engine/src/runner/`.

```sh
GDL_MUTE=1 gauntlet --run-level levelA1 [--runner-party WAR,VAL] [--runner-timeout 1800] \
    [--runner-out runner-out] [--runner-verbose]
GDL_MUTE=1 gauntlet --run-level A            # a realm; `all` for every level
scripts/run-levels.sh -j 4 -o runner-out     # every level, one process each, niced
gauntlet --runner-merge runner-out           # rebuild runner-out/summary.md
```

- **Navigation**: floor samples on a 2-unit grid, joined where the hero's own movement
  rules (ledge limit, one-sided walls, stairs, solid items, falling) walk from one to the
  next; refreshed as movers, doors and walls change; A* to goals; transporter pads.
- **Objectives**, as a player would: the exit; else what stands in the way (a door and a
  key for it, a breakable wall, barrel or generator on the way), levers and pads judged by
  what their target would open up (a lookahead that moves the target), rotators, keys,
  anything else to pick up; fights what comes near, hunts generators that keep spawning,
  drinks potions; in the tower collects the crystals, opens the first barrier, visits the
  lectern and walks into a portal; on boss levels fights the boss (a debug kill after a
  few minutes, recorded) and waits for the good wizard; timed levels are played out.
  With several heroes the others follow the first and gather on the exit.
- **God mode** (`Game::debug_god`) goes on when a hero drops below 35% health, and the
  result says so (`completed_god`).
- **On failure** (stuck, unreachable, timeout, exit reached but no end, NaN, a door that
  won't open to a hero with a key, falling out of the world): a bug report with a
  `runner.md` note (objective, path, last positions, objectives so far), a `stuck.state`
  save state and a top-down `navmap.png` (reachable floor green, the rest grey, possible
  floor seams pink). An unreachable exit's report lists the floor seams that stopped the
  nav's walks at the edge of the reachable area.
- Results: `<out>/summary.md`, `summary.tsv`, `summary.json`; per level `<out>/<level>/`.
  Debugging aids: `--runner-state <file>` starts from a save state, `--runner-probe
  x,y,z:x,y,z` walks the hero between two spots in the game and in the nav's simulation,
  `GDL_NAV_HMAP=x0,z0,x1,z1` prints a floor height map, `GDL_RUNNER_SAVE=<secs>:<file>`
  saves a state during a run, `GDL_RUNNER_DEBUG=<secs>` prints the hero's state.

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
  tower's barriers. Breakable walls (291 in 46 levels) hide secret rooms: swings, shots
  and explosions wear them down, and an Archer's potion shows them (level 25) or brings
  them down (level 50).
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
- **Particles** from the decompiled MB particle system: the levels' fire, smoke and mist
  emitters, blood, sparks, gem sparkles, generator hits, the wizard's wrist sparks and
  amulet glows, each from the game's own particle records and presets; thrown weapons
  leave the original's coloured streak.
- **Save games** on a virtual memory card, with the original's load/save screens.
- **Power-ups** wait in each player's inventory until switched on from the D-pad, as on
  GameCube; the name shows over the player's panel, glowing while it is on.
- **Sumner** in the tower: the welcome story, his speeches and the stained-glass and
  runestone scenes when shards and runestones are brought back, level-up congratulations,
  the crystal and gargoyle-gate messages, and his hint menu at the lectern.
- **Shop** after each level and from the Tower Menu; **map screen** and intro movies.

## Not done yet (roadmap)

- Firework and flame particle presets (sfx.c's festival explosions), trigger-driven
  world emitters
- Android touch controls (Bluetooth controllers work)

## Layout

```
crates/gcdisc       GameCube disc image reader (ISO/GCM/NKit)
crates/gdl-formats  Game file formats: objects/meshes, textures, worlds, animations, audio
crates/gdl-cli      `gdl` tool: ls/extract the disc, inspect and convert assets, read DOL data
crates/gdl-sim      The simulation: game rules, levels, heroes, monsters, the camera and
                    save data; deterministic, with no window, GPU or audio
crates/gdl-net      Network play: the lobby, and everyone's inputs for each tick (GGRS, lockstep)
crates/gdl-engine   The game: renderer (wgpu), screens, input devices, audio, HUD, on gdl-sim
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
`GDL_NO_BOSS_CAMERA=1`, `GDL_DEBUG_CAM=1`, `GDL_PSYS=1` (particle systems at the end),
`GDL_FX=<row|GENHIT|GENDIE>[:frame]` (start an effect in front of player 0).
`GDL_BLEND=<alpha>` runs one more tick and draws the frame `alpha` of the way to it, as a
fast display does (the game ticks at 60 Hz and the window interpolates between the last
two ticks; snapshots are otherwise the exact tick).

## Credits and references

Format knowledge comes from the community tools, verified against the retail disc:
[MosesofEgypt/gdl_tools](https://github.com/MosesofEgypt/gdl_tools),
[haekb/io_scene_gdl](https://github.com/haekb/io_scene_gdl),
[MeltyTool](https://github.com/MeltyPlayer/MeltyTool), and engine behaviour from the
[GameCube decompilation](https://github.com/sabishii-bit/Gauntlet-Dark-Legacy-Decompilation).
Their code is not copied here. Gauntlet is a trademark of its respective owners.
You need your own copy of the game.
