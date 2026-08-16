use std::path::Path;
use anyhow::Result;

use crate::ui::tui::TuiApp;

pub struct TuiCommand;

impl TuiCommand {
    pub fn execute(workspace_root: &Path) -> Result<()> {
        TuiApp::run(workspace_root)
    }
}
