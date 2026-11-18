use std::path::PathBuf;

use crate::command::{self, Parsed};
use crate::domain::Op;
use crate::message::Message;
use crate::model::{Mode, Model};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    None,
    Apply(Op),
    Export(PathBuf),
    Quit,
}

pub(crate) fn update(model: &mut Model, message: Message) -> Effect {
    match message {
        Message::Quit => {
            model.running = false;
            Effect::Quit
        }
        Message::Move(delta) => {
            if model.mode == Mode::Normal {
                move_selection(model, delta);
            }
            Effect::None
        }
        Message::Enter { width } => {
            if model.mode == Mode::Normal {
                open_detail(model, width);
            }
            Effect::None
        }
        Message::Escape => {
            escape(model);
            Effect::None
        }
        Message::BeginCommand(text) => {
            if model.mode == Mode::Normal {
                model.mode = Mode::Command;
                model.command.set(text);
                model.status.clear();
                model.status_error = false;
            }
            Effect::None
        }
        Message::CommandInsert(text) => {
            if model.mode == Mode::Command {
                model.command.insert(&text);
            }
            Effect::None
        }
        Message::CommandBackspace => {
            if model.mode == Mode::Command {
                model.command.backspace();
            }
            Effect::None
        }
        Message::CommandDelete => {
            if model.mode == Mode::Command {
                model.command.delete_forward();
            }
            Effect::None
        }
        Message::CommandLeft => {
            if model.mode == Mode::Command {
                model.command.move_left();
            }
            Effect::None
        }
        Message::CommandRight => {
            if model.mode == Mode::Command {
                model.command.move_right();
            }
            Effect::None
        }
        Message::CommandHome => {
            if model.mode == Mode::Command {
                model.command.move_home();
            }
            Effect::None
        }
        Message::CommandEnd => {
            if model.mode == Mode::Command {
                model.command.move_end();
            }
            Effect::None
        }
        Message::SubmitCommand { today } => {
            if model.mode != Mode::Command {
                return Effect::None;
            }
            submit(model, today)
        }
        Message::CycleSort => {
            if model.mode == Mode::Normal {
                let selected = model.selected_id();
                model.sort = model.sort.cycle();
                if let Some(id) = selected {
                    model.select_id(id);
                } else {
                    model.sync_table();
                }
                model.status = format!("sort: {}", model.sort.label());
                model.status_error = false;
            }
            Effect::None
        }
        Message::OpenStagePick => {
            if model.mode == Mode::Normal {
                open_stage_pick(model);
            }
            Effect::None
        }
        Message::PickStage { stage, today } => {
            if model.mode != Mode::StagePick {
                return Effect::None;
            }
            pick_stage(model, stage, today)
        }
        Message::PromptDelete => {
            if model.mode == Mode::Normal {
                prompt_delete(model);
            }
            Effect::None
        }
        Message::ShowHelp => {
            if model.mode == Mode::Normal {
                model.mode = Mode::Help;
            }
            Effect::None
        }
        Message::Loaded {
            snapshot,
            status,
            focus,
        } => {
            model.apps = snapshot.applications;
            model.status = status;
            model.status_error = false;
            if let Some(id) = focus {
                model.select_id(id);
            } else {
                model.sync_table();
            }
            Effect::None
        }
        Message::Failed(text) => {
            model.status = text;
            model.status_error = true;
            Effect::None
        }
        Message::Status(text) => {
            model.status = text;
            model.status_error = false;
            Effect::None
        }
    }
}

fn move_selection(model: &mut Model, delta: i32) {
    let Some(last) = model.visible_indices().len().checked_sub(1) else {
        model.sync_table();
        return;
    };
    let steps = usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX);
    model.selection = if delta < 0 {
        model.selection.saturating_sub(steps)
    } else {
        model.selection.saturating_add(steps).min(last)
    };
    model.sync_table();
}

fn open_detail(model: &mut Model, width: u16) {
    if model.selected_id().is_none() {
        model.status = "no application selected".into();
        model.status_error = true;
        return;
    }
    if width < 100 {
        model.show_detail = true;
    }
}

fn escape(model: &mut Model) {
    match model.mode {
        Mode::Command => {
            model.mode = Mode::Normal;
            model.command.clear();
            model.status = "cancelled".into();
            model.status_error = false;
        }
        Mode::StagePick | Mode::Help => {
            model.mode = Mode::Normal;
        }
        Mode::Normal => {
            model.show_detail = false;
        }
    }
}

