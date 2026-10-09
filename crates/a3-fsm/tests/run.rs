//! The machine: the step order of scripted and native FSMs, one transition per step, the
//! native zero-time chain, threshold slots and final states (`docs/re/ai-fsm.md` §1.5, §4.3).

use std::collections::HashMap;

use a3_config::{ConfigTree, parse_text};
use a3_fsm::{Action, Condition, Driver, Fsm, Machine};

/// Records what runs, answers conditions from a table, and draws a fixed "random" number.
#[derive(Default)]
struct Recorder {
    log: Vec<String>,
    values: HashMap<String, f32>,
    random: f32,
}

impl Driver for Recorder {
    fn action(&mut self, action: &Action) {
        match action {
            Action::Script(code) if !code.is_empty() => self.log.push(code.clone()),
            Action::Native(native) if native.function != "nothing" => {
                self.log.push(native.function.clone())
            }
            _ => {}
        }
    }

    fn exit(&mut self, action: &Action) {
        if let Action::Native(native) = action
            && native.function != "nothing"
        {
            self.log.push(format!("exit {}", native.function));
        }
    }

    fn precondition(&mut self, code: &str) {
        self.log.push(code.to_owned());
    }

    fn condition(&mut self, condition: &Condition) -> f32 {
        let key = match condition {
            Condition::Script(code) => code.clone(),
            Condition::Native(native) => native.function.clone(),
        };
        if key == "true" {
            return 1.0;
        }
        if let Condition::Native(native) = condition
            && native.function == "const"
        {
            return native.parameters[0];
        }
        self.values.get(&key).copied().unwrap_or(0.0)
    }

    fn random(&mut self) -> f32 {
        self.random
    }
}

const SCRIPTED: &str = r#"class FSM { fsmName = "T"; class States {
    class A { name = "A"; init = "initA"; precondition = "preA"; class Links {
        class ToC { priority = 2; to = "C"; precondition = "linkPreC"; condition = "goC"; action = "actC"; };
        class ToB { priority = 1; to = "B"; precondition = ""; condition = "goB"; action = "actB"; };
    }; };
    class B { name = "B"; init = "initB"; precondition = "preB"; class Links {
        class Back { priority = 0; to = "A"; precondition = ""; condition = ""; action = ""; };
    }; };
    class C { name = "C"; init = "initC"; precondition = "preC"; class Links {}; };
}; initState = "A"; finalStates[] = {"C"}; };"#;

fn name(fsm: &Fsm, machine: &Machine) -> String {
    machine
        .state()
        .map_or("<ended>".into(), |s| fsm.state(s).class_name.clone())
}

#[test]
fn a_scripted_step_runs_precondition_links_and_the_next_init() {
    let fsm = Fsm::parse_scripted(SCRIPTED).unwrap().fsm;
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder::default();
    driver.values.insert("goB".into(), 1.0);

    assert!(machine.step(&fsm, &mut driver));
    // The higher-priority link is checked first (and fails); then B is taken, and B's init
    // runs in the same step.
    assert_eq!(driver.log, ["initA", "preA", "linkPreC", "actB", "initB"]);
    assert_eq!(name(&fsm, &machine), "B");

    // Next step: B's precondition, then its empty condition holds at once.
    driver.log.clear();
    assert!(machine.step(&fsm, &mut driver));
    assert_eq!(driver.log, ["preB", "initA"]);
}

#[test]
fn a_scripted_state_runs_its_precondition_every_step_and_ends_a_step_after_its_final_state() {
    let fsm = Fsm::parse_scripted(SCRIPTED).unwrap().fsm;
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder::default();
    machine.step(&fsm, &mut driver);
    driver.log.clear();
    for _ in 0..2 {
        assert!(machine.step(&fsm, &mut driver));
    }
    assert_eq!(driver.log, ["preA", "linkPreC", "preA", "linkPreC"]);

    driver.values.insert("goC".into(), 1.0);
    driver.log.clear();
    assert!(
        machine.step(&fsm, &mut driver),
        "the final state is entered..."
    );
    assert_eq!(driver.log, ["preA", "linkPreC", "actC", "initC"]);
    assert!(
        !machine.step(&fsm, &mut driver),
        "...and left one step later"
    );
    assert!(machine.is_finished());
    assert_eq!(driver.log[4..], ["preC"]);
    // An ended machine does nothing.
    assert!(!machine.step(&fsm, &mut driver));
    assert_eq!(driver.log.len(), 5);
}

