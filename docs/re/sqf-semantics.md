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

## Variables and nil

- **Reading a variable** (`GameInstructionVariable::Execute`, `FUN_1402feef0`): when the
  variable is undefined or holds nil, the VM raises VAR ("Undefined variable in expression:
  _x") at the read, for locals and globals alike. Two flags suppress it: one on the evaluation
  state (`+0x1c`) and one on the VM context (`+0x4c4`). The context flag is what `isNil {...}`
  sets, so the code it evaluates may read undefined variables. High confidence for the check;
  medium for which commands set the flags.
- **Commands with nil arguments** (`GameInstructionOperator::Execute`, `FUN_1402fe7d0`, same
  for unary): for the first overload whose types match (nil matches every type), the handler
  is called only if the command accepts nil (an opt-in check through a vtable call). Otherwise
  the command is skipped and its result is a nil of the overload's return type. There is no
  error: `nil + 1` is nil. `str`, `typeName`, `isNil`, `isEqualTo` and similar take nil. High
  confidence.
- `typeName nil` is `"ANY"` (the nil type is the ANY set at `0x142162480`). `str nil` is
  `"any"` (`GameDataNil` to-string, `FUN_1402d9bf0`). High confidence.
- `str` of a NOTHING value is `"nothing"` (`FUN_1402da200`); `typeName` gives `"NOTHING"`. A
  value with no data at all prints `"<null>"`. High confidence.

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
  the size gives nil with no error. Anything further out raises DIM with (size, index + 1) and
  returns nil: `[1,2] select 3` → "2 elements provided, 4 expected". High confidence. The
  wiki's "Zero divisor" for out-of-range `select` describes older builds.
- `array select [start, count]` (`FUN_1402e4b80`): both values are rounded the same way. A
  start outside `0..size-1` gives `[]` with no error. A count < 1 gives `[]`. The range is
  clipped to the end. High confidence.

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
