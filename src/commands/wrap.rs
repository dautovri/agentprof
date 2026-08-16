use anyhow::Result;

use crate::core::agent_wrapper::AgentWrapper;

pub struct WrapCommand;

impl WrapCommand {
    pub fn execute(cmd_and_args: &[String]) -> Result<i32> {
        AgentWrapper::wrap_command(cmd_and_args)
    }
}
