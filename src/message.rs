use jiff::civil::Date;

use crate::domain::{ApplicationId, Snapshot, Stage};

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
    ShowSankey,
    Loaded {
        snapshot: Snapshot,
        status: String,
        focus: Option<ApplicationId>,
    },
    Failed(String),
    Status(String),
}
