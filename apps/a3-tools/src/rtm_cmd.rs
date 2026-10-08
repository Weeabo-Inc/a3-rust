//! `a3-tools rtm ...`: inspect RTM animations.

use std::fmt::Write as _;

use a3_rtm::{Animation, Encoding};
use clap::Subcommand;

use crate::input::InputArgs;

#[derive(Subcommand)]
pub enum RtmCommand {
    /// Print the encoding, move vector, bones, keyframe phases and events of an RTM.
    Info(InputArgs),
    /// Print every bone's transform (quaternion x y z w, translation x y z) at each keyframe,
    /// or at one interpolated phase.
    Dump {
        #[command(flatten)]
        input: InputArgs,
        /// Sample at this phase instead of listing the keyframes.
        #[arg(long)]
        phase: Option<f32>,
    },
}

pub fn run(cmd: RtmCommand) -> anyhow::Result<()> {
    match cmd {
        RtmCommand::Info(input) => print!("{}", info(&Animation::read(&input.read()?)?)),
        RtmCommand::Dump { input, phase } => {
            print!("{}", dump(&Animation::read(&input.read()?)?, phase))
        }
    }
    Ok(())
}

fn info(anim: &Animation) -> String {
    let mut out = String::new();
    let encoding = match anim.encoding {
        Encoding::Plain => "RTM_0101 (plain)".to_string(),
        Encoding::Binarized { version } => format!("BMTR version {version}"),
    };
    let step = anim.step;
    let _ = writeln!(out, "encoding {encoding}");
    let _ = writeln!(out, "step     {} {} {}", step.x, step.y, step.z);
    let _ = writeln!(out, "frames   {}", anim.frames.len());
    let phases: Vec<String> = anim.frames.iter().map(|f| f.phase.to_string()).collect();
    let _ = writeln!(out, "phases   {}", phases.join(" "));
    let _ = writeln!(out, "bones    {}", anim.bones.len());
    for (i, bone) in anim.bones.iter().enumerate() {
        let _ = writeln!(out, "  {i:>3} {bone}");
    }
    let _ = writeln!(out, "events   {}", anim.events.len());
    for event in &anim.events {
        let _ = writeln!(out, "  {:.4} {} {:?}", event.phase, event.name, event.value);
    }
    out
}

fn dump(anim: &Animation, phase: Option<f32>) -> String {
    let mut out = String::new();
    let mut pose = |phase: f32, transforms: &[a3_rtm::BoneTransform]| {
        let _ = writeln!(out, "phase {phase}");
        for (bone, t) in anim.bones.iter().zip(transforms) {
            let (q, p) = (t.rotation, t.translation);
            let _ = writeln!(
                out,
                "  {bone:<24} q {:>8.5} {:>8.5} {:>8.5} {:>8.5}  t {:>8.4} {:>8.4} {:>8.4}",
                q.x, q.y, q.z, q.w, p.x, p.y, p.z
            );
        }
    };
    match phase {
        Some(phase) => pose(phase, &anim.sample(phase)),
        None => {
            for frame in &anim.frames {
                pose(frame.phase, &frame.transforms);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_rtm::{BoneTransform, Event, Frame};
    use glam::{Quat, Vec3};

    fn sample() -> Animation {
        let raised = BoneTransform {
            rotation: Quat::IDENTITY,
            translation: Vec3::new(0.0, 1.0, 0.0),
        };
        Animation {
            encoding: Encoding::Binarized { version: 5 },
            step: Vec3::new(0.0, 0.0, -1.5),
            bones: vec!["pelvis".into()],
            frames: vec![
                Frame {
                    phase: 0.0,
                    transforms: vec![BoneTransform::IDENTITY],
                },
                Frame {
                    phase: 1.0,
                    transforms: vec![raised],
                },
            ],
            events: vec![Event {
                phase: 0.5,
                name: "StepSound".into(),
                value: String::new(),
            }],
        }
    }

    #[test]
    fn info_lists_bones_phases_and_events() {
        let text = info(&sample());
        assert!(text.contains("BMTR version 5"), "{text}");
        assert!(text.contains("phases   0 1"), "{text}");
        assert!(text.contains("    0 pelvis"), "{text}");
        assert!(text.contains("0.5000 StepSound"), "{text}");
    }

    #[test]
    fn dump_samples_an_interpolated_phase() {
        let text = dump(&sample(), Some(0.5));
        assert!(text.starts_with("phase 0.5\n"), "{text}");
        assert!(text.contains("t   0.0000   0.5000   0.0000"), "{text}");
    }
}