#[test]
fn a_scripted_link_back_to_its_own_state_reruns_init() {
    let text = r#"class FSM { fsmName = "Loop"; class States {
        class A { name = "A"; init = "initA"; precondition = ""; class Links {
            class Again { priority = 0; to = "A"; precondition = ""; condition = "true"; action = ""; };
        }; };
    }; initState = "A"; finalStates[] = {}; };"#;
    let fsm = Fsm::parse_scripted(text).unwrap().fsm;
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder::default();
    machine.step(&fsm, &mut driver);
    machine.step(&fsm, &mut driver);
    assert_eq!(driver.log, ["initA", "initA", "initA"]);
}

fn native(text: &str, class: &str) -> Fsm {
    let tree = ConfigTree::from_config(&parse_text(text).unwrap());
    Fsm::from_native_config(&(tree.root() >> "CfgFSMs" >> class))
        .unwrap()
        .fsm
}

const CHOICE: &str = r#"class CfgFSMs { class T { class States {
    class Init { name = "Init";
        class Init { function = "nothing"; parameters[] = {}; thresholds[] = {{1, 0.2, 1.2}}; };
        class Links {
            class Rare { priority = 2; to = "Rare";
                class Condition { function = "const"; parameters[] = {0.5}; threshold = 1; };
                class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; };
            class Calm { priority = 1; to = "Calm";
                class Condition { function = " 1 - behaviourCombat"; parameters[] = {}; threshold = 0; };
                class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; };
        };
    };
    class Rare { name = "Rare"; class Init { function = "rare"; parameters[] = {}; thresholds[] = {}; }; class Links {}; };
    class Calm { name = "Calm"; class Init { function = "calm"; parameters[] = {}; thresholds[] = {}; }; class Links {}; };
}; initState = "Init"; finalStates[] = {"Rare", "Calm"}; }; };"#;

#[test]
fn a_native_condition_holds_when_its_slot_is_at_most_its_value() {
    let fsm = native(CHOICE, "T");
    // Slot 1 is drawn in 0.2..1.2: a draw of 0.3 gives 0.5, which `const 0.5` meets exactly.
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder {
        random: 0.3,
        ..Default::default()
    };
    assert!(
        (machine.thresholds()[0] - 0.5).abs() < 1e-6,
        "slots start at 0.5"
    );
    // The final state is entered and left in the same step: its own `true` link chains.
    assert!(!machine.step(&fsm, &mut driver));
    assert!((machine.thresholds()[1] - 0.5).abs() < 1e-6);
    assert_eq!(driver.log, ["rare", "exit rare"]);

    // A draw of 0.5 gives 0.7: `const 0.5` fails and `1-behaviourCombat` (combat 0) holds.
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder {
        random: 0.5,
        ..Default::default()
    };
    assert!(!machine.step(&fsm, &mut driver));
    assert_eq!(driver.log, ["calm", "exit calm"]);

    // In combat neither holds: the machine waits in Init.
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder {
        random: 0.5,
        ..Default::default()
    };
    driver.values.insert("behaviourCombat".into(), 1.0);
    assert!(machine.step(&fsm, &mut driver));
    assert!(driver.log.is_empty());
    assert_eq!(name(&fsm, &machine), "Init");
}

/// The shape of CfgFSMs `Formation`: pass-through states (one `true` link) around a state
/// that waits a step.
const CHAIN: &str = r#"class CfgFSMs { class T { class States {
    class Init { name = "Init";
        class Init { function = "init"; parameters[] = {}; thresholds[] = {{0, 0.5, 0.5}}; };
        class Links { class Always { priority = 0; to = "Start";
            class Condition { function = "true"; parameters[] = {}; threshold = 0; };
            class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; }; };
    };
    class Start { name = "Start";
        class Init { function = "start"; parameters[] = {}; thresholds[] = {}; };
        class Links {
            class Combat { priority = 1; to = "Pass";
                class Condition { function = "combat"; parameters[] = {}; threshold = 0; };
                class Action { function = "linkAction"; parameters[] = {}; thresholds[] = {}; }; };
            class Idle { priority = 0; to = "Start";
                class Condition { function = "true"; parameters[] = {}; threshold = 0; };
                class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; };
        };
    };
    class Pass { name = "Pass";
        class Init { function = "pass"; parameters[] = {}; thresholds[] = {}; };
        class Links { class Always { priority = 0; to = "Wait";
            class Condition { function = "true"; parameters[] = {}; threshold = 0; };
            class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; }; };
    };
    class Wait { name = "Wait";
        class Init { function = "wait"; parameters[] = {}; thresholds[] = {}; };
        class Links { class Done { priority = 0; to = "Start";
            class Condition { function = "done"; parameters[] = {}; threshold = 0; };
            class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; }; };
    };
}; initState = "Init"; finalStates[] = {}; }; };"#;

