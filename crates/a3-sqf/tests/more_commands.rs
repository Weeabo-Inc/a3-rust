//! Structured text, JSON, hash map objects, toFixed and other
//! world-independent commands (examples from the community wiki).

mod common;

use common::{err, s, vm};

#[test]
fn structured_text() {
    assert_eq!(s("typeName parseText \"<t>a</t>\""), "\"TEXT\"");
    assert_eq!(
        s("composeText [lineBreak] isEqualTo parseText \"<br/>\""),
        "true"
    );
    assert_eq!(
        s(
            "composeText [\"Image: \", image \"\\a\\b.paa\"] isEqualTo parseText \"Image: <img image='\\a\\b.paa'/>\""
        ),
        "true"
    );
    assert_eq!(
        s("composeText [\"line1\", lineBreak, \"line2\"] == parseText \"line1<br/>line2\""),
        "true"
    );
    assert_eq!(
        s("str parseText \"<t color='#ff0000'>Red</t> &amp; blue\""),
        "\"Red & blue\""
    );
    assert_eq!(s("str text \"<b>\""), "\"<b>\"");
    assert_eq!(
        s("str formatText [\"%1 and %2\", \"a<\", parseText \"<t>b</t>\"]"),
        "\"a< and b\""
    );
}

#[test]
fn json() {
    assert_eq!(
        s("toJSON [\"this\", \"is\", \"an\", \"array\"]"),
        r#""[""this"",""is"",""an"",""array""]""#
    );
    assert_eq!(
        s("toJSON [1, 2.5, true, nil, objNull, [\"q\"\"\"]]"),
        r#""[1,2.5,true,null,null,[""q\""""]]""#
    );
    assert_eq!(
        s("_m = createHashMap; _m set [\"b\", true]; _m set [1, 2]; toJSON _m"),
        r#""{""b"":true}""#
    );
    assert_eq!(s("toJSON objNull"), "\"\"");
    assert_eq!(s("fromJSON \"42\""), "42");
    assert_eq!(s("fromJSON \"\"\"Hello there\"\"\""), "\"Hello there\"");
    assert_eq!(
        s("fromJSON \"[42, \"\"Hello there\"\", true, null]\""),
        "[42,\"Hello there\",true,any]"
    );
    assert_eq!(
        s(
            "(fromJSON \"{\"\"key1\"\": \"\"value1\"\", \"\"n\"\": {\"\"x\"\": 1}}\") get \"n\" get \"x\""
        ),
        "1"
    );
}

#[test]
fn to_fixed_lasts_until_the_script_ends() {
    assert_eq!(s("1 toFixed 2"), "\"1.00\"");
    assert_eq!(
        s(
            "_r = []; call { toFixed 2; _r pushBack str 1.5; call { _r pushBack str pi } }; _r pushBack str 1.5; _r"
        ),
        // The setting belongs to the script, not the scope (server oracle).
        "[\"1.50\",\"3.14\",\"1.50\"]"
    );
    assert_eq!(s("toFixed 1; toFixed -1; str 0.25"), "\"0.25\"");
}

