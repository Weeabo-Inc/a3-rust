# SQF script command table (arma3_x64.exe 2.22.0.154103)

`docs/re/sqf-commands.tsv` holds every script command overload the engine registers: **3,171
overloads, 2,649 distinct names** (311 nular, 1,457 unary, 1,403 binary; 1,338 distinct unary
names and 1,191 distinct binary names). Regenerate:

```sh
python tools/re/sqf_commands.py P:/a3-rust/oirignal/arma3_x64.exe --tsv docs/re/sqf-commands.tsv
python tools/re/sqf_commands.py ... --long      # adds description / example / example_result / changed
```

The descriptions, examples and `since` versions are the engine's own built-in help texts. Query:
`python tools/re/a3re.py sqf '^select$'`. Addresses are RVAs (+0x140000000 for VA).

A cross-check against the arma-wiki database was not possible (its Postgres was down during
this work); the counts above are from the binary only. Follow-up: compare names and signatures
with the wiki and list commands that exist only on one side.

## How commands are registered

Registration is code, not a static table: large static-initialiser functions build descriptor
objects on the stack and pass them to three constructors (one per form). Example: the function at
RVA 0x4bd410 (0x57,000 instructions) registers most `General` unary commands.

| Form | Constructor (RVA) | Register arguments | Stack arguments (`[rsp+N]`) |
|---|---|---|---|
| nular | `0x2c86c0` | rcx = this, rdx = return type, r8 = name, r9 = handler | 0x20 description, 0x28 example, 0x30 example result, 0x38 since, 0x40 changed, 0x48 category |
| unary | `0x2c8050` | rcx = this, rdx = return type, r8 = name, r9 = handler | 0x20 right type, 0x28 right-arg name, 0x30 description, 0x38 example, 0x40 example result, 0x48 since, 0x50 changed, 0x58 category, 0x60 0 |
| binary | `0x2c8a20` | rcx = this, rdx = return type, r8 = name, r9d = **priority** | 0x20 handler, 0x28 left type, 0x30 right type, 0x38 left-arg name, 0x40 right-arg name, 0x48 description, 0x50 example, 0x58 example result, 0x60 since, 0x68 changed, 0x70 category, 0x78 0 |

All strings are `const char*` (UTF-8). Types are pointers to static `GameType` objects.
Confidence: high for name/handler/types/priority (cross-checked on `setDamage`, `select`, `+`,
`count`, `if`/`then`, `diag_tickTime`); medium for the exact meaning of the `changed` slot.

### Handler signatures (from decompiled handlers; medium confidence)

- nular: `GameValue* handler(GameValue* result, const GameState* state)`
- unary: `GameValue* handler(GameValue* result, const GameState* state, const GameValue& right)`
- binary: `GameValue* handler(GameValue* result, const GameState* state, const GameValue& left, const GameValue& right)`

Example: `diag_tickTime` (handler 0x8a6fc0, renamed `SQF_diag_tickTime` in the shared project)
returns `(float)timeGetTimeMs() * 0.001` via the scalar GameValue constructor 0x2c92e0. RTTI
lambda names leak further handler names, e.g. `GetConfigClasses`, `GetConfigProperties`,
`ListApply`, `ListCountCond`, `ListFindCond`.

## Types

35 basic types are registered by `GameType::GameType(this, name, createFn, localisationKey, ...)`
at `0x2c9060` (e.g. RVA 0x181b0 registers `SCALAR` with `@STR_EVAL_TYPESCALAR`, `"Number"`,
`"A real number."`). Commands reference `GameType` *sets*: copies (`0x2c9030`) and unions built
with `GameType::operator|` at `0x2cbf20` (rcx = left, rdx = result, r8 = right).

| Basic type RVA | Name | | Basic type RVA | Name |
|---|---|---|---|---|
| 0x2162260 | SCALAR | | 0x216ba20 | SIDE |
| 0x21622c0 | BOOL | | 0x216ba60 | VECTOR |
| 0x2162320 | ARRAY | | 0x216baa0 | SCRIPT |
| 0x2162380 | STRING | | 0x216bae0 | TEXT |
| 0x21623e0 | NOTHING | | 0x216bd30 | CONTROL |
| 0x2162440 | ANY | | 0x216be00 | ORIENT |
| 0x21624a0 | NAMESPACE | | 0x216c180 | TEAM_MEMBER |
| 0x2162500 | NaN | | 0x216c300 | GROUP |
| 0x2162560 | IF | | 0x216c340 | TRANS |
| 0x21625c0 | WHILE | | 0x216c430 | SUBGROUP |
| 0x2162620 | FOR | | 0x216c4c0 | OBJECT |
| 0x2162680 | SWITCH | | 0x216c540 | DISPLAY |
| 0x21626e0 | EXCEPTION | | 0x216c640 | CONFIG |
| 0x2162740 | WITH | | 0x216c6a0 | NetObject |
| 0x21627a0 | CODE | | 0x219fa90 | HASHMAP |
| 0x2169560 | EXPRESSION | | 0x21d1f30 | LOCATION |
| 0x216b950 | TARGET | | 0x21d3790 | TASK |
| | | | 0x21d37f0 | DIARY_RECORD |

In the TSV, union types print as `A|B` (`SCALAR|NaN` is how plain numbers are accepted). 20
overloads keep a `?` part where the union was built through a temporary the extractor does not
follow (e.g. `HASHMAP get ?|NaN`); resolve those by hand with `a3re.py decompile <site>`.
`EXPRESSION` commands (category `Simple expression`, 46) belong to the config "simple expression"
evaluator, not to SQF proper.

## Binary operator priorities (r9d)

| Priority | Count | Operators |
|---|---|---|
| 1 | 4 | `or`, `\|\|` |
| 2 | 4 | `and`, `&&` |
| 3 | 24 | `== != < > <= >=`, and the config path operators `>>` and `/` (CONFIG / STRING) |
| 4 | 1,335 | every named binary command (`setDamage`, `select`, `then`, ...), `:` (switch case), and some `==`/`!=` overloads |
| 5 | 1 | `else` |
| 6 | 19 | `+`, `-`, `max`, `min` |
| 7 | 11 | `*`, `/` (numbers), `%`, `mod`, `atan2` |
| 9 | 4 | `^`, `pow` |
| 10 | 1 | `#` |

Higher binds tighter. Priority is stored per overload; `==`/`!=` overloads registered with 4
instead of 3 need a check against the parser (does it use the priority of the first overload
found by name?) — follow-up.

## TSV columns

`name, kind (nular|unary|binary), left_type, right_type, return_type, handler (RVA), priority
(binary only), category, since, left_name, right_name, site` — `site` is the RVA of the
constructor call, useful to read the surrounding registration code.

Categories as registered: General 2,729; Default 155; Editor 58; Identity 58; Location 47;
Simple expression 46; Agents 39; Server 17; Visual 16; Conversations 2; Debug 2; Location PC 2.
