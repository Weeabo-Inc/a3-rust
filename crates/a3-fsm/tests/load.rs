//! Loading FSM definitions: `.fsm` text (scripted) and `CfgFSMs` classes (native).

use a3_config::{ConfigTree, parse_text};
use a3_fsm::{Action, Condition, Fsm, FsmKind, NativeAction, ThresholdDraw};

/// A `.fsm` file as the FSM Editor writes it: layout data in comments, code as `"..." \n "..."`.
const SCRIPTED: &str = r#"/*%FSM<COMPILE "scriptedFSM.cfg, Test">*/
/*%FSM<HEAD>*/
/*
item0[] = {"Init",0,250,-40.0,-180.0,50.0,-130.0,0.0,"Init"};
*//*%FSM</HEAD>*/
class FSM
{
  fsmName = "Test";
  class States
  {
    /*%FSM<STATE "Init">*/
    class Init
    {
      name = "Init";
      init = /*%FSM<STATEINIT""">*/"_n = 0;" \n "_t = time;"/*%FSM</STATEINIT""">*/;
      precondition = /*%FSM<STATEPRECONDITION""">*/""/*%FSM</STATEPRECONDITION""">*/;
      class Links
      {
        /*%FSM<LINK "Low">*/
        class Low
        {
          priority = 0.000000;
          to="Loop";
          precondition = /*%FSM<CONDPRECONDITION""">*/""/*%FSM</CONDPRECONDITION""">*/;
          condition=/*%FSM<CONDITION""">*/"true"/*%FSM</CONDITION""">*/;
          action=/*%FSM<ACTION""">*/""/*%FSM</ACTION""">*/;
        };
        /*%FSM</LINK>*/
        /*%FSM<LINK "High">*/
        class High
        {
          priority = 2.000000;
          to="End";
          precondition = /*%FSM<CONDPRECONDITION""">*/""/*%FSM</CONDPRECONDITION""">*/;
          condition=/*%FSM<CONDITION""">*/"_n > 3"/*%FSM</CONDITION""">*/;
          action=/*%FSM<ACTION""">*/"done = true;"/*%FSM</ACTION""">*/;
        };
        /*%FSM</LINK>*/
      };
    };
    /*%FSM</STATE>*/
    class Loop
    {
      name = "Loop";
      init = "_n = _n + 1;";
      precondition = "";
      class Links
      {
        class Back { priority = 0.0; to = "Init"; precondition = ""; condition = "true"; action = ""; };
      };
    };
    class End
    {
      name = "End";
      init = "";
      precondition = "";
      class Links {};
    };
  };
  initState="Init";
  finalStates[] =
  {
    "End",
  };
};
/*%FSM</COMPILE>*/"#;

#[test]
fn scripted_fsm_loads_states_links_and_code() {
    let loaded = Fsm::parse_scripted(SCRIPTED).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    let fsm = loaded.fsm;
    assert_eq!(fsm.name, "Test");
    assert_eq!(fsm.kind, FsmKind::Scripted);
    assert_eq!(fsm.states.len(), 3);
    assert_eq!(fsm.init_state, 0);

    let init = fsm.state(0);
    assert_eq!(init.init, Action::Script("_n = 0;\n_t = time;".into()));
    // Highest priority first, whatever the file order.
    let names: Vec<_> = init.links.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["High", "Low"]);
    assert_eq!(init.links[0].to, fsm.state_named("End"));
    assert_eq!(init.links[0].condition, Condition::Script("_n > 3".into()));
    assert_eq!(init.links[0].action, Action::Script("done = true;".into()));

    let end = fsm.state(fsm.state_named("end").unwrap());
    assert!(end.is_final);
    assert!(!fsm.state(fsm.state_named("Loop").unwrap()).is_final);
}

#[test]
fn equal_priorities_come_out_in_the_engine_order() {
    let text = r#"class FSM { fsmName = "Ties"; class States {
        class A { name = "A"; init = ""; class Links {
            class First { priority = 1; to = "B"; condition = "true"; action = ""; };
            class Second { priority = 1; to = "A"; condition = "true"; action = ""; };
            class Third { priority = 1; to = "B"; condition = "true"; action = ""; };
        }; };
        class B { name = "B"; init = ""; class Links {}; };
    }; initState = "A"; finalStates[] = {}; };"#;
    let fsm = Fsm::parse_scripted(text).unwrap().fsm;
    let names: Vec<_> = fsm.state(0).links.iter().map(|l| l.name.as_str()).collect();
    // The engine's unstable qsort, not file order (`docs/re/ai-fsm.md` §1.4).
    assert_eq!(names, ["Second", "Third", "First"]);
}

#[test]
fn a_bad_init_state_and_a_dangling_link_are_reported_and_tolerated() {
    let text = r#"class FSM { fsmName = "Bad"; class States {
        class A { name = "A"; init = ""; class Links {
            class Lost { priority = 0; to = "Nowhere"; condition = "true"; action = ""; };
        }; };
    }; initState = "Missing"; finalStates[] = {}; };"#;
    let loaded = Fsm::parse_scripted(text).unwrap();
    assert_eq!(loaded.fsm.init_state, 0);
    assert!(loaded.fsm.state(0).links.is_empty());
    assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
}