fn submit(model: &mut Model, today: jiff::civil::Date) -> Effect {
    match command::parse(&model.command.buffer, today) {
        Ok(Parsed::Apply(op)) => {
            leave_command(model);
            Effect::Apply(op)
        }
        Ok(Parsed::Filter(filter)) => {
            let selected = model.selected_id();
            model.filter = filter;
            if let Some(id) = selected {
                model.select_id(id);
            } else {
                model.selection = 0;
                model.sync_table();
            }
            model.status = format!("filter: {}", model.filter.label());
            model.status_error = false;
            leave_command(model);
            Effect::None
        }
        Ok(Parsed::Export(path)) => {
            leave_command(model);
            Effect::Export(path)
        }
        Err(error) => {
            model.status = error.to_string();
            model.status_error = true;
            Effect::None
        }
    }
}

fn leave_command(model: &mut Model) {
    model.mode = Mode::Normal;
    model.command.clear();
}

fn open_stage_pick(model: &mut Model) {
    if model.selected_id().is_none() {
        model.status = "no application selected".into();
        model.status_error = true;
        return;
    }
    model.mode = Mode::StagePick;
    model.status = "choose a stage".into();
    model.status_error = false;
}

fn pick_stage(model: &mut Model, stage: crate::domain::Stage, today: jiff::civil::Date) -> Effect {
    let Some(id) = model.selected_id() else {
        model.mode = Mode::Normal;
        model.status = "no application selected".into();
        model.status_error = true;
        return Effect::None;
    };
    model.mode = Mode::Normal;
    Effect::Apply(Op::SetStage {
        application_id: id,
        stage,
        on: today,
    })
}

