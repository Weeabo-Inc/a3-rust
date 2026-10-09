# SQF runtime semantics (arma3_x64.exe 2.22.0.154103)

Behaviour of the SQF VM's core, confirmed in the decompiled handlers (RVAs from
`docs/re/sqf-commands.tsv`; VA = RVA + 0x140000000) and in the offline community wiki
(`P:\ArmaWiki`). `crates/a3-sqf` follows these. Confidence is noted per item.

## Error codes and messages

`SetError(state, code, args...)` (`FUN_1402eb6d0`; `FUN_1402eb3b0` also records the
position) stores an `EvalError` code. The code→name table is built at `FUN_1400191e0`. The
message is the stringtable entry `STR_EVAL_<NAME>` from `languagecore_f\stringtable.xml`.

| Code | Name | Message (`Original`) |
|---|---|---|
| 1 | GEN | Generic error in expression |
| 2 | EXPO | |
| 3 | NUM | |
| 4 | VAR | Undefined variable in expression: %s |
| 5 | BAD_VAR | Reserved variable in expression |
| 6 | DIV_ZERO | Zero divisor |
| 7 | TG90 | |
| 8–13 | OPENB, CLOSEB, OPEN_BRACKETS, CLOSE_BRACKETS, OPEN_BRACES, CLOSE_BRACES | Missing ( ) [ ] { } |
| 14 | EQU | |
| 15 | SEMICOLON | Missing ; |
| 16, 17 | QUOTE, SINGLE_QUOTE | |
| 18 | OPER | |
| 19 | LINE_LONG | |
| 20 | TYPE | Type %s, expected %s |
| 21 | NAMESPACE | (local variable without a local scope) |
| 22 | DIM | %d elements provided, %d expected |
| 23 | UNEXPECTED_CLOSEB | |
| 24 | ASSERTATION_FAILED | |
| 25 | HALT_FUNCTION | |
| 26 | FOREIGN | (free-text errors, e.g. "Invalid switch block") |
| 27–31 | SCOPE_NAME_DEFINED_TWICE, SCOPE_NOT_FOUND, INVALID_TRY_BLOCK, UNHANDLED_EXCEPTION, STACK_OVERFLOW | |
| 32 | HANDLED | (not an error) |
| 33 | DIMMAX | Max array size would be reached. Current size: %d, wanted size: %d. |
| 34 | RECURSION | |
| 35 | WAIT_UNTIL | Undefined behavior: waitUntil returned nil. True or false expected. |

Codes 0 and 32 do not count as errors (`code & ~0x20 == 0`). Confidence: high (names and
numbers come from the table initializer, messages from the shipped stringtable).

## Errors: how the script continues

Confirmed on the original server with scratch probes (`tools/oracle/oracle.py
oracle --probes-dir`, 104 probes about errors, nil and sort; the recorded
`tools/oracle/probes/` results agree). High confidence.

- **An error a command handler raises is logged and the script goes on.** The
  handler still produces a value: `1/0` logs `Zero divisor` and is `inf`,
  `5 % 0` logs it and is 0, `[1,2,3] select 9` logs `3 elements provided, 10
  expected` and is the empty value, `"abc" regexMatch "("` logs the regexp
  error and is `false`, `{1} count [1,2]` logs `Type Number, expected Bool`
  and counts 0, `params ["a"]` in a global scope logs `Local variable in
  global space` and the block still returns its value, `compile "1 2"` logs
  `Missing ;` and is the empty value.
- **Only a failure the VM itself detects ends the script**: no overload takes
  the argument types (`1 + "x"` is `Generic error in expression`, `[1,2]
  select "a"` is `select: Type String, expected Number,Bool,Array,code`), or
  an unknown command. The probe's spawned script then never reports.
- **The code of `isNil {...}` runs in its own evaluation context**: an error
  in it ends the block, not the script, and `isNil` is `true`
  (`_r = 1; isNil {_r = 1 + "x"}; _r` is 1).
- **Only the first error of a script is written to the RPT**: `1/0; 5%0;
  "END"` logs one `Zero divisor` line and ends with `"END"`. A compile error
  from `compile`/`compileScript` is printed by the compiler as well, so
  `call compile "1 2"` shows two blocks. A consequence for diagnosis: a harmless
  first error hides a later fatal one, so a script that aborted can only be read
  from its first error line. The scenario sweep's `failed_scripts` is
  unattributable for `A3\functions_f\initFunctions.sqf` for exactly this reason.
- An undefined variable read is a logged error too, and yields nil
  (`a3ro_undefined + 1` is nil, the script continues).
