/// Progress events emitted by the shared installer runner.
#[derive(Debug, Clone)]
pub enum InstallEvent {
    Log(String),
    Progress {
        current: u32,
        total: u32,
        message: String,
    },
}

impl InstallEvent {
    pub fn log(message: impl Into<String>) -> Self {
        Self::Log(message.into())
    }

    pub fn progress(current: u32, total: u32, message: impl Into<String>) -> Self {
        Self::Progress {
            current,
            total,
            message: message.into(),
        }
    }
}
