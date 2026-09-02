mod server;
mod skills;

use std::path::PathBuf;

use rmcp::ServiceExt;
use znicz_core::IpcClient;
use znicz_library::Library;

pub use server::ZniczMcpServer;
pub use skills::SkillRegistry;

/// Serve MCP over stdio.
///
/// `library` is optional: without it the player tools still work and the
/// library tools explain that no library is configured.
pub async fn run_stdio(
    player: IpcClient,
    skills_dirs: Vec<PathBuf>,
    library: Option<Library>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (stdin, stdout) = rmcp::transport::stdio();
    let server = match library {
        Some(library) => ZniczMcpServer::with_library(player, skills_dirs, library),
        None => ZniczMcpServer::new(player, skills_dirs),
    };
    let service = server.serve((stdin, stdout)).await?;
    service.waiting().await?;
    Ok(())
}
