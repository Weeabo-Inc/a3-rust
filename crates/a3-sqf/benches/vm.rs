//! Hot-path benchmarks: loops, arithmetic, calls, array operations and the
//! compiler.

use std::hint::black_box;

use a3_sqf::{NullHost, Vm};
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_script(c: &mut Criterion, name: &str, src: &str) {
    let mut vm = Vm::new(NullHost);
    let code = vm.compile(src).expect("benchmark script compiles");
    c.bench_function(name, |b| {
        b.iter(|| black_box(vm.call(&code, None).expect("benchmark script runs")))
    });
}

fn benches(c: &mut Criterion) {
    bench_script(
        c,
        "for_loop_sum_10k",
        "private _s = 0; for \"_i\" from 1 to 10000 do { _s = _s + _i }; _s",
    );
    bench_script(
        c,
        "while_loop_10k",
        "private _i = 0; while { _i < 10000 } do { _i = _i + 1 }; _i",
    );
    bench_script(
        c,
        "call_10k",
        "private _f = { _this + 1 }; private _r = 0; for \"_i\" from 1 to 10000 do { _r = _i call _f }; _r",
    );
    bench_script(
        c,
        "push_back_10k",
        "private _a = []; for \"_i\" from 1 to 10000 do { _a pushBack _i }; count _a",
    );
    bench_script(
        c,
        "for_each_select_apply_10k",
        "private _a = []; _a resize 10000; _a = _a apply { 1 }; private _n = 0; { _n = _n + _x } forEach _a; count (_a select { _x > 0 })",
    );
    bench_script(
        c,
        "sort_10k",
        "private _a = []; for \"_i\" from 1 to 10000 do { _a pushBack (10000 - _i) }; _a sort true; _a select 0",
    );
    bench_script(
        c,
        "hashmap_10k",
        "private _m = createHashMap; for \"_i\" from 1 to 10000 do { _m set [_i, _i * 2] }; private _s = 0; for \"_i\" from 1 to 10000 do { _s = _s + (_m get _i) }; _s",
    );

    let script = include_str!("../tests/fixtures/sample.sqf");
    let vm = Vm::new(NullHost);
    c.bench_function("compile_sample", |b| {
        b.iter(|| black_box(vm.compile(black_box(script)).expect("sample compiles")))
    });
}

criterion_group!(vm_benches, benches);
criterion_main!(vm_benches);
