//! `#define` / `#undef` and macro expansion.
//!
//! Expected outputs marked "engine" are results observed in the original game, collected in
//! <https://github.com/Krzmbrzl/ArmaPreprocessorTestCases>, or documented on the BI community wiki
//! page "PreProcessor Commands".

mod common;
use a3_preproc::ErrorKind;
use common::{pp, try_pp};

#[test]
fn object_like_macro_is_replaced() {
    assert_eq!(pp("#define MACRO test\nMACRO"), "\ntest");
}

#[test]
fn spaces_between_name_and_body_are_swallowed() {
    // engine: Test37, Test38
    assert_eq!(pp("#define MACRO                   test\nMACRO"), "\ntest");
}

#[test]
fn tab_between_name_and_body_is_kept() {
    // engine: Test39
    assert_eq!(pp("#define MACRO\ttest\nMACRO"), "\n\ttest");
}

#[test]
fn body_may_follow_name_without_space() {
    // engine: Test40 / wiki
    assert_eq!(pp("#define MACRO#test\nMACRO"), "\n\"test\"");
}

#[test]
fn backslash_inside_body_is_literal() {
    // engine: BackslashInMacroBody
    assert_eq!(
        pp("#define MACRO This is \\ great\nMACRO"),
        "\nThis is \\ great"
    );
}

#[test]
fn backslash_followed_by_space_does_not_continue() {
    // engine: BodyLineEndsWithBackslashSpace
    assert_eq!(
        pp("#define MACRO This is \\ \ngreat\nMACRO"),
        "\ngreat\nThis is \\ "
    );
}

#[test]
fn continuation_joins_lines_and_keeps_line_count() {
    // engine: Test01
    let src = "#define MY_MACRO Hello macro\n\n#define OTHER_MACRO More than \\\none line!\n\t\t\t\t\n#define MACRO_WITH_ARGUMENTS(ARG) This is ARG\n\nMACRO_WITH_ARGUMENTS(MY_MACRO)\n\nOTHER_MACRO";
    assert_eq!(
        pp(src),
        "\n\n\n\n\n\n\nThis is Hello macro\n\nMore than one line!"
    );
}

#[test]
fn continuation_keeps_indentation_of_next_line() {
    // engine: Test02 (comment header shortened)
    let src = "/**\n * x\n */\n\n#define BLA There is \\\n\t\tSo much\t\tI can\\\ntalk about    \\\nright here\n\nBLA";
    assert_eq!(
        pp(src),
        "\n\n\n\n\n\n\n\n\nThere is \t\tSo much\t\tI cantalk about    right here"
    );
}

#[test]
fn line_comment_ends_define_even_before_backslash() {
    // engine: Test22
    assert_eq!(
        pp("#define BLA Something // I am a comment\\\nis wrong\nBLA"),
        "\nis wrong\nSomething "
    );
}

#[test]
fn block_comment_before_backslash_is_removed() {
    // engine: Test23
    assert_eq!(
        pp("#define BLA Something /* I am a comment*/\\\nis wrong\nBLA"),
        "\n\nSomething is wrong"
    );
}

#[test]
fn multi_line_block_comment_continues_define() {
    // engine: Test24
    assert_eq!(
        pp("#define BLA Something /* I am\na mutline\ncomment */\\\nis wrong\nBLA"),
        "\n\n\n\nSomething is wrong"
    );
}

#[test]
fn double_quoted_string_is_not_expanded() {
    // engine: DoubleQuoteExpansion
    assert_eq!(
        pp("#define MACRO Hello\n#define OTHER \"MACRO\"\nOTHER"),
        "\n\n\"MACRO\""
    );
}

#[test]
fn single_quoted_string_is_expanded() {
    // engine: SingleQuoteExpansion / wiki
    assert_eq!(
        pp("#define MACRO Hello\n#define OTHER 'MACRO'\nOTHER"),
        "\n\n'Hello'"
    );
    assert_eq!(
        pp("#define ARG world\nsystemChat 'hello ARG';\nsystemChat \"hello ARG\";"),
        "\nsystemChat 'hello world';\nsystemChat \"hello ARG\";"
    );
}

#[test]
fn only_whole_words_are_replaced() {
    assert_eq!(pp("#define A x\nA AB _A A_ 1A A.A"), "\nx AB _A A_ 1A x.x");
}

#[test]
fn undef_removes_macro() {
    assert_eq!(pp("#define A x\nA\n#undef A\nA"), "\nx\n\nA");
}

#[test]
fn redefinition_replaces_body() {
    assert_eq!(pp("#define A x\n#define A y\nA"), "\n\ny");
}

