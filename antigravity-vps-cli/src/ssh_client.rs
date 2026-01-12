use anyhow::{Result, anyhow};
use async_ssh2_lite::{AsyncSession, SessionConfiguration};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

pub struct SshClientFactory;

impl SshClientFactory {
    pub async fn new() -> Result<Self> {
        Ok(SshClientFactory)
    }

    pub async fn connect(&self, user: &str, host: String) -> Result<SshSession> {
        let tcp = TcpStream::connect(format!("{}:22", host)).await?;
        let config = SessionConfiguration::new();
        let mut session = AsyncSession::new(tcp, config)?;
        session.handshake().await?;
        // Assuming user has ssh-agent running or keys are otherwise configured for `userauth_agent`
        session.userauth_agent(user).await?;
        Ok(SshSession { session })
    }
}

pub struct SshSession {
    session: AsyncSession<TcpStream>,
}

impl SshSession {
    pub async fn exec_command(&mut self, command: &str) -> Result<String> {
        let mut channel = self.session.channel_session().await?;
        channel.exec(command).await?;
        let mut buf = Vec::new();
        channel.read_to_end(&mut buf).await?;
        String::from_utf8(buf).map_err(|e| anyhow!("Failed to convert output to UTF-8: {}", e))
    }

    pub async fn close(&mut self) -> Result<()> {
        self.session
            .disconnect(None, "Disconnected by client", None)
            .await?;
        Ok(())
    }

    // Placeholder for upload_file
    pub async fn upload_file(
        &mut self,
        _remote_host: &str,
        _local_path: &std::path::PathBuf,
        _remote_path: &str,
    ) -> Result<()> {
        anyhow::bail!("Upload not implemented yet");
    }

    // Placeholder for download_file
    pub async fn download_file(
        &mut self,
        _remote_host: &str,
        _remote_path: &str,
        _local_path: &std::path::PathBuf,
    ) -> Result<()> {
        anyhow::bail!("Download not implemented yet");
    }
}

pub struct SshClientFactory;

impl SshClientFactory {
    pub async fn new() -> Result<Self> {
        Ok(SshClientFactory)
    }

    pub async fn connect(&self, user: &str, host: String) -> Result<SshSession> {
        let tcp = TcpStream::connect(format!("{}:22", host)).await?;
        let config = SessionConfiguration::new();
        let mut session = AsyncSession::new(tcp, config)?;
        session.handshake().await?;
        // Assuming user has ssh-agent running or keys are otherwise configured for `userauth_agent`
        session.userauth_agent(user).await?;
        Ok(SshSession { session })
    }
}

pub struct SshSession {
    session: AsyncSession<TcpStream>,
}

impl SshSession {
    pub async fn exec_command(&mut self, command: &str) -> Result<String> {
        let mut channel = self.session.channel_session().await?;
        channel.exec(command).await?;
        let mut buf = Vec::new();
        channel.read_to_end(&mut buf).await?;
        String::from_utf8(buf).map_err(|e| anyhow!("Failed to convert output to UTF-8: {}", e))
    }

    pub async fn close(&mut self) -> Result<()> {
        self.session
            .disconnect(None, "Disconnected by client", None)
            .await?;
        Ok(())
    }

    // Placeholder for upload_file
    pub async fn upload_file(
        &mut self,
        _remote_host: &str,
        _local_path: &std::path::PathBuf,
        _remote_path: &str,
    ) -> Result<()> {
        anyhow::bail!("Upload not implemented yet");
    }

    // Placeholder for download_file
    pub async fn download_file(
        &mut self,
        _remote_host: &str,
        _remote_path: &str,
        _local_path: &std::path::PathBuf,
    ) -> Result<()> {
        anyhow::bail!("Download not implemented yet");
    }
}
