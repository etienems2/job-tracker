//! Inputs the event loop hands to `update`. Keys are translated before this.

use jiff::civil::Date;

use crate::domain::{Snapshot, Stage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Message {
    Quit,
    Move(i32),
    Enter {
        width: u16,
    },
    Escape,
    BeginCommand(String),
    CommandInsert(String),
    CommandBackspace,
    CommandDelete,
    CommandLeft,
    CommandRight,
    CommandHome,
    CommandEnd,
    SubmitCommand {
        today: Date,
    },
    CycleSort,
    OpenStagePick,
    PickStage {
        stage: Stage,
        today: Date,
    },
    PromptDelete,
    ShowHelp,
    Loaded {
        snapshot: Snapshot,
        status: String,
        focus: Option<i64>,
    },
    Failed(String),
    Status(String),
}
