use crate::runtime::messages::AgentRequest;
use crate::ui::state::ChatState;
use tokio::sync::mpsc;

pub struct CommandContext<'a> {
    pub state: &'a mut ChatState,
    pub agent_tx: &'a mpsc::Sender<AgentRequest>,
}

pub type CommandResult = Result<(), String>;