#[test]
fn a_file_without_class_fsm_does_not_load() {
    assert!(Fsm::parse_scripted("class Other {};").is_err());
    assert!(Fsm::parse_scripted("class FSM { class States {}; };").is_err());
}

/// A trimmed `CfgFSMs` class in the shape of the shipped `Formation` and `Dragonfly`.
const NATIVE: &str = r#"
class CfgFSMs {
    class Test {
        class States {
            class Init {
                name = "Init";
                class Init { function = "setNoBackwards"; parameters[] = {1.0}; thresholds[] = {{0, 0.5, 0.5}}; };
                class Links {
                    class Always {
                        priority = 0.0; to = "Choose";
                        class Condition { function = "true"; parameters[] = {}; threshold = 0; };
                        class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; };
                    };
                };
            };
            class Choose {
                name = "Choose";
                class Init { function = "nothing"; parameters[] = {}; thresholds[] = {{1, 0, 1.0}}; };
                class Links {
                    class Rare {
                        priority = 1.0; to = "Init";
                        class Condition { function = "const"; parameters[] = {0.1}; threshold = 1; };
                        class Action { function = "script:hint ""rare"""; parameters[] = {}; thresholds[] = {}; };
                    };
                    class Calm {
                        priority = 3.0; to = "Init";
                        class Condition { function = "1-behaviourCombat"; parameters[] = {}; threshold = 0; };
                        class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; };
                    };
                };
            };
        };
        initState = "Init";
        finalStates[] = {};
    };
};"#;

#[test]
fn native_fsm_loads_functions_parameters_and_thresholds() {
    let tree = ConfigTree::from_config(&parse_text(NATIVE).unwrap());
    let loaded = Fsm::from_native_config(&(tree.root() >> "CfgFSMs" >> "Test")).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    let fsm = loaded.fsm;
    assert_eq!(fsm.name, "Test");
    assert_eq!(fsm.kind, FsmKind::Native);
    assert_eq!(
        fsm.state(0).init,
        Action::Native(NativeAction {
            function: "setNoBackwards".into(),
            parameters: vec![1.0],
            thresholds: vec![ThresholdDraw {
                index: 0,
                min: 0.5,
                max: 0.5
            }],
        })
    );
    let choose = fsm.state(1);
    assert_eq!(choose.links[0].name, "Calm");
    let Condition::Native(calm) = &choose.links[0].condition else {
        panic!("native condition");
    };
    assert_eq!(calm.function, "behaviourCombat");
    assert!(calm.inverted);
    let Condition::Native(rare) = &choose.links[1].condition else {
        panic!("native condition");
    };
    assert_eq!((rare.function.as_str(), rare.threshold), ("const", 1));
    assert_eq!(rare.parameters, [0.1]);
    assert!(!rare.inverted);
    // `script:` makes a native action SQF.
    assert_eq!(
        choose.links[1].action,
        Action::Script("hint \"rare\"".into())
    );
    assert_eq!(fsm.threshold_count(), 2);
}

#[test]
fn real_game_fsms_load() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();

    // Every native FSM of CfgFSMs.
    let cfg = data.config.root() >> "CfgFSMs";
    let mut native = 0;
    for class in cfg.entries().into_iter().filter(|c| c.is_class()) {
        let loaded = Fsm::from_native_config(&class).unwrap();
        assert!(
            loaded.warnings.is_empty(),
            "{}: {:?}",
            class.name(),
            loaded.warnings
        );
        native += 1;
    }
    assert!(
        native >= 4,
        "CfgFSMs has Dragonfly, Butterfly, HoneyBee and Formation"
    );
    let formation = Fsm::from_native_config(&(cfg >> "Formation")).unwrap().fsm;
    assert_eq!(formation.state(formation.init_state).class_name, "Init");
    assert_eq!(formation.states.len(), 18);

    // Every scripted FSM shipped with the game.
    let mut scripted = 0;
    let mut failed = Vec::new();
    for path in data.vfs.glob("**/*.fsm") {
        let text = a3_gamedata::read_text(&data.vfs, path.as_str()).unwrap();
        // FSM Editor sources compiled with another config (`entityFSM.cfg` → CfgFSMs,
        // `radioProtocol_config.cfg`, `ORBAT.cfg`, campaign descriptions) are not files the
        // engine runs as FSMs.
        let header = text.lines().next().unwrap_or_default();
        if header.contains("%FSM<COMPILE") && !header.contains("scriptedFSM.cfg") {
            continue;
        }
        // Written with an empty value (`itemno = ;`), which our config parser rejects; whether
        // the engine tolerates it is unverified.
        if path.as_str().ends_with("defense\\missionflow.fsm") {
            continue;
        }
        match Fsm::parse_scripted(&text) {
            Ok(_) => scripted += 1,
            Err(e) => failed.push(format!("{path}: {e}")),
        }
    }
    assert!(scripted > 500, "loaded {scripted}");
    assert!(failed.is_empty(), "{failed:#?}");
}