#[test]
fn nested_object_like_macros_expand() {
    assert_eq!(pp("#define A B+1\n#define B 2\nA"), "\n\n2+1");
}

#[test]
fn self_reference_does_not_recurse() {
    assert_eq!(pp("#define X X+1\nX"), "\nX+1");
    assert_eq!(pp("#define A B\n#define B A\nA B"), "\n\nA B");
}

#[test]
fn empty_define_expands_to_nothing() {
    assert_eq!(pp("#define EMPTY\n[EMPTY]"), "\n[]");
}

#[test]
fn define_without_name_is_an_error() {
    let err = try_pp("#define 1abc x").unwrap_err();
    assert!(matches!(
        err.kind,
        ErrorKind::MalformedDirective {
            directive: "define",
            ..
        }
    ));
}

// --- function-like macros --------------------------------------------------------------------

#[test]
fn function_like_macro_substitutes_arguments() {
    assert_eq!(
        pp("#define BLASTOFF(UNIT,RATE) UNIT setVelocity [0,0,RATE];\nBLASTOFF(player,10)"),
        "\nplayer setVelocity [0,0,10];"
    );
}

#[test]
fn parameter_names_are_trimmed() {
    // engine: ErrorTest07, ErrorTest08
    assert_eq!(
        pp("#define MACRO(ARG ) This is ARG\n\nMACRO(Sparta)"),
        "\n\nThis is Sparta"
    );
    assert_eq!(
        pp("#define MACRO( ARG) This is ARG\n\nMACRO(Sparta)"),
        "\n\nThis is Sparta"
    );
}

#[test]
fn arguments_are_not_trimmed() {
    // engine: Test41, Test42
    assert_eq!(pp("#define MACRO(arg) arg\nMACRO( test )"), "\n test ");
    assert_eq!(
        pp("#define MACRO(arg) arg\nMACRO(      test)"),
        "\n      test"
    );
}

#[test]
fn parameters_replace_whole_words_only() {
    // engine: Test12
    let src = "#define MACRO(ARG) This is ARG.test test.ARG test.ARG.test ARG/test test/ARG test/ARG/test test\\ARG ARG\\test test\\ARG\\test someARG ARG_test test_ARG ARG-test test-ARG\n\nMACRO(X)";
    assert_eq!(
        pp(src),
        "\n\nThis is X.test test.X test.X.test X/test test/X test/X/test test\\X X\\test test\\X\\test someARG ARG_test test_ARG X-test test-X"
    );
}

#[test]
fn macro_call_in_body_expands() {
    // engine: Test16
    assert_eq!(
        pp(
            "#define MACRO(ARG) This is ARG\n#define OTHER(A) MACRO(A) and it is cool\n\nOTHER(Arma)"
        ),
        "\n\n\nThis is Arma and it is cool"
    );
}

#[test]
fn function_like_macro_without_parentheses_is_left_alone() {
    // engine: Test17
    assert_eq!(pp("#define MACRO(A) This is A\nMACRO"), "\nMACRO");
}

#[test]
fn object_like_macro_followed_by_parentheses() {
    // engine: Test18
    assert_eq!(
        pp("#define MACRO Test here\nMACRO(Tester)"),
        "\nTest here(Tester)"
    );
}

#[test]
fn zero_parameter_macro() {
    // engine: Test19
    assert_eq!(pp("#define MACRO() test\nMACRO()"), "\ntest");
}

#[test]
fn parameter_shadows_macro_of_same_name() {
    // engine: Test26 / wiki
    assert_eq!(
        pp("#define ONE foo\n#define TWO(ONE) ONE\nTWO(bar)"),
        "\n\nbar"
    );
}

#[test]
fn brackets_and_braces_pass_through_arguments() {
    // engine: Test43
    assert_eq!(
        pp("#define MACRO(arg) arg\nMACRO(Some [random] {input} in here)"),
        "\nSome [random] {input} in here"
    );
}

#[test]
fn strings_pass_through_arguments() {
    // engine: Test44, Test46, Test47, Test48
    assert_eq!(pp("#define MACRO(arg) arg\nMACRO(\"Test\")"), "\n\"Test\"");
    assert_eq!(pp("#define MACRO(arg) arg\nMACRO('Test')"), "\n'Test'");
    assert_eq!(
        pp("#define MACRO(arg) arg\nMACRO(\"Some \"\"content\"\"\")"),
        "\n\"Some \"\"content\"\"\""
    );
    assert_eq!(
        pp("#define MACRO(arg) arg\nMACRO('Some ''content''')"),
        "\n'Some ''content'''"
    );
}

