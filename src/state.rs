use crate::database;
use crate::running::RunningClubFridge;
use crate::setup::Setup;
use crate::starting::StartingClubFridge;
use iced::keyboard::{Key, Modifiers};
use iced::{application, window, Subscription, Task};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing::{error, info};

#[derive(Debug, Default, clap::Parser)]
pub struct Options {
    /// Run in fullscreen
    #[arg(long)]
    fullscreen: bool,

    /// Run in fullscreen
    #[arg(long, default_value = "clubfridge.db?mode=rwc")]
    database: SqliteConnectOptions,

    /// Run in offline mode (no network requests)
    #[arg(long)]
    offline: bool,

    /// When an application update is available, show an "Update" button that
    /// quits the application. Should only be used when the application is
    /// automatically restarted by a supervisor.
    #[arg(long)]
    update_button: bool,
}

pub struct ClubFridge {
    pub state: State,
    update_button: bool,
}

pub enum State {
    Starting(StartingClubFridge),
    Setup(Setup),
    Running(RunningClubFridge),
}

impl ClubFridge {
    pub fn run(options: Options) -> iced::Result {
        application("ClubFridge neo", Self::update, Self::view)
            .theme(Self::theme)
            .subscription(Self::subscription)
            .resizable(true)
            .window_size((800., 480.))
            .run_with(|| Self::new(options))
    }

    pub fn new(options: Options) -> (Self, Task<Message>) {
        // This can be simplified once https://github.com/iced-rs/iced/pull/2627 is released.
        let fullscreen_task = options
            .fullscreen
            .then(|| {
                window::get_latest()
                    .and_then(|id| window::change_mode(id, window::Mode::Fullscreen))
            })
            .unwrap_or(Task::none());

        let connect_task = Task::future(async move {
            info!("Connecting to database…");
            let pool_options = SqlitePoolOptions::default();
            match pool_options.connect_with(options.database).await {
                Ok(pool) => Message::DatabaseConnected(pool),
                Err(err) => {
                    error!("Failed to connect to database: {err}");
                    Message::DatabaseConnectionFailed
                }
            }
        });

        let startup_task = Task::batch([fullscreen_task, connect_task]);

        let cf = Self {
            state: State::Starting(StartingClubFridge::new(options.offline)),
            update_button: options.update_button,
        };

        (cf, startup_task)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        match &self.state {
            State::Starting(cf) => cf.subscription(),
            State::Setup(cf) => cf.subscription(),
            State::Running(cf) => cf.subscription(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        if let Message::GotoSetup(pool) = message {
            self.state = State::Setup(Setup::new(pool));
            return Task::none();
        }

        if let Message::StartupComplete(pool, vereinsflieger) = message {
            let (cf, task) = RunningClubFridge::new(pool, vereinsflieger, self.update_button);
            self.state = State::Running(cf);
            return task;
        }

        if matches!(message, Message::Shutdown) {
            info!("Shutting down…");
            return window::get_latest().and_then(window::close);
        }

        match &mut self.state {
            State::Starting(cf) => cf.update(message),
            State::Setup(cf) => cf.update(message),
            State::Running(cf) => cf.update(message),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    DatabaseConnected(SqlitePool),
    DatabaseConnectionFailed,
    DatabaseMigrated,
    DatabaseMigrationFailed,
    CredentialsFound(database::Credentials),
    GotoSetup(SqlitePool),
    CredentialLookupFailed,

    SetClubId(String),
    SetAppKey(String),
    SetUsername(String),
    SetPassword(String),
    SubmitSetup,
    AuthenticationFailed,

    StartupComplete(SqlitePool, Option<crate::vereinsflieger::Client>),

    SelfUpdate,
    SelfUpdateResult(Result<self_update::Status, Arc<anyhow::Error>>),
    LoadFromVF,
    UploadSalesToVF,
    KeyPress(Key, Modifiers),
    FindMemberResult {
        input: String,
        result: Result<Option<database::Member>, Arc<sqlx::Error>>,
    },
    FindArticleResult {
        input: String,
        result: Result<Option<database::Article>, Arc<sqlx::Error>>,
    },
    Pay,
    Cancel,
    DecrementTimeout,
    PopupTimeoutReached,
    SalesSaved,
    SavingSalesFailed,

    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_initial_state() {
        let (cf, _) = ClubFridge::new(Default::default());
        assert!(matches!(cf.state, State::Starting(_)));
    }
}
