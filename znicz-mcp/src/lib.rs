mod server;
mod skills;

use std::path::PathBuf;

use rmcp::ServiceExt;
use znicz_core::PlayerHandle;

pub use server::ZniczMcpServer;
pub use skills::SkillRegistry;

pub async fn run_stdio(
    player: PlayerHandle,
    skills_dirs: Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (stdin, stdout) = rmcp::transport::stdio();
    let server = ZniczMcpServer::new(player, skills_dirs);
    let service = server.serve((stdin, stdout)).await?;
    service.waiting().await?;
    Ok(())
}