#[test]
fn comma_inside_string_argument_is_dropped() {
    // engine: Test45 / wiki ("probably a bug")
    assert_eq!(
        pp("#define MACRO(arg) arg\nMACRO(\"Some, content\")"),
        "\n\"Some content\""
    );
}

#[test]
fn comma_inside_nested_parentheses_is_dropped() {
    // engine: NestedParensInMacroArgument
    assert_eq!(
        pp("#define A(a) -a-\n#define B Test A(test(and, some more))\nB"),
        "\n\nTest -test(and some more)-"
    );
}

#[test]
fn brackets_do_not_protect_commas() {
    // wiki: HINTARG([1,2,3]) "won't even compile"
    let out = try_pp("#define HINTARG(ARG) hint str ARG\nHINTARG([1,2,3]);").unwrap();
    assert_eq!(out.warnings.len(), 1);
    let warning = &out.warnings[0];
    assert_eq!(
        warning.kind,
        ErrorKind::MacroArgCount {
            name: "HINTARG".into(),
            expected: 1,
            found: 3
        }
    );
    assert_eq!(warning.line, 2);
}

#[test]
fn wrong_argument_count_expands_to_nothing() {
    // engine: ErrorTest06 (`MACRO(Yeah)` for two parameters leaves an empty line, processing
    // continues). Shipped scripts such as Contact's fn_moveModule.sqf rely on this.
    let out = try_pp(
        "#define MACRO(one,two) This is one and two\n\nMACRO(Yeah)\n\nhint \"Me\";\n\nMACRO(Well,)",
    )
    .unwrap();
    assert_eq!(out.text, "\n\n\n\nhint \"Me\";\n\nThis is Well and ");
    assert_eq!(out.warnings.len(), 1);
}

#[test]
fn array_through_helper_macro() {
    // wiki workaround
    assert_eq!(
        pp("#define HINTARG(ARG) hint str ARG\n#define array1 [1,2,3]\nHINTARG(array1);"),
        "\n\nhint str [1,2,3];"
    );
}

#[test]
fn orphaned_closing_parenthesis_stays() {
    // engine: OrphanedClosingParen
    assert_eq!(
        pp("#define A(a,b) -a--b-\n#define B Test A(test and, some more))\nB"),
        "\n\nTest -test and-- some more-)"
    );
}

#[test]
fn unterminated_call_is_an_error() {
    let err = try_pp("#define F(a) a\nF(1").unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnterminatedMacroCall("F".into()));
}

#[test]
fn call_may_span_lines() {
    assert_eq!(pp("#define F(a,b) a+b\nF(1,\n2)\nx"), "\n1+\n2\nx");
}

#[test]
fn arguments_are_expanded_before_substitution() {
    // wiki: GLUE(FOO,BAR) -> 123456
    assert_eq!(
        pp("#define GLUE(g1,g2) g1##g2\n#define FOO 123\n#define BAR 456\ntest = GLUE(FOO,BAR);"),
        "\n\n\ntest = 123456;"
    );
}

#[test]
fn cba_style_macros() {
    let src = "\
#define PREFIX ace
#define COMPONENT medical
#define DOUBLES(var1,var2) var1##_##var2
#define TRIPLES(var1,var2,var3) var1##_##var2##_##var3
#define QUOTE(var1) #var1
#define GVAR(var1) TRIPLES(PREFIX,COMPONENT,var1)
#define QGVAR(var1) QUOTE(GVAR(var1))
#define FUNC(var1) TRIPLES(DOUBLES(PREFIX,COMPONENT),fnc,var1)
GVAR(enabled) = true; x = QGVAR(enabled); [] call FUNC(init);";
    assert_eq!(
        pp(src).trim_start(),
        "ace_medical_enabled = true; x = \"ace_medical_enabled\"; [] call ace_medical_fnc_init;"
    );
}

// --- # and ## --------------------------------------------------------------------------------

#[test]
fn stringify_parameter() {
    // engine: Test11
    assert_eq!(
        pp(
            "#define STRINGIFY(ARG) #ARG\n#define OTHER #Something\n\nSTRINGIFY(Hello)\nSTRINGIFY(Hello there)\nSTRINGIFY(OTHER)"
        ),
        "\n\n\n\"Hello\"\n\"Hello there\"\n\"\"Something\"\""
    );
}

#[test]
fn stringify_expanded_argument() {
    // wiki: STRINGIFY(FOO) -> "123"
    assert_eq!(
        pp(
            "#define STRINGIFY(s) #s\n#define FOO 123\ntest1 = STRINGIFY(123); test2 = STRINGIFY(FOO);"
        ),
        "\n\ntest1 = \"123\"; test2 = \"123\";"
    );
}

