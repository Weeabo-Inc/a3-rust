//! Hash maps, regex, vectors and other world-independent commands.

mod common;

use common::{err, eval, s, vm};

#[test]
fn hash_map_basics() {
    assert_eq!(
        s("_m = createHashMap; _m set [\"a\", 1]; _m get \"a\""),
        "1"
    );
    assert_eq!(
        s("_m = createHashMap; [_m set [1, 2], _m set [1, 3], _m get 1]"),
        "[false,true,3]"
    );
    assert_eq!(
        s("_m = createHashMapFromArray [[\"a\", 1], [\"b\", 2]]; count _m"),
        "2"
    );
    assert_eq!(
        s("_m = [[1, 2], [\"x\", \"y\"]] createHashMapFromArray []; count _m"),
        "2"
    );
    assert_eq!(s("_m = createHashMap; _m get \"missing\""), "any");
    assert_eq!(s("_m = createHashMap; _m getOrDefault [\"k\", 5]"), "5");
    assert_eq!(
        s("_m = createHashMap; _m getOrDefault [\"k\", 5, true]; _m get \"k\""),
        "5"
    );
    assert_eq!(
        s("_m = createHashMapFromArray [[1, 10]]; [_m deleteAt 1, count _m]"),
        "[10,0]"
    );
    assert_eq!(
        s("_m = createHashMapFromArray [[1, 10]]; [1 in _m, 2 in _m]"),
        "[true,false]"
    );
    assert_eq!(
        s("_m = createHashMapFromArray [[\"A\", 1]]; \"a\" in _m"),
        "false"
    );
    assert_eq!(s("toArray createHashMapFromArray [[1, 2]]"), "[[1],[2]]");
    assert_eq!(s("keys createHashMapFromArray [[1, 2], [3, 4]]"), "[1,3]");
    assert_eq!(s("values createHashMapFromArray [[1, 2], [3, 4]]"), "[2,4]");
}

#[test]
fn hash_map_get_or_default_call_runs_code_only_when_missing() {
    assert_eq!(
        s(
            "_n = 0; _m = createHashMapFromArray [[1, 2]]; _m getOrDefaultCall [1, { _n = _n + 1; 9 }]; _n"
        ),
        "0"
    );
    assert_eq!(
        s("_m = createHashMap; _m getOrDefaultCall [1, { 9 }, true]; _m get 1"),
        "9"
    );
}

#[test]
fn hash_map_iteration_and_merge() {
    assert_eq!(
        s("_r = 0; { _r = _r + _x * _y } forEach createHashMapFromArray [[2, 3], [4, 5]]; _r"),
        "26"
    );
    assert_eq!(
        s(
            "_a = createHashMapFromArray [[1, 1]]; _a merge createHashMapFromArray [[1, 9], [2, 2]]; [_a get 1, _a get 2]"
        ),
        "[1,2]"
    );
    assert_eq!(
        s(
            "_a = createHashMapFromArray [[1, 1]]; _a merge [createHashMapFromArray [[1, 9]], true]; _a get 1"
        ),
        "9"
    );
}

#[test]
fn hash_map_array_keys_are_snapshots() {
    assert_eq!(
        s("_k = [1]; _m = createHashMap; _m set [_k, \"v\"]; _k pushBack 2; _m get [1]"),
        "\"v\""
    );
}

#[test]
fn hash_map_rejects_bad_keys_and_read_only_edits() {
    assert!(err("createHashMap set [objNull, 1]").contains("Type Object"));
    assert!(err("_m = compileFinal createHashMap; _m set [1, 1]").contains("Read-only"));
}

#[test]
fn hash_map_plus_copies() {
    assert_eq!(
        s("_a = createHashMap; _b = +_a; _b set [1, 1]; count _a"),
        "0"
    );
}

#[test]
fn regex_commands() {
    assert_eq!(
        s("\"I'm a Cookie clicker\" regexMatch \".*cookie.*/i\""),
        "true"
    );
    assert_eq!(
        s("\"I'm a Cookie clicker\" regexMatch \".*cookie.*\""),
        "false"
    );
    assert_eq!(
        s("\"wookie boOkie cookie\" regexReplace [\".oo/i\", \"[$&]\"]"),
        "\"[woo]kie [boO]kie [coo]kie\""
    );
    assert_eq!(
        s("\"wookie boOkie cookie\" regexFind [\".ookie\"]"),
        "[[[\"wookie\",0]],[[\"cookie\",14]]]"
    );
    assert_eq!(
        s("\"a1b22\" regexReplace [\"(\\d+)\", \"<$1>\"]"),
        "\"a<1>b<22>\""
    );
    assert_eq!(
        s("\"ab ab\" regexFind [\"(a)(b)\", 1]"),
        "[[[\"ab\",3],[\"a\",3],[\"b\",4]]]"
    );
}

