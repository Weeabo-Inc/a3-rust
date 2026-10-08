//! Sound controller expressions, with expressions taken from the game's config.

use a3_audio::expr::{Expr, envelope, factor, interpolate};

fn eval(text: &str, vars: &[(&str, f32)]) -> f32 {
    let expr = Expr::parse(text).unwrap_or_else(|e| panic!("{e}"));
    expr.eval(|name| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| *v))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn arithmetic_follows_sqf_precedence() {
    assert_eq!(eval("1 + 2 * 3", &[]), 7.0);
    assert_eq!(eval("(1 + 2) * 3", &[]), 9.0);
    assert_eq!(eval("2 ^ 3 * 2", &[]), 16.0);
    assert_eq!(eval("10 - 4 - 3", &[]), 3.0);
    assert_eq!(eval("-2 * 3", &[]), -6.0);
    assert_eq!(eval("1.5e2 / 3", &[]), 50.0);
    assert_eq!(eval(".5 + 0.25", &[]), 0.75);
    // `max`/`min` bind like `+`: `meadows max sea/2` is `meadows max (sea / 2)`.
    assert_eq!(
        eval("meadows max sea/2", &[("meadows", 0.3), ("sea", 0.8)]),
        0.4
    );
    assert_eq!(eval("1 + 2 max 5", &[]), 5.0);
}

#[test]
fn factor_ramps_and_clamps_in_both_directions() {
    assert!(close(factor(0.3, 0.1, 0.5), 0.5));
    assert_eq!(factor(0.0, 0.1, 0.5), 0.0);
    assert_eq!(factor(9.0, 0.1, 0.5), 1.0);
    // Falling ramp: 1 below 0, 0 above 75.
    assert_eq!(factor(-5.0, 75.0, 0.0), 1.0);
    assert!(close(factor(25.0, 75.0, 0.0), 2.0 / 3.0));
    assert_eq!(factor(100.0, 75.0, 0.0), 0.0);
}

#[test]
fn factor_binds_looser_than_arithmetic() {
    // `(x + 1) factor [0, 4]`.
    assert_eq!(eval("x + 1 factor [0, 4]", &[("x", 1.0)]), 0.5);
    assert_eq!(eval("2 * (x factor [0, 4])", &[("x", 1.0)]), 0.5);
    assert_eq!(
        eval("y interpolate [0, 10, 100, 200]", &[("y", 5.0)]),
        150.0
    );
}

#[test]
fn evaluates_shipped_environment_expressions() {
    // Meadows_Low_SoundShader.
    let meadows = "1.2 * (windy factor[0.1,0.5]) * (1-(0.5*forest))*(1-(0.5*houses))*(1-sea)* (altitudeGround factor [75,0])";
    let v = eval(
        meadows,
        &[
            ("windy", 0.3),
            ("forest", 0.2),
            ("houses", 0.0),
            ("sea", 0.0),
            ("altitudeground", 15.0),
        ],
    );
    assert!(close(v, 1.2 * 0.5 * 0.9 * 1.0 * 1.0 * 0.8), "{v}");

    // Wind_Generic_Low_SoundShader: comparisons give 1 or 0.
    let wind = "(windy factor[0.9,0.3]) * ((altitudeGround * (1 - sea) + altitudeSea * sea) factor [20, 80]) + 0.5 * (altitudeSea factor [200,300]) * (windy > 0.01) * (altitudeGround factor [80, 20]) * (1-forest)";
    let v = eval(
        wind,
        &[
            ("windy", 0.6),
            ("altitudeground", 50.0),
            ("altitudesea", 250.0),
            ("sea", 0.0),
            ("forest", 0.0),
        ],
    );
    assert!(close(v, 0.5 * 0.5 + 0.5 * 0.5 * 1.0 * 0.5), "{v}");

    // `houses max interior` and unary `abs`.
    assert_eq!(
        eval("houses max interior", &[("houses", 0.2), ("interior", 0.7)]),
        0.7
    );
    assert_eq!(eval("abs speed factor [1, 3]", &[("speed", -2.0)]), 0.5);
    assert_eq!(eval("abs(speed) * 2", &[("speed", -2.0)]), 4.0);
}