#[test]
fn stringify_and_concat_mixed() {
    // engine: Test13
    assert_eq!(
        pp("#define MACRO(ARG) some#ARG ARG#some some##ARG ARG##some\n\nMACRO(Y)"),
        "\n\nsome\"Y\" Y\"some\" someY Ysome"
    );
}

#[test]
fn concat_in_object_like_macro() {
    // engine: Test14
    assert_eq!(
        pp("#define MACRO Some##Test\n\nMACRO\n\n#define OTHER(ARG) Some##Test\n\nOTHER(nothing)"),
        "\n\nSomeTest\n\n\n\nSomeTest"
    );
}

#[test]
fn triple_hash_concats_then_stringifies() {
    // engine: Test15, Test31, Test32, Test33
    assert_eq!(pp("#define MACRO Some###Test\nMACRO"), "\nSome\"Test\"");
    assert_eq!(pp("#define MACRO #Test\nMACRO"), "\n\"Test\"");
    assert_eq!(pp("#define MACRO ###\nMACRO"), "\n");
    assert_eq!(pp("#define MACRO ###test\nMACRO"), "\n\"test\"");
    assert_eq!(pp("#define MACRO #####test\nMACRO"), "\n\"test\"");
}

#[test]
fn double_hash_alone_vanishes() {
    // engine: Test30, Test34, Test35, Test36
    assert_eq!(pp("#define MACRO ##\nMACRO"), "\n");
    assert_eq!(pp("#define MACRO ######test\nMACRO"), "\ntest");
    assert_eq!(pp("#define MACRO##test\nMACRO"), "\ntest");
    assert_eq!(pp("#define MACRO  ## \nMACRO"), "\n ");
}

#[test]
fn hash_before_non_identifier_vanishes() {
    // engine: Attempted*Stringification, Test29
    assert_eq!(pp("#define MACRO Test #,\nMACRO"), "\nTest ,");
    assert_eq!(pp("#define MACRO Test #-!$\nMACRO"), "\nTest -!$");
    assert_eq!(pp("#define MACRO Test #33\nMACRO"), "\nTest 33");
    assert_eq!(pp("#define MACRO test #(other)\nMACRO"), "\ntest (other)");
    assert_eq!(pp("#define MACRO #\nMACRO"), "\n");
}

#[test]
fn hash_quotes_identifier_like_words() {
    // engine: StringifyMacroNameWithNumbers, StringifyUnderscore
    assert_eq!(
        pp("#define MACRO Test #test123\nMACRO"),
        "\nTest \"test123\""
    );
    assert_eq!(pp("#define MACRO Test #_\nMACRO"), "\nTest \"_\"");
}

#[test]
fn hash_inside_double_quotes_in_body_is_literal() {
    assert_eq!(pp("#define M(a) \"#a ## a\" a\nM(x)"), "\n\"#a ## a\" x");
}

#[test]
fn concat_builds_path() {
    // wiki: model = \OFP2\Structures\Various\##FOLDER##\##FOLDER;
    assert_eq!(
        pp("#define M(FOLDER) model = \\OFP2\\Various\\##FOLDER##\\##FOLDER;\nM(Fence)"),
        "\nmodel = \\OFP2\\Various\\Fence\\Fence;"
    );
}

#[test]
fn hash_in_plain_code_is_untouched() {
    // `#` is the SQF select operator.
    assert_eq!(pp("_a = _arr # 0;"), "_a = _arr # 0;");
}

#[test]
fn nested_macro_call_keeps_its_commas() {
    // CBA's FUNC is TRIPLES(DOUBLES(PREFIX,COMPONENT),fnc,var1) and works in the engine, while
    // commas in non-macro parentheses are dropped (NestedParensInMacroArgument).
    assert_eq!(
        pp("#define D(a,b) a-b\n#define F(x,y) [x|y]\nF(D(1,2),3) F(d(1,2),3)"),
        "\n\n[1-2|3] [d(12)|3]"
    );
}

#[test]
fn predefined_macros_from_api() {
    let resolver = a3_preproc::MemoryResolver::new();
    let mut pp = a3_preproc::Preprocessor::new(&resolver);
    assert!(pp.define("DEBUG_MODE_FULL"));
    assert!(pp.define("ADD(a,b) a+b"));
    assert!(!pp.define("1bad"));
    assert!(pp.is_defined("DEBUG_MODE_FULL"));
    let out = pp
        .preprocess_str("\\x.sqf", "#ifdef DEBUG_MODE_FULL\nADD(1,2)\n#endif")
        .unwrap();
    assert_eq!(out.text, "\n1+2\n");
}
