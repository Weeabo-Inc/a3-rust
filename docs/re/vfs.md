# Virtual file system: mounting and priority

Implemented in `crates/a3-vfs`. Confidence is given per item.

## Paths

- Case-insensitive, backslash-separated; a leading backslash is ignored. **High** (every config
  reference in shipped data is written this way, in mixed case).
- `a3-vfs` normalises to lower case (ASCII only), `/` to `\`, and drops empty components.

## Prefix

- A PBO's files appear at `<prefix>\<entry name>`. The prefix is the `prefix` header property; when
  absent, the file stem of the PBO. **High** for the property; **medium** for the stem fallback.

## Load order (what `Vfs::mount_game` does)

1. `Dta/*.pbo` (core: `bin`, `core`, `languagecore_f`, `splashwindow`).
2. `Addons/*.pbo` (the base game, mod name `A3`).
3. Official DLC folders, each `<dir>/Addons/*.pbo`, in this order: `curator`, `kart`, `heli`,
   `mark`, `expansion`, `jets`, `argo`, `orange`, `tacops`, `tank`, `enoch`, `aow`.
   **Medium**: taken from the "Loaded mods" table of game RPT logs, which lists mods highest
   priority first. Needs confirmation from the executable.
4. Mods given by the caller (`-mod=` order), each `<mod>/addons/*.pbo`.

Inside one folder PBOs mount in file-name order, ignoring case. **Low**: matches NTFS enumeration
order, unconfirmed.

Folders not loaded by default but found by `optional_mod_dirs`: `Contact` (the Contact campaign,
loaded only when selected in the launcher) and creator DLC folders (`CSLA`, `GM`, `RF`, `WS`, `vn`;
these hold EBOs, which `a3-vfs` skips and reports).

## Overrides

- `a3-vfs`: a later mount overrides an earlier one **per file**.
- The real engine: **uncertain**. Community knowledge says a later PBO with the *same prefix* as an
  earlier one replaces it, and that a mod can only override a vanilla file by reusing the vanilla
  PBO's exact prefix. Whether the engine resolves a path through the longest matching bank prefix
  (and then fails if that bank lacks the file) or searches every bank is not known. In the
  vanilla install no two archives share a prefix and no file path is provided twice (the full
  mount reports 0 overrides), so this does not affect vanilla data.

## Measurements (build 2.22.0.154103, warm file cache)

`mount_game` with every optional folder: 508 PBOs, 504,771 files, 359 EBOs skipped, 0 failures,
0 overrides. Release build: ~0.6 s. Debug build: ~5.5 s. Only headers are read; archives are
memory-mapped.