#[test]
fn envelope_rises_holds_and_falls() {
    let e = |x: f32| eval("x envelope [1, 2, 3, 5]", &[("x", x)]);
    assert_eq!(
        [e(0.0), e(1.5), e(2.5), e(4.0), e(6.0)],
        [0.0, 0.5, 1.0, 0.5, 0.0]
    );
}

#[test]
fn stray_closing_parentheses_at_the_end_are_ignored() {
    // From UAV_05_ForsageInt_SoundShader.
    assert_eq!(eval("1 * (x factor [0, 2]))", &[("x", 1.0)]), 0.5);
}

#[test]
fn variables_are_case_insensitive_and_unknown_ones_are_zero() {
    let expr = Expr::parse("AltitudeSEA + nothingHere").unwrap();
    assert_eq!(expr.variables(), ["altitudesea", "nothinghere"]);
    assert_eq!(expr.eval(|n| (n == "altitudesea").then_some(3.0)), 3.0);
}

#[test]
fn constants_fold() {
    assert_eq!(Expr::parse("-3").unwrap().as_constant(), Some(-3.0));
    assert_eq!(Expr::parse("x").unwrap().as_constant(), None);
    assert_eq!(Expr::constant(0.5).eval(|_| None), 0.5);
}

#[test]
fn operators_outside_the_expression_set_only_take_constants() {
    // Like the engine: `^`, `==`, `sin` and friends fold constants but fail on variables.
    assert_eq!(eval("2 ^ 3 + x", &[("x", 1.0)]), 9.0);
    assert_eq!(eval("sin 90 * x", &[("x", 2.0)]), 2.0);
    for text in ["x ^ 2", "x == 1", "sin x", "x mod 2", "x && 1"] {
        assert!(Expr::parse(text).is_err(), "{text}");
    }
    assert_eq!(eval("x pow 2", &[("x", 3.0)]), 9.0);
    assert_eq!(eval("sqr x + 1", &[("x", 3.0)]), 10.0);
    assert!(
        Expr::parse("x factor [0, y]").is_err(),
        "list items are constants"
    );
}

#[test]
fn random_gen_draws_at_every_evaluation() {
    let expr = Expr::parse("randomGen 2").unwrap();
    assert_eq!(expr.as_constant(), None);
    let values: Vec<f32> = (0..100).map(|_| expr.eval(|_| None)).collect();
    assert!(values.iter().all(|v| (0.0..2.0).contains(v)));
    assert!(values.windows(2).any(|w| w[0] != w[1]));
}

#[test]
fn unknown_variables_fail_to_compile_in_a_context() {
    let ok = Expr::compile("Forest * (windy factor [0, 1])", &["forest", "windy"]);
    assert!(ok.is_ok());
    let err = Expr::compile("meadows * forest", &["meadow", "forest"]).unwrap_err();
    assert_eq!(err.at, 0);
    assert!(err.message.contains("meadows"));
}

#[test]
fn interpolate_and_envelope_match_the_engine() {
    assert_eq!(interpolate(5.0, [0.0, 10.0, 100.0, 200.0]), 150.0);
    assert_eq!(interpolate(-1.0, [0.0, 10.0, 100.0, 200.0]), 100.0);
    // Falling input range: a maps to c, b maps to d.
    assert_eq!(interpolate(2.0, [10.0, 0.0, 100.0, 200.0]), 180.0);
    assert_eq!(envelope(1.0, [1.0, 2.0, 3.0, 5.0]), 0.0, "open interval");
    assert_eq!(envelope(4.0, [1.0, 2.0, 3.0, 5.0]), 0.5);
    assert!(
        factor(2.0, 2.0, 2.0).is_nan(),
        "a == b == x is NaN in the engine"
    );
    assert_eq!(factor(1.0, 2.0, 2.0), 1.0);
}

#[test]
fn reports_syntax_errors_with_their_position() {
    for (text, at) in [
        ("1 +", 3),
        ("(1", 2),
        ("x factor 3", 9),
        ("1 # 2", 2),
        ("factor [1,2]", 0),
    ] {
        let err = Expr::parse(text).unwrap_err();
        assert_eq!(err.at, at, "{text}: {err}");
    }
}
