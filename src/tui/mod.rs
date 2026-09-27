pub mod app;

use crate::agent::Agent;
use app::App;

pub async fn run(agent: &mut Agent) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = App::new().run(&mut terminal, agent).await;
    ratatui::restore();
    result.map_err(Into::into)
}