#[test]
fn hash_map_objects() {
    let mut vm = vm();
    let v = vm
        .eval(
            "private _decl = [
                [\"#type\", \"Counter\"],
                [\"#create\", { _self set [\"n\", _this] }],
                [\"Add\", { _self set [\"n\", (_self get \"n\") + _this]; _self get \"n\" }],
                [\"#str\", { format [\"Counter(%1)\", _self get \"n\"] }]
            ];
            private _c = createHashMapObject [_decl, 10];
            private _r = [_c call [\"Add\", 5], str _c, _c get \"#type\"];
            private _copy = +_c;
            _copy call [\"Add\", 1];
            _r pushBack (_c get \"n\");
            _r pushBack (_copy get \"n\");
            _r",
        )
        .unwrap();
    assert_eq!(
        v.to_sqf_string(),
        "[15,\"Counter(15)\",[\"Counter\"],15,16]"
    );
}

#[test]
fn hash_map_object_inheritance_and_flags() {
    let mut vm = vm();
    let v = vm
        .eval(
            "log1 = [];
            private _animal = [[\"#type\", \"IAnimal\"], [\"#create\", { log1 pushBack \"animal\" }], [\"Sound\", { \"...\" }]];
            private _pig = [[\"#base\", _animal], [\"#type\", \"Pig\"], [\"#create\", { log1 pushBack \"pig\" }], [\"Sound\", { \"oink\" }]];
            private _p = createHashMapObject [_pig];
            [log1, _p get \"#type\", _p call [\"Sound\"], \"IAnimal\" in (_p get \"#type\")]",
        )
        .unwrap();
    assert_eq!(
        v.to_sqf_string(),
        "[[\"animal\",\"pig\"],[\"IAnimal\",\"Pig\"],\"oink\",true]"
    );
    assert!(
        err("private _o = createHashMapObject [[[\"#flags\", [\"sealed\"]], [\"a\", 1]]]; _o set [\"b\", 2]")
            .contains("Tried to add key to sealed HashMap")
    );
    assert_eq!(
        s(
            "private _o = createHashMapObject [[[\"#flags\", [\"Sealed\"]], [\"a\", 1]]]; _o set [\"a\", 2]; _o get \"a\""
        ),
        "2"
    );
    assert!(
        err("private _o = createHashMapObject [[[\"#flags\", [\"noCopy\"]]]]; +_o")
            .contains("noCopy")
    );
}

#[test]
fn strings_and_arrays() {
    assert_eq!(s("\"Test\" insert [0, \"Radio\"]"), "\"RadioTest\"");
    assert_eq!(s("\"Test\" insert [2, \"Radio\"]"), "\"TeRadiost\"");
    assert_eq!(s("\"Test\" insert [-1, \"Radio\"]"), "\"TestRadio\"");
    assert_eq!(s("reverse \"abc\""), "\"cba\"");
    assert_eq!(
        s("_a = [\"a\", \"b\", \"c\"]; _a insert [-2, [\"w\"]]; _a"),
        "[\"a\",\"b\",\"w\",\"c\"]"
    );
    assert_eq!(
        s("_a = [\"a\", \"b\", \"c\"]; _a insert [-1, [\"w\"]]; _a"),
        "[\"a\",\"b\",\"c\",\"w\"]"
    );
    assert_eq!(
        s("([\"cow\", \"cat\"] createHashMapFromArray [1, 2]) toArray false"),
        "[[\"cow\",1],[\"cat\",2]]"
    );
    assert_eq!(
        s(
            "_m = createHashMap; _m insert [true, [[\"a\", \"b\"], [1, 2]]]; [_m get \"a\", _m get \"b\"]"
        ),
        "[1,2]"
    );
}

#[test]
fn try_with_arguments_and_misc() {
    assert_eq!(s("5 try { _this * 2 } catch { 0 }"), "10");
    assert_eq!(
        s("x2 = 1; [missionNamespace isNil \"x2\", missionNamespace isNil \"nope\"]"),
        "[false,true]"
    );
    assert_eq!(s("requiredVersion \"2.06\""), "true");
    assert_eq!(s("requiredVersion \"9.0\""), "false");
    assert_eq!(s("0 spawn {}; diag_scope"), "0");
    assert_eq!(s("call { call { diag_scope } }"), "2");
    assert_eq!(s("call { isNil { call { x3 = diag_scope } }; x3 }"), "1");
    assert_eq!(
        s("private _v = 1; call { scopeName \"inner\"; count diag_stacktrace }"),
        "2"
    );
    assert_eq!(
        s("private _v = 7; call { (diag_stacktrace select 0 select 3) get \"_v\" }"),
        "7"
    );
}
