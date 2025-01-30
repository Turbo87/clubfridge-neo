use crate::database;
use crate::running::RunningClubFridge;
use crate::starting::StartingClubFridge;
use iced::futures::FutureExt;
use iced::keyboard::Key;
use iced::{application, window, Subscription, Task};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
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
}

pub struct ClubFridge {
    pub state: State,
}

pub enum State {
    Starting(StartingClubFridge),
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

        let state = State::Starting(StartingClubFridge::new(options.offline));
        (Self { state }, startup_task)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        match &self.state {
            State::Starting(cf) => cf.subscription(),
            State::Running(cf) => cf.subscription(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        if let Message::StartupComplete(pool, credentials) = message {
            let vereinsflieger = credentials.map(crate::vereinsflieger::Client::new);

            let task = match &vereinsflieger {
                Some(_) => Task::batch([
                    Task::done(Message::LoadFromVF),
                    Task::done(Message::UploadSalesToVF),
                ]),
                None => {
                    info!("Running in offline mode, skipping Vereinsflieger sync");
                    Task::none()
                }
            };

            self.state = State::Running(RunningClubFridge::new(pool, vereinsflieger));

            return task;
        }

        match &mut self.state {
            State::Starting(cf) => cf.update(message),
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
    CredentialLookupFailed,

    StartupComplete(SqlitePool, Option<database::Credentials>),

    LoadFromVF,
    UploadSalesToVF,
    KeyPress(Key),
    SetUser(database::Member),
    AddSale(database::Article),
    Pay,
    Cancel,
    HideSaleConfirmation,
    SalesSaved,
    SavingSalesFailed,
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