fn prompt_delete(model: &mut Model) {
    let Some(id) = model.selected_id() else {
        model.status = "no application selected".into();
        model.status_error = true;
        return;
    };
    model.mode = Mode::Command;
    model.command.set(format!("delete {id} "));
    model.status.clear();
    model.status_error = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Application, Snapshot, Stage, StageEvent};
    use crate::message::Message;
    use jiff::civil::Date;

    fn today() -> Date {
        Date::new(2026, 11, 17).unwrap()
    }

    fn application(id: i64, company: &str, stage: Stage, on: Date) -> Application {
        Application::new(
            id,
            company.into(),
            "Role".into(),
            vec![StageEvent {
                id,
                stage,
                occurred_on: on,
            }],
        )
    }

    fn model_with(apps: Vec<Application>) -> Model {
        Model::new(Snapshot { applications: apps }, PathBuf::from("tracker.db"))
    }

    #[test]
    fn submit_add_emits_apply_without_a_store() {
        let mut model = model_with(vec![]);
        let effect = update(
            &mut model,
            Message::BeginCommand("add Acme | Backend".into()),
        );
        assert!(matches!(effect, Effect::None));
        assert_eq!(model.mode, Mode::Command);

        let effect = update(&mut model, Message::SubmitCommand { today: today() });
        assert_eq!(
            effect,
            Effect::Apply(Op::Add {
                company: "Acme".into(),
                role: "Backend".into(),
                stage: Stage::Applied,
                on: today(),
            })
        );
        assert_eq!(model.mode, Mode::Normal);
        assert!(model.command.buffer.is_empty());
    }

    #[test]
    fn a_bad_prefix_sets_the_status_and_does_not_apply() {
        let mut model = model_with(vec![]);
        update(&mut model, Message::BeginCommand("stage 3 t".into()));
        let effect = update(&mut model, Message::SubmitCommand { today: today() });
        assert_eq!(effect, Effect::None);
        assert_eq!(model.mode, Mode::Command);
        assert!(model.status_error);
        assert!(model.status.contains("ambiguous"));
        assert!(model.apps.is_empty());
    }

    #[test]
    fn help_and_quit_change_mode_without_a_store() {
        let mut model = model_with(vec![]);
        assert_eq!(update(&mut model, Message::ShowHelp), Effect::None);
        assert_eq!(model.mode, Mode::Help);
        assert_eq!(update(&mut model, Message::Escape), Effect::None);
        assert_eq!(model.mode, Mode::Normal);
        assert_eq!(update(&mut model, Message::Quit), Effect::Quit);
        assert!(!model.running);
    }

    #[test]
    fn stage_picker_emits_set_stage_for_the_selection() {
        let mut model = model_with(vec![application(4, "Northwind", Stage::Applied, today())]);
        update(&mut model, Message::OpenStagePick);
        assert_eq!(model.mode, Mode::StagePick);
        let effect = update(
            &mut model,
            Message::PickStage {
                stage: Stage::TechnicalMeeting,
                today: today(),
            },
        );
        assert_eq!(
            effect,
            Effect::Apply(Op::SetStage {
                application_id: 4,
                stage: Stage::TechnicalMeeting,
                on: today(),
            })
        );
        assert_eq!(model.mode, Mode::Normal);
    }

    #[test]
    fn delete_prompt_prefills_the_selected_id() {
        let mut model = model_with(vec![application(3, "Acme", Stage::Applied, today())]);
        update(&mut model, Message::PromptDelete);
        assert_eq!(model.mode, Mode::Command);
        assert_eq!(model.command.buffer, "delete 3 ");
        let effect = update(&mut model, Message::SubmitCommand { today: today() });
        assert_eq!(effect, Effect::Apply(Op::Delete { application_id: 3 }));
    }

    #[test]
    fn failed_load_leaves_the_snapshot_alone() {
        let mut model = model_with(vec![application(3, "Acme", Stage::Applied, today())]);
        update(&mut model, Message::Failed("no application 9".into()));
        assert_eq!(model.apps.len(), 1);
        assert_eq!(model.apps[0].company, "Acme");
        assert!(model.status_error);
        assert_eq!(model.status, "no application 9");
    }

    #[test]
    fn loaded_replaces_the_snapshot_and_focuses_the_id() {
        let mut model = model_with(vec![]);
        let snapshot = Snapshot {
            applications: vec![
                application(1, "Acme", Stage::Applied, today()),
                application(2, "Northwind", Stage::FirstMeeting, today()),
            ],
        };
        update(
            &mut model,
            Message::Loaded {
                snapshot,
                status: "wrote application 2".into(),
                focus: Some(2),
            },
        );
        assert_eq!(model.apps.len(), 2);
        assert_eq!(model.selected_id(), Some(2));
        assert!(!model.status_error);
    }

    #[test]
    fn filter_and_sort_keep_the_selected_application_when_it_remains_visible() {
        let mut model = model_with(vec![
            application(1, "Acme", Stage::Applied, Date::new(2026, 11, 1).unwrap()),
            application(
                2,
                "Northwind",
                Stage::Refused,
                Date::new(2026, 11, 2).unwrap(),
            ),
        ]);
        model.select_id(1);
        update(&mut model, Message::BeginCommand("filter all".into()));
        assert_eq!(
            update(&mut model, Message::SubmitCommand { today: today() }),
            Effect::None
        );
        assert_eq!(model.filter, crate::domain::Filter::All);
        assert_eq!(model.selected_id(), Some(1));
        assert_eq!(model.mode, Mode::Normal);

        update(&mut model, Message::CycleSort);
        assert_eq!(model.sort, crate::domain::Sort::Company);
        assert_eq!(model.selected_id(), Some(1));
        assert_eq!(model.status, "sort: company");
    }

    #[test]
    fn selection_clamps_at_the_ends_of_the_visible_list() {
        let mut model = model_with(vec![
            application(1, "Acme", Stage::Applied, today()),
            application(2, "Northwind", Stage::FirstMeeting, today()),
        ]);
        update(&mut model, Message::Move(-1));
        assert_eq!(model.selection, 0);
        update(&mut model, Message::Move(1));
        update(&mut model, Message::Move(5));
        assert_eq!(model.selection, 1);
    }

    #[test]
    fn narrow_enter_opens_detail_and_escape_closes_it() {
        let mut model = model_with(vec![application(1, "Acme", Stage::Applied, today())]);
        update(&mut model, Message::Enter { width: 80 });
        assert!(model.show_detail);
        update(&mut model, Message::Enter { width: 120 });
        assert!(model.show_detail);
        update(&mut model, Message::Escape);
        assert!(!model.show_detail);
        assert_eq!(model.mode, Mode::Normal);
    }

    #[test]
    fn export_parses_to_an_effect() {
        let mut model = model_with(vec![]);
        update(
            &mut model,
            Message::BeginCommand(r"export C:\temp\applications.json".into()),
        );
        assert_eq!(
            update(&mut model, Message::SubmitCommand { today: today() }),
            Effect::Export(PathBuf::from(r"C:\temp\applications.json"))
        );
    }
}
