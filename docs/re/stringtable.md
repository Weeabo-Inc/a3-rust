# Stringtables and localization

Implemented in `crates/a3-stringtable`. Sources: `arma3_x64.exe` (build 2.22.0.154103; addresses
are VAs in the Ghidra project) and a survey of the install. Confidence: **high** unless marked.

## Survey

- 54 `stringtable.xml` files in the VFS (`a3\language_f*`, `a3\3den_language`, ...), 43,243 keys,
  no key defined twice (ignoring case), no byte order marks, all well-formed XML.
- Every key has all 20 language elements, `Original` first: Original, English, Czech, German,
  Russian, Polish, Hungarian, Italian, Spanish, French, Chinese, Japanese, Korean, Portuguese,
  Chinesesimp, Turkish, Latin, Bulgarian, Slovak, Ukrainian.
- No `stringtable.bin` and no `stringtable.csv` ship in the vanilla PBOs (creator DLC EBOs not
  inspected).
- 4,439 texts have leading or trailing whitespace, some contain CR LF line breaks; kept as is.

## File kinds

The engine knows three names (strings at `0x141a77e78`): `stringtable.csv`, `stringtable.xml`,
`stringtable.bin`; the loader dispatches on the kind (`0x1403388e0`: 0 = CSV, 1 = XML, 2 = bin).

### stringtable.xml

Parser classes `StringtableParserXMLBase` / `StringtableParser` (vtable `0x141aa6230`).

- An element named `Key` or `Text` (exact case) with an `ID` attribute starts a key; the nesting
  around it (`Project`, `Package`, `Container`) does not matter.
- Each child element is a language, named by its tag. Entities are decoded.
- Choice of text per key (`0x14033c2e0`, run at the end of each language element; quality
  levels 4..1):
  1. the selected language;
  2. else `Original`;
  3. else `English`;
  4. else the first language element of the key.
  `Original` is preferred to `English` whichever comes first in the file.
- A key element nested inside a language element logs `Unexpected stringtable format inside
  <Text ID="%s"><%s>`.
- Tag comparison for the language names uses the engine's string compare (`0x14128c730`),
  _medium_: assumed case-insensitive, as `a3-stringtable` does.

### stringtable.bin (`BLMX`)

Reader `0x1403389c0` (all little-endian, strings NUL-terminated):

```
char[4]  "BLMX"                          else "Invalid file type of binarized XML."
u32      language_count, asciiz languages[language_count]
u32      offset_count,   i32 offsets[offset_count]   == language_count, else
                                          "mismatch of number of languages and offsets."
u32      key_count,      asciiz keys[key_count]       0 keys: "stringtable is empty?"
at offsets[i]:  u32 count, asciiz texts[count]        == key_count, else
                                          "mismatch of number of keys and translations."
```

The engine reads only one column: the selected language, else `Original`, else `English`, else
the first. The offsets are absolute file positions (_medium_: the reader seeks the stream to the
value). `a3-stringtable` reads every column and writes the same layout (`Stringtable::to_bin`,
columns after the key list).

### stringtable.csv

Legacy CSV with a `LANGUAGE` header row and `COMMENT` rows (`0x140339ad0`); an unknown language
logs `Unsupported language %s in stringtable` and uses the first column. Not implemented.

## Registration and lookup

- Keys go into a hash table with case-folded hashing and `_strnicmp` comparison
  (`0x1403374d0`): keys are case-insensitive. A key registered again logs `Item %s listed twice`
  and the **first** text is kept.
- The order in which addon stringtables are registered follows config loading
  (`LoadConfigStringtables`), _low_: `a3-stringtable` uses VFS path order. Vanilla has no duplicate
  keys, so the order does not matter for it.
- Lookup (`0x140339890`) searches a list of tables in order (the global one and, presumably,
  mission/campaign tables) and returns the first match; a miss logs `String %.*s not found` and
  returns an empty string.

### `localize` (handler `0x14055d440`)

1. Empty argument: returns it.
2. Starting with `@`: first looked up in a separate table (`0x14033b080`, unknown purpose); if
   that gives a non-empty text, returns it.
3. Starting with `$` followed by `STR` (any case): the `$` is dropped.
4. The (remaining) string is looked up as a key; missing: empty string.

Any string works as a key, not only `STR_...` ones.

### Config strings

Config values written `$STR_...` are localized; `a3-stringtable`'s `Localizer::config_text`
resolves them through the same lookup (empty when missing). _Medium_: where the engine does this
(at config load or at use) is not traced.

## Open questions

- The `@` table, and which tables besides the global one the lookup searches (mission
  `stringtable.xml`, campaign).
- Whether the engine preprocesses `stringtable.xml` (`Stringtable %s: preprocessor error`).