- `sleep` in the unscheduled environment is the same: logged, and the script
  continues.
- The final-value errors below are the one case where the VM raises the error
  itself and the script still goes on.

## Final values: `compileFinal` and assignment

Confirmed on the original server with `tools/oracle/probes/61_errors.probes`
(issue #347); every claim below is one of those probes. High confidence.

`compileFinal` makes a **value** final: the code of `compileFinal "..."`,
`compileFinal {...}` or `compileScript [path, true]`, and the hash map of
`compileFinal createHashMap`. A variable of a *namespace* holding such a value
cannot be overwritten or deleted. Locals are not protected: the check belongs to
the variable space, not to the value, so `private _f = compileFinal "1";
_f = "2"` is `"2"` with no error at all.

**The error does not end the script** — it is logged and the script goes on.
That is what keeps `A3\functions_f\initFunctions.sqf` alive for the 35 campaign
Missions that compile their own `bis_fnc_camp_onmissioninit` over the library's
final one (issue #347; the issue expected an abort, the probe says otherwise).
The engine's own boot shows the same: it logs
`Attempt to override final function - bis_fnc_storeparamsvalues_data` and runs
on.

| Statement, while `x` holds a final value | Effect | RPT |
|---|---|---|
| `x = <anything but nil>` | refused, old value kept | `Attempt to override final function - x` |
| `x = nil`, final **code** | refused, old value kept | *(nothing)* |
| `x = nil`, final **hash map** | refused, old value kept | `Attempt to override final function - x` |
| `ns setVariable ["x", <anything but nil>]` | refused, old value kept | `Attempt to override final function - x` |
| `ns setVariable ["x", nil]` | refused, old value kept | `Attempt to delete final function - x` |
| `private _x` local, `_x = <anything>` | allowed | *(nothing)* |

The probes behind the table, with the value each returned and the RPT lines it
produced:

| Probe | Value | Evidence |
|---|---|---|
| `err.final_global_continues` | `"after"` | `Attempt to override final function - fina` |
| `err.final_global_value_rejected` | `"{1}"` | `Attempt to override final function - finb` |
| `err.final_assign_same_value`, `err.final_assign_self`, `err.final_assign_final_code` | `1` (the first code still runs) | `- fine`, `- finf`, `- fing` |
| `err.final_nil_final_global_read` | `false` (`isNil`), so the value stayed | *(none)* |
| `err.final_nil_hashmap_var_type` | `"HASHMAP"` | `Attempt to override final function - nilj` |
| `err.final_setvariable`, `err.final_setvariable_new`, `err.final_ui_namespace` | `"{1}"` | `- fink`, `- finl`, `- finm` |
| `err.final_nil_setvariable_final_read` | `false`, so the value stayed | `Attempt to delete final function - nilh` |
| `err.final_local`, `err.final_local_after_global` | `"""2"""`, `"2"` | none for the local, `- finh` for the global |
| `err.final_inside_call`, `err.final_inside_isnil` | `"{1}"` | `- fini`, `- finj` |
| `err.final_isFinal`, `err.final_str_and_call` | `[true,false,false]`, `["{1 + 1}",2]` | *(none)* |

Both messages name the **stored** variable, which the engine lower-cases — not
the name as spelled at the assignment site or in the `setVariable` key:
`finC_KeEpS_CaSe = compileFinal "1"; finC_KeEpS_CaSe = 2` logs
`- finc_keeps_case`, `finS_MiXeD = ...; fins_mixed = 2` logs `- fins_mixed`, and
`missionNamespace setVariable ["finU_MiXeD", ...]; finu_mixed = 2` logs
`- finu_mixed`. `Sym` lower-cases the same way, so `a3-sqf` already prints what
the engine prints. **Do not change this to the source spelling**: issue #347
asked for exactly that and the probe refuses it.

### Side observations (not implemented)

- `isNil` of a hash map is a **dispatch failure**, not a handler error:
  `isnil: Type HashMap, expected String,code`, and the script ends. `a3-sqf`
  matches (it registers `isNil` for STR and CODE), so the two probes that do this
  (`err.final_nil_hashmap_var`, `err.final_nil_setvariable_hashmap_read`) end
  early on both sides; their `_type` variants carry the assignment result.
- Reading a variable whose name is also a command name *after* deleting it raises
  `Reserved variable in expression` and ends the script. Found by accident:
  `finD = 1; finD = nil; isNil finD` — `finD` is `find`. A plain undefined
  variable only logs "Undefined variable in expression" and is nil.

## Variables and nil

- **Reading a variable** (`GameInstructionVariable::Execute`, `FUN_1402feef0`): when the
  variable is undefined or holds nil, the VM raises VAR ("Undefined variable in expression:
  _x") at the read, for locals and globals alike. The error is logged and the read is nil
  (server oracle). Two flags suppress it: one on the evaluation
  state (`+0x1c`) and one on the VM context (`+0x4c4`). The context flag is what `isNil {...}`
  sets, so the code it evaluates may read undefined variables. High confidence for the check;
  medium for which commands set the flags.
- **A command given `nil` or the empty value as an argument is skipped**; its result is
  nil. `typeName nil`, `str nil`, `typeName (call {})`, `nil isEqualTo nil` and `_a pushBack
  nil` all leave the command unrun (server oracle). `count [1, nil, 3]` is 3: a nil *inside*
  an array is a value, and prints `any`. High confidence.
- `nil` is the engine's ANY value (`GameDataNil`: `typeName` `ANY`, prints `any`,
  `FUN_1402d9bf0`); a command result with no data at all (NOTHING) prints
  **`<null>`** (`format ["%1", call {}]` is `"<null>"`), and so does an
  undefined array element and a missing hash map key. `isNil` is true for
  nil, for NOTHING and for `<null>` (`GameData +0x88`). High confidence.
- `private "x"` declares the variable but keeps the value of a variable that
  already exists in the current scope, so `private ["_this"]` inside a call
  leaves `_this` alone (`[1] call {private ["_this"]; _this}` is `[1]`). A
  declared but never assigned variable reads as an error, as before.
  High confidence.
- `isNil {code}` runs its code **unscheduled** (`canSuspend` is `false` in it).
  High confidence (server oracle).
- `if` without a matching branch gives the empty value, not a special value.

## Numbers

Confirmed on the original server with the oracle (`tools/oracle/probes/10_numbers.probes`,
`11_numbers_edge.probes`) and in the handlers below. High confidence unless noted.

- Numbers are `float`. `str` (`FUN_1402da290`) prints `"%g"` of the float widened to double
  through the statically linked UCRT `common_vsprintf` (options `*__local_stdio_printf_options()
  | 2`, a 127-byte buffer). The options do **not** select the legacy formats: exponents have
  two digits (`1e+06`, `1e-05`), infinities print `inf`/`-inf`, the FPU's default NaN
  (sign set, quiet bit only) prints `-nan(ind)`, any other NaN `nan` or `-nan`. The
  wiki's `"3.14159e-005"` and `1.#INF` come from older builds.
- A scalar that is infinite or NaN has the type **NaN** (`typeName (1/0)` is `"NaN"`,
  `(1/0) isEqualType 0` is false; the scalar's hash uses the NaN type when `_finite` fails,
  `FUN_1402d8380`). Commands whose signature lists `SCALAR` without `NaN` reject such values
  in the type check, e.g. `(1/0) toFixed 2` and `5 random (sqrt -1)` fail.
- The engine runs with the SSE flush-to-zero and denormals-are-zero modes: subnormal literals,
  results and `parseNumber` values are zero (`str 1e-38` is `"0"`, `str -1e-39` is `"-0"`,
  `1e-39 == 0` is true).
- `toFixed` binary (`FUN_140492b60`): digits are the right operand rounded with `cvtss2si`
  (ties to even) and clamped to 0..20, then `"%0.<n>f"` of the double: the exact decimal
  value, rounded ties-to-even (`0.5 toFixed 0` is `"0"`, `2.5 toFixed 0` is `"2"`,
  `123.456 toFixed 20` is `"123.45600128173828125000"`).
- `toFixed` unary (`FUN_140492c70`): rounds like the binary form, stores -1 (off) for values
  below -1 and at most 20 in the **script context** (`+0x4c8`), so the setting outlives the
  scope that set it (`call {toFixed 2}; str 1.5` is `"1.50"`). `str` reads the context of the
  running script.
- `round` (`FUN_1402e9b60`) is `floor(x + 0.5)` in single precision: halves go up
  (`round -2.5` is -2, `round -0.5` is 0) and `round 0.49999997` is 1.
- `parseNumber` (`FUN_1402e7b30`) is `(float)atof(s)`: whitespace, sign, decimal or hex
  (`"0x10"` is 16), `inf`, `nan`; the longest valid prefix, else 0.
- `min`/`max` are `a < b ? a : b` and `a > b ? a : b`: with a NaN operand the right one wins.
- `linearConversion` (`FUN_1405151c0`): when `|max - min| < 1e-6` the result is `minTo`
  itself; otherwise `(maxTo - minTo) * (v - min) / (max - min) + minTo`, clamped to the
  target range when the sixth element is true.
- `seed random x` (`FUN_140547ca0`): `seed` truncated to int (`cvttss2si`), hashed by
  `FUN_14030e340` (`u = ((p ^ 0x3d0000) >> 16 ^ p) * 9; u = (u >> 4 ^ u) * 0x27d4eb2d;
  (u >> 15 ^ u) & 0x7fff`, arithmetic shifts) and multiplied by the float `0x38000100`
  (1/32767), then by `x`. `seed random [x, y]` hashes the bit interleaving
  (`FUN_14030e120`) of `(int)(x * 100 + seed)` and `(int)(y * 100 + seed)`; another element
  count raises DIM and gives 0.

## Arrays

- `array select index` and `array # index` share one handler (`FUN_1402e4aa0`;
  `#` → `FUN_1402e5570`). The index is converted with `cvtss2si`, which rounds half to even
  (0.5 → 0, 1.5 → 2, 2.5 → 2). A negative index counts from the end (2.12+). An index equal to
  the size gives the empty value (`<null>`) with no error; anything further out logs
  "N elements provided, M expected" and also gives the empty value: `[1,2] select 3` →
  "2 elements provided, 4 expected" (server oracle: the script continues). High confidence.
- `array set [index, value]`: an index equal to the size appends, a negative index counts
  from the end (`_a set [-1, 9]` on `[1,2,3]` is `[1,2,9]`), one further out than `-size`
  logs `Zero divisor` and changes nothing, and the elements skipped over are the empty
  value (`[1,2,3] set [5, 9]` is `[1,2,3,<null>,<null>,9]`). High confidence.
- `array sort order` compares strings **without regard to ASCII case** and puts the
  uppercase spelling first when only the case differs: `["b","A","a","B"]` sorts to
  `["A","a","B","b"]`. Bytes above 127 compare as signed, so `["é","e","f","E"]` keeps `é`
  first. `order` false reverses the ascending result. Mixed types: the comparator is not a
  strict order (a number and a string compare equal in both directions), so the order of a
  mixed array is unspecified; `sort` does not error on one. High confidence for strings,
  medium for mixed types.
- `array select [start, count]` (`FUN_1402e4b80`): both values are rounded the same way. A
  start outside `0..size-1` gives `[]` with no error. A count < 1 gives `[]`. The range is
  clipped to the end. High confidence.
- `vectorAdd` keeps the size of the longer vector: 2D + 2D is 2D, 3D + 2D is 3D with the
  missing component zero. High confidence.


## Strings and forceUnicode

- `count string` (`FUN_1402e2710`) is the byte length (`strlen`). Under `forceUnicode` it is
  the UTF-16 length (`MultiByteToWideChar(CP_UTF8, …)`). High confidence.
- `forceUnicode mode` (`FUN_1404936a0`) stores the mode (rounded, clamped to -1..1) at context
  `+0x4cc`, so it applies to the whole script, child scopes included. The check
  `FUN_1402f8720` turns mode 1 back to -1 after one use. Mode 0 stays on until the end of the
  script, and -1 turns it off. Commands that check it (wiki): `copyFromClipboard`,
  `copyToClipboard`, `count`, `find`, `in`, `insert`, `reverse`, `select`, `splitString`,
  `trim`, `regexFind`, `regexMatch`, `regexReplace`. High confidence.

## Regular expressions

Boost.Regex is used (RTTI: `basic_regex<…, w32_regex_traits>`). Flag parsing is in
`FUN_1408637d0`:

- The flags are the text after the **last** `/`, if every character there is a flag letter.
  Otherwise the whole string is the pattern.
- No flags part: default syntax flags include `icase` (0x100000), and matching is global.
- A flags part: `icase` only with `i`; `g` makes it global, otherwise only the first match
  counts (`format_first_only`, 0x8000000). `n` adds 0x400000 (nosubs); `o` is accepted. So
  `pattern/` means case-sensitive, first match only.
- `regexMatch` (`FUN_140866000` → `FUN_14085d4b0`) runs the matcher with `match_all` (0x8000,
  set in `FUN_1408787c0`), so it is a whole-string match. The wiki guide's
  `"Hello there!" regexMatch "There"` example contradicts this; the `regexMatch` page's
  `"Cookie clicker" regexMatch "cookie/i" // false` agrees with it.
- Replacement strings use Boost's Perl format: `$&`, `$n`, `` $` `` (text since the previous
  match), `$'`, `$+{name}`, `\L`/`\U`/`\E`/`\l`/`\u` (wiki examples).

High confidence for the flags and match mode. Medium for the details of the format syntax
(taken from Boost's documentation and the wiki examples).

## Control flow

- **switch** (`CallStackItemSwitch`, step `FUN_140302340`): if no case matched and there is no
  `default`, the result is `true`. Errors are FOREIGN "Invalid switch block" and "Unable to
  evaluate switch block". High confidence.
- **for "_i" from a to b step s** (`CallStackItemForBASIC`, `FUN_140301690`): after each pass
  the loop reads `_i` back from its scope and continues from that value plus the step, so the
  body can move the loop. It runs while `i <= b` (for `s < 0`, while `i >= b`). The variable
  is private to the loop. High confidence. Medium for what happens when `_i` becomes a
  non-number: the loop stops in our implementation.
- **for [{init}, {cond}, {step}]**: the three blocks and the body share one loop scope.
  `private _i` in `init` stays local to the loop; `_i = 0` without `private` assigns an
  existing outer `_i` (wiki `for`, example 4). Medium confidence (wiki only).
- **while** (`CallStackItemRepeat`): unscheduled loops stop after 10,000 iterations (wiki
  "Scheduler"; the constant was not found in `FUN_140301ee0`, so the cap is applied
  elsewhere). Medium confidence.
- **Return types** declared in the command table: `WHILE do` → NOTHING, `WITH do` → NOTHING,
  `CODE forEach ARRAY` → NOTHING, `FOR do` → ANY, `SWITCH do` → ANY. a3-sqf returns the last
  body value from loops. To verify: the actual value `forEach` leaves.

## Hash maps

Supported key types (wiki "HashMap"): Array (of supported types, deep-copied), Boolean, Code,
Config, Namespace, NaN, Number, Side, String. Iteration order is unspecified. a3-sqf matches
this. Unverified in the binary.

- `HashMap set [key, value, onlyIfNotExists]` returns whether an **existing** value was
  overwritten: false for a new key, true for an existing one, false when `onlyIfNotExists`
  stopped the write (`[_h set ["a",1], _h set ["a",2]]` is `[false,true]`). High confidence
  (server oracle).
- A missing key (`get`, `deleteAt`) is the empty value, and `isNil {_h get "x"}` is true.
  High confidence.
- Iteration order: for up to four keys it is the insertion order; from five keys on it is
  the engine's hash order (`["b","a","c","d","e"]` comes back as `["e","a","b","c","d"]`).
  Not reproduced (a3-sqf keeps insertion order; issue #275). The nine orderings the oracle
  recorded fit the model "content hash, iterate buckets ascending, one constant bucket
  count, chain in insertion order", but they do **not** identify the hash or the count:
  ~1 in 130 random FNV-shaped hashes fits all of them, djb2/sdbm/java31/ELF/PJW are
  excluded outright (the last character must be mixed non-linearly), and no rehash is
  needed below ten keys. A trace that prints `keys` after **every** insertion would
  identify both; the recorded data cannot.

## Strings

- `format [format, args...]`: `%1`..`%N` are the arguments, `%%` is a literal `%`, and any
  other `%` is dropped, including a trailing one: `format ["100%"]` is `"100"`, `format
  ["a%bc"]` is `"abc"`, `format ["%1%%", 5]` is `"5%"`, a missing argument is `""`. Same for
  `formatText`.
- `parseSimpleArray` returns what it parsed before the error and logs
  `parseSimpleArray format error`: `"[1, b]"` is `[1]`, `"[1, 2"` is `[1,2]`, `"1"` is `[]`.
  A trailing comma before `]` is accepted. High confidence.
- `str` of a namespace is `Namespace` (every namespace), of structured text the markup with
  every tag dropped (`<br/>` included: `str lineBreak` is `""`). High confidence.


## Scheduler

Wiki "Scheduler": scheduled scripts get 3 ms per frame in total (50 ms on a loading screen).
The script that has waited longest runs first. A spawned script starts on the next frame.
a3-sqf uses `DEFAULT_FRAME_BUDGET = 3 ms`. Not yet confirmed in the binary.

## Open points

- The commands that opt in to nil arguments: we use "the overload's argument types include
  ANY".
- What sets the evaluation-state flag (`+0x1c`) that suppresses VAR errors.
- `0 = expr` and assignment to command names (`pixelGrid = 16`): they compile in the shipped
  scripts; the runtime effect is unverified.
