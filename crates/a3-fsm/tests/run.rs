//! The machine: stepping order, one link per step, priorities, final states, and native
//! conditions against drawn thresholds.

use std::collections::HashMap;

use a3_config::{ConfigTree, parse_text};
use a3_fsm::{Action, Condition, Driver, Fsm, Machine};

/// Records what runs, answers conditions from a table, and draws fixed "random" numbers.
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

    fn precondition(&mut self, code: &str) {
        if !code.is_empty() {
            self.log.push(code.to_owned());
        }
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
        class Back { priority = 0; to = "A"; precondition = ""; condition = "true"; action = ""; };
    }; };
    class C { name = "C"; init = "initC"; precondition = ""; class Links {}; };
}; initState = "A"; finalStates[] = {"C"}; };"#;

#[test]
fn the_init_state_runs_through_to_its_links_in_the_first_step() {
    let fsm = Fsm::parse_scripted(SCRIPTED).unwrap().fsm;
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder::default();
    driver.values.insert("goB".into(), 1.0);

    assert!(machine.step(&fsm, &mut driver));
    // The higher-priority link is checked first (and fails); then B is taken.
    assert_eq!(
        driver.log,
        ["initA", "preA", "linkPreC", "actB", "initB"]
    );
    assert_eq!(fsm.state(machine.state()).name, "B");

    // Next step: B resumes with its precondition and leaves at once.
    driver.log.clear();
    assert!(machine.step(&fsm, &mut driver));
    assert_eq!(driver.log, ["preB", "initA"]);
}

#[test]
fn a_state_whose_links_all_fail_waits_and_checks_again() {
    let fsm = Fsm::parse_scripted(SCRIPTED).unwrap().fsm;
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder::default();
    machine.step(&fsm, &mut driver);
    driver.log.clear();
    for _ in 0..3 {
        assert!(machine.step(&fsm, &mut driver));
    }
    // No precondition again, only the link checks.
    assert_eq!(driver.log, ["linkPreC", "linkPreC", "linkPreC"]);
    driver.values.insert("goC".into(), 1.0);
    assert!(!machine.step(&fsm, &mut driver), "a final state ends the machine");
    assert!(machine.is_finished());
    assert_eq!(driver.log[3..], ["linkPreC", "actC", "initC"]);
    // A finished machine does nothing.
    assert!(!machine.step(&fsm, &mut driver));
    assert_eq!(driver.log.len(), 6);
}

const NATIVE: &str = r#"class CfgFSMs { class T { class States {
    class Init { name = "Init";
        class Init { function = "nothing"; parameters[] = {}; thresholds[] = {{1, 0.2, 1.2}}; };
        class Links {
            class Rare { priority = 2; to = "Rare";
                class Condition { function = "const"; parameters[] = {0.5}; threshold = 1; };
                class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; };
            class Calm { priority = 1; to = "Calm";
                class Condition { function = "1-behaviourCombat"; parameters[] = {}; threshold = 0; };
                class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; };
        };
    };
    class Rare { name = "Rare"; class Init { function = "rare"; parameters[] = {}; thresholds[] = {}; }; class Links {}; };
    class Calm { name = "Calm"; class Init { function = "calm"; parameters[] = {}; thresholds[] = {}; }; class Links {}; };
}; initState = "Init"; finalStates[] = {"Rare", "Calm"}; }; };"#;

fn native() -> Fsm {
    let tree = ConfigTree::from_config(&parse_text(NATIVE).unwrap());
    Fsm::from_native_config(&(tree.root() >> "CfgFSMs" >> "T"))
        .unwrap()
        .fsm
}

#[test]
fn a_native_condition_holds_when_its_value_exceeds_its_threshold() {
    let fsm = native();
    // Threshold 1 is drawn in 0.2..1.2; a draw of 0.2 gives 0.4, below `const 0.5`.
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder {
        random: 0.2,
        ..Default::default()
    };
    machine.step(&fsm, &mut driver);
    assert!((machine.thresholds()[1] - 0.4).abs() < 1e-6);
    assert_eq!(driver.log, ["rare"]);

    // A draw of 0.5 gives 0.7: `const 0.5` fails, and `1-behaviourCombat` (combat 0) holds.
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder {
        random: 0.5,
        ..Default::default()
    };
    machine.step(&fsm, &mut driver);
    assert_eq!(driver.log, ["calm"]);

    // In combat, neither holds and the machine waits.
    let mut machine = Machine::new(&fsm);
    let mut driver = Recorder {
        random: 0.5,
        ..Default::default()
    };
    driver.values.insert("behaviourCombat".into(), 1.0);
    assert!(machine.step(&fsm, &mut driver));
    assert!(driver.log.is_empty());
    assert_eq!(fsm.state(machine.state()).name, "Init");
}
