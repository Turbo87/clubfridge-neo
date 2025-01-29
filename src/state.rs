use crate::database;
use crate::running::RunningClubFridge;
use crate::starting::StartingClubFridge;
use iced::futures::FutureExt;
use iced::keyboard::Key;
use iced::{application, window, Subscription, Task};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
use tracing::error;

#[derive(Debug, Default, clap::Parser)]
pub struct Options {
    /// Run in fullscreen
    #[arg(long)]
    fullscreen: bool,

    /// Run in fullscreen
    #[arg(long, default_value = "clubfridge.db?mode=rwc")]
    database: SqliteConnectOptions,
}

pub enum ClubFridge {
    Starting(StartingClubFridge),
    Running(RunningClubFridge),
}

impl Default for ClubFridge {
    fn default() -> Self {
        Self::Starting(Default::default())
    }
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

        let connect_task =
            Task::future(
                database::connect(options.database).map(|result| match result {
                    Ok(pool) => Message::DatabaseConnected(pool),
                    Err(err) => {
                        error!("Failed to connect to database: {err}");
                        Message::DatabaseConnectionFailed
                    }
                }),
            );

        let startup_task = Task::batch([fullscreen_task, connect_task]);

        (Self::default(), startup_task)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        match self {
            Self::Starting(cf) => cf.subscription(),
            Self::Running(cf) => cf.subscription(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        if let Message::StartupComplete(pool) = message {
            *self = Self::Running(RunningClubFridge {
                pool,
                user: None,
                input: String::new(),
                items: Vec::new(),
                show_sale_confirmation: false,
            });

            return Task::none();
        }

        match self {
            Self::Starting(cf) => cf.update(message),
            Self::Running(cf) => cf.update(message),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    DatabaseConnected(SqlitePool),
    DatabaseConnectionFailed,
    DatabaseMigrated,
    DatabaseMigrationFailed,

    StartupComplete(SqlitePool),

    KeyPress(Key),
    SetUser(database::Member),
    AddToSale(database::Article),
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
        assert!(matches!(cf, ClubFridge::Starting(_)));
    }
}