#[test]
fn vector_commands() {
    assert_eq!(s("[1, 2, 3] vectorAdd [1, 1, 1]"), "[2,3,4]");
    assert_eq!(s("[1, 2, 3] vectorDiff [1, 1, 1]"), "[0,1,2]");
    assert_eq!(s("[1, 2, 3] vectorMultiply 2"), "[2,4,6]");
    assert_eq!(s("[1, 0, 0] vectorCrossProduct [0, 1, 0]"), "[0,0,1]");
    assert_eq!(s("[1, 2, 3] vectorDotProduct [4, 5, 6]"), "32");
    assert_eq!(s("vectorMagnitude [3, 4, 0]"), "5");
    assert_eq!(s("vectorNormalized [0, 0, 5]"), "[0,0,1]");
    assert_eq!(s("[0, 0, 0] vectorDistance [3, 4, 0]"), "5");
    assert_eq!(
        s("vectorLinearConversion [0, 10, 5, [0, 0, 0], [10, 20, 30], true]"),
        "[5,10,15]"
    );
}

#[test]
fn random_forms() {
    let mut vm = vm();
    for _ in 0..100 {
        let v = vm.eval("random 10").unwrap().as_number().unwrap();
        assert!((0.0..10.0).contains(&v));
        let g = vm.eval("random [0, 5, 10]").unwrap().as_number().unwrap();
        assert!((0.0..=10.0).contains(&g));
    }
    assert_eq!(
        vm.eval("(42 random 1) == (42 random 1)")
            .unwrap()
            .to_sqf_string(),
        "true"
    );
}

#[test]
fn parse_simple_array() {
    assert_eq!(
        s("parseSimpleArray \"[1, -2.5, \"\"a\"\", true, [false, []]]\""),
        "[1,-2.5,\"a\",true,[false,[]]]"
    );
    assert_eq!(s("parseSimpleArray \"[1 + 1]\""), "[]");
}

#[test]
fn select_max_min_and_weighted() {
    assert_eq!(s("selectMax [1, 5, 3]"), "5");
    assert_eq!(s("selectMin [4, 2, 8]"), "2");
    assert_eq!(s("[\"a\", \"b\"] selectRandomWeighted [0, 1]"), "\"b\"");
    assert_eq!(s("selectRandomWeighted [\"a\", 1, \"b\", 0]"), "\"a\"");
}

#[test]
fn compile_script_uses_the_host_loader() {
    let mut vm = vm();
    vm.host.files.insert("fn.sqf".into(), "_this * 3".into());
    assert_eq!(
        vm.eval("4 call compileScript [\"fn.sqf\"]")
            .unwrap()
            .to_sqf_string(),
        "12"
    );
}

#[test]
fn sample_script_runs() {
    let src = include_str!("fixtures/sample.sqf");
    let mut vm = vm();
    let code = vm.compile_file("sample.sqf", src).unwrap();
    let contacts = vm
        .eval(
            "[[\"a\", \"WEST\", 100], [\"b\", \"east\", 300, [\"hvt\"]], [\"c\", \"guer\", 5000]]",
        )
        .unwrap();
    let out = vm.call(&code, Some(eval("[]"))).unwrap();
    assert_eq!(vm.eval("1").unwrap().to_sqf_string(), "1");
    assert_eq!(out.ty().type_name(), "HASHMAP");
    let out = vm
        .call(&code, Some(a3_sqf::Value::array([contacts])))
        .unwrap();
    let a3_sqf::Value::HashMap(m) = out else {
        panic!("not a hashmap")
    };
    let get = |k: &str| {
        m.borrow()
            .get(&a3_sqf::HashKey::String(k.into()))
            .map(|v| v.to_sqf_string())
            .unwrap_or_default()
    };
    assert_eq!(get("hvt"), "\"b\"");
    assert_eq!(get("average"), "1800");
    assert_eq!(get("label"), "\"summary (3 contacts)\"");
    assert!(vm.host.errors.is_empty(), "{:?}", vm.host.errors);
}
