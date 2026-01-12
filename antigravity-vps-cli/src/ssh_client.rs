use anyhow::Result;
use async_ssh2_lite::AsyncSession;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::task;

pub struct SshClientFactory;

impl SshClientFactory {
    pub async fn new() -> Result<Self> {
        Ok(SshClientFactory)
    }

    pub async fn connect(&self, user: &str, host: String) -> Result<SshSession> {
        let tcp = TcpStream::connect(format!("{}:22", host)).await?;
        let mut session = AsyncSession::new(tcp, None)?; 
        session.handshake().await?;
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
        let output = task::spawn_blocking(move || {
            let mut buf = Vec::new();
            // This is blocking, but it's in a spawn_blocking call.
            let _ = channel.read_to_end(&mut buf);
            String::from_utf8(buf).map_err(anyhow::Error::new)
        })
        .await??;
        Ok(output)
    }

    pub async fn close(&mut self) -> Result<()> {
        self.session.disconnect(None, "Disconnected by client", None).await?;
        Ok(())
    }
}