#[test]
fn a_native_step_takes_one_transition_then_chains_through_plain_true_states() {
    let fsm = native(CHAIN, "T");
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder::default();
    driver.values.insert("combat".into(), 1.0);

    assert!(machine.step(&fsm, &mut driver));
    // Init's enter, the one transition Init -> Start, then Start has two links and is no
    // pass-through: the step ends there.
    assert_eq!(driver.log, ["init", "exit init", "start"]);
    assert_eq!(name(&fsm, &machine), "Start");

    driver.log.clear();
    assert!(machine.step(&fsm, &mut driver));
    // Start -> Pass (link enter and exit, old state exit, new state enter), then Pass chains
    // into Wait in the same step; Wait's link is not a plain `true`, so it waits.
    assert_eq!(
        driver.log,
        [
            "linkAction",
            "exit linkAction",
            "exit start",
            "pass",
            "exit pass",
            "wait"
        ]
    );
    assert_eq!(name(&fsm, &machine), "Wait");

    driver.log.clear();
    assert!(machine.step(&fsm, &mut driver));
    assert!(driver.log.is_empty(), "{:?}", driver.log);
}

#[test]
fn a_native_chain_stops_when_it_comes_back_to_where_it_started() {
    // A and B lead to each other on plain `true`: one step goes A -> B (the transition), then
    // chains B -> A -> B and stops on coming back to B, where the chain started.
    let text = r#"class CfgFSMs { class T { class States {
        class A { name = "A"; class Init { function = "a"; parameters[] = {}; thresholds[] = {}; };
            class Links { class Go { priority = 0; to = "B";
                class Condition { function = "true"; parameters[] = {}; threshold = 0; };
                class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; }; }; };
        class B { name = "B"; class Init { function = "b"; parameters[] = {}; thresholds[] = {}; };
            class Links { class Go { priority = 0; to = "A";
                class Condition { function = "true"; parameters[] = {}; threshold = 0; };
                class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; }; }; };
    }; initState = "A"; finalStates[] = {}; }; };"#;
    let fsm = native(text, "T");
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder::default();
    machine.step(&fsm, &mut driver);
    let enters: Vec<_> = driver
        .log
        .iter()
        .filter(|l| !l.starts_with("exit"))
        .collect();
    // a (init), b (transition), a (chain), b (chain back to the chain's first state: stop).
    assert_eq!(enters, ["a", "b", "a", "b"]);
    assert_eq!(name(&fsm, &machine), "B");
}

#[test]
fn equal_priorities_come_out_in_the_engine_qsort_order() {
    // Shortsort moves the first lowest-priority link to the end, repeatedly: for two tied
    // links it swaps them.
    let text = r#"class FSM { fsmName = "Ties"; class States {
        class A { name = "A"; init = ""; class Links {
            class First { priority = 1; to = "B"; condition = "true"; action = ""; };
            class Second { priority = 1; to = "A"; condition = "true"; action = ""; };
        }; };
        class B { name = "B"; init = ""; class Links {
            class P0 { priority = 0; to = "A"; condition = ""; action = ""; };
            class P5 { priority = 5; to = "A"; condition = ""; action = ""; };
            class Q0 { priority = 0; to = "A"; condition = ""; action = ""; };
            class P3 { priority = 3; to = "A"; condition = ""; action = ""; };
        }; };
    }; initState = "A"; finalStates[] = {}; };"#;
    let fsm = Fsm::parse_scripted(text).unwrap().fsm;
    let names = |state: usize| -> Vec<String> {
        fsm.state(state)
            .links
            .iter()
            .map(|l| l.name.clone())
            .collect()
    };
    assert_eq!(names(0), ["Second", "First"]);
    // [P0 P5 Q0 P3]: P0 (first of the lowest) to the end: [P3 P5 Q0 P0]; then Q0 to index 2:
    // [P3 P5 Q0 P0]; then P3 to index 1: [P5 P3 Q0 P0].
    assert_eq!(names(1), ["P5", "P3", "Q0", "P0"]);
}
