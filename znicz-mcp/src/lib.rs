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
/// `spotify` is a second connection to the same player, used only by the
/// Spotify tools, so a slow Spotify call does not hold up the player tools.
/// `library` is optional: without it the player tools still work and the
/// library tools explain that no library is configured.
pub async fn run_stdio(
    player: IpcClient,
    spotify: IpcClient,
    skills_dirs: Vec<PathBuf>,
    library: Option<Library>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (stdin, stdout) = rmcp::transport::stdio();
    let server = match library {
        Some(library) => ZniczMcpServer::with_library(player, spotify, skills_dirs, library),
        None => ZniczMcpServer::new(player, spotify, skills_dirs),
    };
    let service = server.serve((stdin, stdout)).await?;
    service.waiting().await?;
    Ok(())
}
